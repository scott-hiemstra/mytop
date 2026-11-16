//! DRM fdinfo scanner (kernel Documentation/gpu/drm-usage-stats.rst).
//!
//! Every open DRM file descriptor exposes per-client engine busy time and
//! memory usage in /proc/<pid>/fdinfo/<fd>. Summing clients per PCI device
//! gives per-engine utilisation; per pid gives per-process GPU usage.
//! Without root we can only see our own processes' fds.

use super::ProcGpu;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    os::unix::ffi::OsStrExt,
    time::Instant,
};

#[derive(Default)]
pub struct DevUsage {
    /// engine name -> busy % (summed over clients, capped at 100)
    pub engines: BTreeMap<String, f64>,
    pub vram: u64,
    pub procs: HashMap<u32, ProcGpu>,
    pub clients: usize,
}

impl DevUsage {
    pub fn max_engine(&self) -> f64 {
        self.engines.values().copied().fold(0.0, f64::max)
    }
}

#[derive(Default)]
struct Prev {
    engines_ns: HashMap<String, u64>,
    cycles: HashMap<String, (u64, u64)>,
}

pub struct Scanner {
    prev: HashMap<(String, u64), Prev>,
    last: Instant,
}

fn parse_mem(v: &str) -> u64 {
    let mut it = v.split_whitespace();
    let n: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    match it.next() {
        Some("KiB") => n * 1024,
        Some("MiB") => n * 1024 * 1024,
        Some("GiB") => n * 1024 * 1024 * 1024,
        _ => n,
    }
}

fn is_vram_region(r: &str) -> bool {
    !r.contains("stolen") && (r.starts_with("vram") || r.starts_with("local"))
}

impl Scanner {
    pub fn new() -> Self {
        Self {
            prev: HashMap::new(),
            last: Instant::now(),
        }
    }

    pub fn scan(&mut self) -> HashMap<String, DevUsage> {
        let now = Instant::now();
        let dt_ns = now.duration_since(self.last).as_nanos() as f64;
        self.last = now;

        let mut out: HashMap<String, DevUsage> = HashMap::new();
        let mut next_prev = HashMap::new();
        let mut seen = HashSet::new();

        for pe in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = pe.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(fds) = fs::read_dir(format!("/proc/{pid}/fd")) else {
                continue;
            };
            for fe in fds.flatten() {
                let Ok(target) = fs::read_link(fe.path()) else {
                    continue;
                };
                if !target.as_os_str().as_bytes().starts_with(b"/dev/dri/") {
                    continue;
                }
                let Ok(info) = fs::read_to_string(format!(
                    "/proc/{pid}/fdinfo/{}",
                    fe.file_name().to_string_lossy()
                )) else {
                    continue;
                };

                let mut pdev = None;
                let mut client_id = None;
                let mut engines_ns: HashMap<String, u64> = HashMap::new();
                let mut capacity: HashMap<String, u64> = HashMap::new();
                let mut cycles: HashMap<String, u64> = HashMap::new();
                let mut total_cycles: HashMap<String, u64> = HashMap::new();
                let (mut resident, mut has_resident, mut legacy_vram) = (0u64, false, 0u64);

                for line in info.lines() {
                    let Some((k, v)) = line.split_once(':') else {
                        continue;
                    };
                    let v = v.trim();
                    if k == "drm-pdev" {
                        pdev = Some(v.to_string());
                    } else if k == "drm-client-id" {
                        client_id = v.parse::<u64>().ok();
                    } else if let Some(e) = k.strip_prefix("drm-engine-capacity-") {
                        capacity.insert(e.into(), v.parse().unwrap_or(1));
                    } else if let Some(e) = k.strip_prefix("drm-engine-") {
                        let ns = v
                            .split_whitespace()
                            .next()
                            .and_then(|x| x.parse().ok())
                            .unwrap_or(0);
                        engines_ns.insert(e.into(), ns);
                    } else if let Some(e) = k.strip_prefix("drm-total-cycles-") {
                        total_cycles.insert(e.into(), v.parse().unwrap_or(0));
                    } else if let Some(e) = k.strip_prefix("drm-cycles-") {
                        cycles.insert(e.into(), v.parse().unwrap_or(0));
                    } else if let Some(r) = k.strip_prefix("drm-resident-") {
                        if is_vram_region(r) {
                            resident += parse_mem(v);
                            has_resident = true;
                        }
                    } else if k == "drm-memory-vram" {
                        legacy_vram += parse_mem(v);
                    }
                }
                let (Some(pdev), Some(cid)) = (pdev, client_id) else {
                    continue;
                };
                let key = (pdev.clone(), cid);
                // the same client can be reachable through several (dup'd) fds
                if !seen.insert(key.clone()) {
                    continue;
                }
                let vram = if has_resident { resident } else { legacy_vram };
                let prev = self.prev.get(&key);
                let du = out.entry(pdev).or_default();
                du.clients += 1;

                let mut proc_max = 0.0f64;
                let cap = |e: &str| capacity.get(e).copied().unwrap_or(1).max(1) as f64;
                for (eng, ns) in &engines_ns {
                    let busy = match prev.and_then(|p| p.engines_ns.get(eng)) {
                        Some(&p) if dt_ns > 0.0 => {
                            ns.saturating_sub(p) as f64 / (dt_ns * cap(eng)) * 100.0
                        }
                        _ => 0.0,
                    };
                    *du.engines.entry(eng.clone()).or_default() += busy;
                    proc_max = proc_max.max(busy);
                }
                // xe reports GPU cycles instead of ns
                let mut cyc_pairs = HashMap::new();
                for (eng, c) in &cycles {
                    let tc = total_cycles.get(eng).copied().unwrap_or(0);
                    cyc_pairs.insert(eng.clone(), (*c, tc));
                    let busy = match prev.and_then(|p| p.cycles.get(eng)) {
                        Some(&(pc, ptc)) if tc > ptc => {
                            c.saturating_sub(pc) as f64 / ((tc - ptc) as f64 * cap(eng)) * 100.0
                        }
                        _ => 0.0,
                    };
                    *du.engines.entry(eng.clone()).or_default() += busy;
                    proc_max = proc_max.max(busy);
                }

                let pg = du.procs.entry(pid).or_default();
                pg.util = (pg.util + proc_max).min(100.0);
                pg.vram += vram;
                du.vram += vram;
                next_prev.insert(
                    key,
                    Prev {
                        engines_ns,
                        cycles: cyc_pairs,
                    },
                );
            }
        }
        for du in out.values_mut() {
            for v in du.engines.values_mut() {
                *v = v.min(100.0);
            }
        }
        self.prev = next_prev;
        out
    }
}
