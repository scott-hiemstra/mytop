//! GPU monitoring.
//!
//! * NVIDIA – NVML (libnvidia-ml.so, loaded at runtime so the binary runs
//!   fine on machines without NVIDIA drivers).
//! * AMD (amdgpu) – sysfs (`gpu_busy_percent`, `mem_info_vram_*`, hwmon).
//! * Intel (i915 / xe) – sysfs + hwmon + i915 query ioctl for VRAM.
//! * AMD + Intel per-process/per-engine usage – DRM fdinfo
//!   (`/proc/<pid>/fdinfo/<fd>`), the same interface nvtop uses.

mod amd;
pub mod fdinfo;
mod intel;
mod nvidia;

use crate::{history::History, util::*};
use nvml_wrapper::Nvml;
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
}

#[derive(Default, Clone)]
pub struct GpuSample {
    pub util: Option<f64>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    pub temp: Option<f64>,
    pub power: Option<f64>,
    pub power_limit: Option<f64>,
    pub clock: Option<u32>,
    pub mem_clock: Option<u32>,
    pub fan_rpm: Option<u64>,
    pub fan_pct: Option<u32>,
    pub enc: Option<f64>,
    pub dec: Option<f64>,
    /// Per-engine busy % (from fdinfo), e.g. render/video/copy/compute.
    pub engines: Vec<(String, f64)>,
    /// Extra vendor-specific key/value stats.
    pub extra: Vec<(String, String)>,
    pub error: Option<String>,
}

#[derive(Default, Clone, Copy)]
pub struct ProcGpu {
    pub util: f64,
    pub vram: u64,
}

enum Backend {
    Nvidia(u32),
    Amd(amd::AmdGpu),
    Intel(intel::IntelGpu),
}

pub struct Gpu {
    backend: Backend,
    pub name: String,
    pub vendor: Vendor,
    pub driver: String,
    pub pci: String,
    pub s: GpuSample,
    pub util_hist: History,
    pub mem_hist: History,
}

pub struct Gpus {
    nvml: Option<Nvml>,
    nv_ts: HashMap<u32, u64>,
    scanner: fdinfo::Scanner,
    pub list: Vec<Gpu>,
    pub procs: HashMap<u32, ProcGpu>,
    pub nvml_error: Option<String>,
}

fn pci_ids_name(vendor: &str, device: &str) -> Option<String> {
    let v = vendor.trim_start_matches("0x").to_lowercase();
    let d = device.trim_start_matches("0x").to_lowercase();
    for p in [
        "/usr/share/hwdata/pci.ids",
        "/usr/share/misc/pci.ids",
        "/usr/share/pci.ids",
    ] {
        let Ok(s) = fs::read_to_string(p) else {
            continue;
        };
        let mut in_vendor = false;
        for line in s.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if !line.starts_with('\t') {
                if in_vendor {
                    break;
                }
                in_vendor = line.starts_with(&v) && line[v.len()..].starts_with("  ");
            } else if in_vendor && !line.starts_with("\t\t") {
                let l = &line[1..];
                if l.starts_with(&d) && l[d.len()..].starts_with("  ") {
                    return Some(l[d.len()..].trim().to_string());
                }
            }
        }
    }
    None
}

fn pretty_name(vendor: &str, raw: &str) -> String {
    match (raw.find('['), raw.rfind(']')) {
        (Some(a), Some(b)) if b > a => format!("{vendor} {}", &raw[a + 1..b]),
        _ => format!("{vendor} {raw}"),
    }
}

fn link_name(p: &Path) -> Option<String> {
    fs::read_link(p)
        .ok()?
        .file_name()?
        .to_str()
        .map(String::from)
}

impl Gpus {
    pub fn new() -> Self {
        let mut list = vec![];
        let mut nvml_error = None;
        let mut has_nvidia_card = false;

        // DRM cards: amdgpu / i915 / xe
        for e in fs::read_dir("/sys/class/drm")
            .into_iter()
            .flatten()
            .flatten()
        {
            let fname = e.file_name().to_string_lossy().into_owned();
            let Some(num) = fname.strip_prefix("card") else {
                continue;
            };
            if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let card = e.path();
            let dev = card.join("device");
            let Some(driver) = link_name(&dev.join("driver")) else {
                continue;
            };
            let Some(pci) = fs::canonicalize(&dev)
                .ok()
                .and_then(|p| p.file_name()?.to_str().map(String::from))
            else {
                continue;
            };
            let vendor_id = read_str(dev.join("vendor")).unwrap_or_default();
            let device_id = read_str(dev.join("device")).unwrap_or_default();
            let raw = read_str(dev.join("product_name"))
                .filter(|s| !s.is_empty())
                .or_else(|| pci_ids_name(&vendor_id, &device_id))
                .unwrap_or_else(|| device_id.clone());
            let (backend, vendor, name) = match driver.as_str() {
                "amdgpu" => (
                    Backend::Amd(amd::AmdGpu::new(&dev)),
                    Vendor::Amd,
                    pretty_name("AMD", &raw),
                ),
                "i915" | "xe" => (
                    Backend::Intel(intel::IntelGpu::new(&card, &dev, driver == "xe")),
                    Vendor::Intel,
                    pretty_name("Intel", &raw),
                ),
                "nvidia" => {
                    has_nvidia_card = true;
                    continue;
                }
                _ => continue,
            };
            list.push(Gpu {
                backend,
                name,
                vendor,
                driver,
                pci,
                s: GpuSample::default(),
                util_hist: History::new(1000),
                mem_hist: History::new(1000),
            });
        }

        // NVIDIA via NVML
        let nvml = match Nvml::init() {
            Ok(n) => Some(n),
            Err(e) => {
                if has_nvidia_card {
                    nvml_error = Some(format!("NVML unavailable: {e}"));
                }
                None
            }
        };
        if let Some(n) = &nvml {
            for i in 0..n.device_count().unwrap_or(0) {
                let Ok(d) = n.device_by_index(i) else {
                    continue;
                };
                let pci = d
                    .pci_info()
                    .map(|p| {
                        let b = p.bus_id.to_lowercase();
                        // NVML: "00000000:25:00.0" -> sysfs style "0000:25:00.0"
                        if b.len() > 12 {
                            b[b.len() - 12..].to_string()
                        } else {
                            b
                        }
                    })
                    .unwrap_or_default();
                list.push(Gpu {
                    backend: Backend::Nvidia(i),
                    name: d.name().unwrap_or_else(|_| format!("NVIDIA GPU {i}")),
                    vendor: Vendor::Nvidia,
                    driver: "nvidia".into(),
                    pci,
                    s: GpuSample::default(),
                    util_hist: History::new(1000),
                    mem_hist: History::new(1000),
                });
            }
        }

        list.sort_by(|a, b| a.pci.cmp(&b.pci));
        Self {
            nvml,
            nv_ts: HashMap::new(),
            scanner: fdinfo::Scanner::new(),
            list,
            procs: HashMap::new(),
            nvml_error,
        }
    }

    pub fn update(&mut self) {
        let needs_fdinfo = self.list.iter().any(|g| g.vendor != Vendor::Nvidia);
        let usage = if needs_fdinfo {
            self.scanner.scan()
        } else {
            HashMap::new()
        };
        self.procs.clear();

        for g in &mut self.list {
            let du = usage.get(&g.pci);
            g.s = match &mut g.backend {
                Backend::Nvidia(i) => match &self.nvml {
                    Some(n) => {
                        nvidia::sample(n, *i, self.nv_ts.entry(*i).or_default(), &mut self.procs)
                    }
                    None => GpuSample::default(),
                },
                Backend::Amd(a) => a.sample(du),
                Backend::Intel(x) => x.sample(du),
            };
            if let (Some(du), false) = (du, g.vendor == Vendor::Nvidia) {
                for (pid, pg) in &du.procs {
                    let e = self.procs.entry(*pid).or_default();
                    e.util = (e.util + pg.util).min(100.0);
                    e.vram += pg.vram;
                }
            }
            g.util_hist.push(g.s.util.unwrap_or(0.0));
            let mem_pct = match (g.s.mem_used, g.s.mem_total) {
                (Some(u), Some(t)) if t > 0 => u as f64 / t as f64 * 100.0,
                _ => 0.0,
            };
            g.mem_hist.push(mem_pct);
        }
    }
}
