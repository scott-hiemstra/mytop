//! AMD (amdgpu) via sysfs. Untested on real hardware — written against the
//! documented amdgpu sysfs/hwmon interface (Documentation/gpu/amdgpu/thermal.rst).

use super::{GpuSample, fdinfo::DevUsage};
use crate::util::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct AmdGpu {
    dev: PathBuf,
    hwmon: Option<PathBuf>,
}

impl AmdGpu {
    pub fn new(dev: &Path) -> Self {
        let hwmon = fs::read_dir(dev.join("hwmon"))
            .ok()
            .and_then(|mut d| d.next()?.ok().map(|e| e.path()));
        Self {
            dev: dev.to_path_buf(),
            hwmon,
        }
    }

    /// Current sclk from pp_dpm_sclk ("1: 1800Mhz *"), used if hwmon has no freq1.
    fn dpm_clock(&self, file: &str) -> Option<u32> {
        let s = read_str(self.dev.join(file))?;
        let l = s.lines().find(|l| l.trim_end().ends_with('*'))?;
        let mhz = l.split_whitespace().nth(1)?.to_lowercase();
        mhz.trim_end_matches("mhz").parse().ok()
    }

    pub fn sample(&mut self, du: Option<&DevUsage>) -> GpuSample {
        let d = &self.dev;
        let mut s = GpuSample {
            util: read_num(d.join("gpu_busy_percent")),
            mem_used: read_num(d.join("mem_info_vram_used")),
            mem_total: read_num(d.join("mem_info_vram_total")),
            ..Default::default()
        };
        if let Some(mb) = read_num::<u32>(d.join("mem_busy_percent")) {
            s.extra.push(("mem bus".into(), format!("{mb}%")));
        }
        if let Some(h) = &self.hwmon {
            s.temp = read_num::<f64>(h.join("temp1_input")).map(|t| t / 1000.0);
            if let Some(j) = read_num::<f64>(h.join("temp2_input")) {
                s.extra
                    .push(("hotspot".into(), format!("{:.0}°C", j / 1000.0)));
            }
            s.power = read_num::<f64>(h.join("power1_average"))
                .or_else(|| read_num(h.join("power1_input")))
                .map(|uw| uw / 1e6);
            s.power_limit = read_num::<f64>(h.join("power1_cap")).map(|uw| uw / 1e6);
            s.clock = read_num::<u64>(h.join("freq1_input")).map(|hz| (hz / 1_000_000) as u32);
            s.mem_clock = read_num::<u64>(h.join("freq2_input")).map(|hz| (hz / 1_000_000) as u32);
            s.fan_rpm = read_num(h.join("fan1_input"));
        }
        if s.clock.is_none() {
            s.clock = self.dpm_clock("pp_dpm_sclk");
        }
        if s.mem_clock.is_none() {
            s.mem_clock = self.dpm_clock("pp_dpm_mclk");
        }
        if let Some(du) = du {
            s.engines = du.engines.iter().map(|(k, v)| (k.clone(), *v)).collect();
            s.enc = du
                .engines
                .iter()
                .filter(|(k, _)| k.starts_with("enc"))
                .map(|(_, v)| *v)
                .reduce(f64::max);
            s.dec = du
                .engines
                .iter()
                .filter(|(k, _)| k.starts_with("dec"))
                .map(|(_, v)| *v)
                .reduce(f64::max);
        }
        s
    }
}
