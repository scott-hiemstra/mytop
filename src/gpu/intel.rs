//! Intel (i915 and xe) via sysfs, hwmon, fdinfo and the i915 query ioctl.

use super::{GpuSample, fdinfo::DevUsage};
use crate::util::*;
use std::{
    fs::{self, File},
    collections::BTreeMap,
    io::Read,
    os::fd::{AsRawFd, FromRawFd},
    path::{Path, PathBuf},
    time::Instant,
};

pub struct IntelGpu {
    xe: bool,
    hwmon: Option<PathBuf>,
    idle_files: Vec<PathBuf>,
    freq_file: Option<PathBuf>,
    render: Option<File>,
    prev_energy: Option<(u64, Instant)>,
    prev_idle: Option<(f64, Instant)>,
    pmu: Option<Pmu>,
    is_root: bool,
}

fn glob2(base: &Path, a: &str, b: &str, tail: &str) -> Vec<PathBuf> {
    // base/<a*>/<b*>/tail
    let mut out = vec![];
    for x in fs::read_dir(base).into_iter().flatten().flatten() {
        if !x.file_name().to_string_lossy().starts_with(a) {
            continue;
        }
        for y in fs::read_dir(x.path()).into_iter().flatten().flatten() {
            if y.file_name().to_string_lossy().starts_with(b) {
                let p = y.path().join(tail);
                if p.exists() {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

impl IntelGpu {
    pub fn new(card: &Path, dev: &Path, xe: bool) -> Self {
        let hwmon = fs::read_dir(dev.join("hwmon"))
            .ok()
            .and_then(|mut d| d.next()?.ok().map(|e| e.path()));
        let (idle_files, freq_file) = if xe {
            (
                glob2(dev, "tile", "gt", "gtidle/idle_residency_ms"),
                glob2(dev, "tile", "gt", "freq0/act_freq")
                    .into_iter()
                    .next(),
            )
        } else {
            let mut idle: Vec<PathBuf> = fs::read_dir(card.join("gt"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path().join("rc6_residency_ms"))
                .filter(|p| p.exists())
                .collect();
            idle.sort();
            if idle.is_empty() && card.join("power/rc6_residency_ms").exists() {
                idle.push(card.join("power/rc6_residency_ms"));
            }
            let freq = [
                card.join("gt/gt0/rps_act_freq_mhz"),
                card.join("gt_act_freq_mhz"),
            ]
            .into_iter()
            .find(|p| p.exists());
            (idle, freq)
        };
        let render = fs::read_dir(dev.join("drm"))
            .into_iter()
            .flatten()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().starts_with("renderD"))
            .and_then(|e| {
                File::options()
                    .read(true)
                    .write(true)
                    .open(Path::new("/dev/dri").join(e.file_name()))
                    .ok()
            });
        let pmu = if xe { None } else { Pmu::open(dev) };
        Self {
            xe,
            hwmon,
            idle_files,
            freq_file,
            render,
            prev_energy: None,
            prev_idle: None,
            pmu,
            is_root: unsafe { libc::geteuid() } == 0,
        }
    }

    pub fn sample(&mut self, du: Option<&DevUsage>) -> GpuSample {
        let now = Instant::now();
        let mut s = GpuSample::default();

        // Engine utilisation from fdinfo (what nvtop / intel_gpu_top -p show)
        if let Some(du) = du {
            s.util = Some(du.max_engine());
            s.engines = du.engines.iter().map(|(k, v)| (k.clone(), *v)).collect();
            if du.vram > 0 {
                s.mem_used = Some(du.vram);
            }
        } else {
            s.util = Some(0.0);
        }

        // Device-wide engine busy from the i915 PMU covers every client,
        // including other users' processes that fdinfo cannot see.
        if let Some(engines) = self.pmu.as_mut().and_then(|p| p.sample(now)) {
            s.util = Some(engines.values().copied().fold(0.0, f64::max));
            s.engines = engines.into_iter().collect();
        }

        // "Awake" = time not in RC6 / gt-idle — device-wide, works without root
        if !self.idle_files.is_empty() {
            let idle: f64 = self
                .idle_files
                .iter()
                .filter_map(read_num::<f64>)
                .sum::<f64>()
                / self.idle_files.len() as f64;
            if let Some((pi, pt)) = self.prev_idle {
                let ms = now.duration_since(pt).as_secs_f64() * 1000.0;
                if ms > 0.0 {
                    let awake = (100.0 - (idle - pi) / ms * 100.0).clamp(0.0, 100.0);
                    s.extra.push(("awake".into(), format!("{awake:.0}%")));
                    // Without root (and without the PMU) fdinfo only sees our
                    // own processes; the GT can't be busier than it is awake,
                    // and it drops into RC6 within ms of going idle, so awake
                    // is the best device-wide estimate left.
                    if self.pmu.is_none() && !self.is_root {
                        s.util = Some(s.util.unwrap_or(0.0).max(awake));
                    }
                }
            }
            self.prev_idle = Some((idle, now));
        }

        s.clock = self.freq_file.as_ref().and_then(read_num);

        if let Some(h) = &self.hwmon {
            s.temp = indexed_files(h, "temp", "_input")
                .first()
                .and_then(|(_, p)| read_num::<f64>(p))
                .map(|t| t / 1000.0);
            s.fan_rpm = read_num(h.join("fan1_input"));
            s.power_limit = read_num::<f64>(h.join("power1_max"))
                .filter(|v| *v > 0.0)
                .map(|uw| uw / 1e6);
            let energy = read_num::<u64>(h.join("energy1_input"))
                .or_else(|| read_num(h.join("energy2_input")));
            if let Some(e) = energy {
                if let Some((pe, pt)) = self.prev_energy {
                    let dt = now.duration_since(pt).as_secs_f64();
                    if dt > 0.0 && e >= pe {
                        s.power = Some((e - pe) as f64 / 1e6 / dt);
                    }
                }
                self.prev_energy = Some((e, now));
            }
        }

        if !self.xe
            && let Some((total, used)) = self.render.as_ref().and_then(i915_vram)
        {
            s.mem_total = Some(total);
            if let Some(u) = used {
                s.mem_used = Some(u);
            }
        }
        s
    }
}

/// i915 PMU engine busy counters (what `intel_gpu_top` reads). System-wide
/// perf events need CAP_PERFMON / root or `kernel.perf_event_paranoid <= 0`.
struct Pmu {
    /// (engine class name, counter file)
    counters: Vec<(String, File)>,
    prev: Option<(Vec<u64>, Instant)>,
}

impl Pmu {
    fn open(dev: &Path) -> Option<Self> {
        let pci = fs::canonicalize(dev).ok()?.file_name()?.to_str()?.to_string();
        let base = Path::new("/sys/bus/event_source/devices");
        let dir = [format!("i915_{}", pci.replace(':', "_")), "i915".into()]
            .into_iter()
            .map(|n| base.join(n))
            .find(|d| d.exists())?;
        let ty: u32 = read_num(dir.join("type"))?;
        let cpu: i32 = read_str(dir.join("cpumask"))
            .and_then(|s| s.split([',', '-']).next()?.trim().parse().ok())
            .unwrap_or(0);
        let mut events: Vec<(String, u64)> = fs::read_dir(dir.join("events"))
            .ok()?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.strip_suffix("-busy")?.to_string();
                let cfg = read_str(e.path())?;
                let cfg = u64::from_str_radix(cfg.trim().strip_prefix("config=0x")?, 16).ok()?;
                Some((name, cfg))
            })
            .collect();
        events.sort();
        let mut counters = vec![];
        for (name, cfg) in events {
            // struct perf_event_attr: type@0 (u32), size@4 (u32), config@8 (u64)
            let mut attr = [0u64; 17];
            attr[0] = ty as u64 | ((size_of_val(&attr) as u64) << 32);
            attr[1] = cfg;
            let fd = unsafe {
                libc::syscall(libc::SYS_perf_event_open, attr.as_ptr(), -1, cpu, -1, 0)
            };
            if fd < 0 {
                return None; // permission denied: all or nothing
            }
            let class = name.trim_end_matches(|c: char| c.is_ascii_digit());
            let class = match class {
                "rcs" => "render",
                "bcs" => "copy",
                "vcs" => "video",
                "vecs" => "video-enhance",
                "ccs" => "compute",
                o => o,
            };
            counters.push((class.to_string(), unsafe { File::from_raw_fd(fd as i32) }));
        }
        (!counters.is_empty()).then_some(Self {
            counters,
            prev: None,
        })
    }

    /// Engine class -> busy %, averaged over instances (like fdinfo capacity).
    fn sample(&mut self, now: Instant) -> Option<BTreeMap<String, f64>> {
        let vals: Vec<u64> = self
            .counters
            .iter_mut()
            .map(|(_, f)| {
                let mut b = [0u8; 8];
                f.read_exact(&mut b).map(|_| u64::from_ne_bytes(b)).unwrap_or(0)
            })
            .collect();
        let prev = self.prev.replace((vals.clone(), now))?;
        let dt = now.duration_since(prev.1).as_nanos() as f64;
        if dt <= 0.0 {
            return None;
        }
        let mut sum: BTreeMap<String, (f64, u32)> = BTreeMap::new();
        for (i, (class, _)) in self.counters.iter().enumerate() {
            let pct = (vals[i].saturating_sub(prev.0[i]) as f64 / dt * 100.0).min(100.0);
            let e = sum.entry(class.clone()).or_default();
            e.0 += pct;
            e.1 += 1;
        }
        Some(sum.into_iter().map(|(k, (s, n))| (k, s / n as f64)).collect())
    }
}

/// DRM_IOCTL_I915_QUERY / DRM_I915_QUERY_MEMORY_REGIONS.
/// Returns (local memory total, used). `used` needs CAP_PERFMON (root);
/// unprivileged callers get unallocated == probed.
fn i915_vram(f: &File) -> Option<(u64, Option<u64>)> {
    #[repr(C)]
    struct Item {
        query_id: u64,
        length: i32,
        flags: u32,
        data_ptr: u64,
    }
    #[repr(C)]
    struct Query {
        num_items: u32,
        flags: u32,
        items_ptr: u64,
    }
    // _IOWR('d', DRM_COMMAND_BASE + DRM_I915_QUERY, struct drm_i915_query)
    const IOCTL: u64 =
        (3 << 30) | ((size_of::<Query>() as u64) << 16) | (0x64 << 8) | (0x40 + 0x39);
    const QUERY_MEMORY_REGIONS: u64 = 4;
    const REGION_SIZE: usize = 88; // sizeof(struct drm_i915_memory_region_info)
    const CLASS_DEVICE: u16 = 1;

    let mut item = Item {
        query_id: QUERY_MEMORY_REGIONS,
        length: 0,
        flags: 0,
        data_ptr: 0,
    };
    let mut q = Query {
        num_items: 1,
        flags: 0,
        items_ptr: &mut item as *mut Item as u64,
    };
    if unsafe { libc::ioctl(f.as_raw_fd(), IOCTL as _, &mut q) } != 0 || item.length <= 0 {
        return None;
    }
    let mut buf = vec![0u8; item.length as usize];
    item.data_ptr = buf.as_mut_ptr() as u64;
    q.items_ptr = &mut item as *mut Item as u64;
    if unsafe { libc::ioctl(f.as_raw_fd(), IOCTL as _, &mut q) } != 0 || item.length <= 0 {
        return None;
    }
    let u16_at = |o: usize| u16::from_ne_bytes(buf[o..o + 2].try_into().unwrap());
    let u64_at = |o: usize| u64::from_ne_bytes(buf[o..o + 8].try_into().unwrap());
    let n = u32::from_ne_bytes(buf[0..4].try_into().ok()?) as usize;
    let (mut total, mut unalloc) = (0u64, 0u64);
    for i in 0..n {
        let o = 16 + i * REGION_SIZE;
        if o + 24 > buf.len() {
            break;
        }
        if u16_at(o) == CLASS_DEVICE {
            total += u64_at(o + 8);
            unalloc = unalloc.saturating_add(u64_at(o + 16));
        }
    }
    if total == 0 {
        return None;
    }
    let used = (unalloc < total).then(|| total - unalloc);
    Some((total, used))
}
