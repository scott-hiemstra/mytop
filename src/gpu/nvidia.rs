use super::{GpuSample, ProcGpu};
use nvml_wrapper::{
    Nvml,
    enum_wrappers::device::{Clock, TemperatureSensor},
    enums::device::UsedGpuMemory,
};
use std::collections::HashMap;

pub fn sample(
    nvml: &Nvml,
    idx: u32,
    last_ts: &mut u64,
    procs: &mut HashMap<u32, ProcGpu>,
) -> GpuSample {
    let d = match nvml.device_by_index(idx) {
        Ok(d) => d,
        Err(e) => {
            return GpuSample {
                error: Some(e.to_string()),
                ..Default::default()
            };
        }
    };
    let mut s = GpuSample::default();
    if let Ok(u) = d.utilization_rates() {
        s.util = Some(u.gpu as f64);
        s.extra.push(("mem bus".into(), format!("{}%", u.memory)));
    }
    if let Ok(m) = d.memory_info() {
        s.mem_used = Some(m.used);
        s.mem_total = Some(m.total);
    }
    s.temp = d.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f64);
    s.power = d.power_usage().ok().map(|mw| mw as f64 / 1000.0);
    s.power_limit = d.enforced_power_limit().ok().map(|mw| mw as f64 / 1000.0);
    s.clock = d.clock_info(Clock::Graphics).ok();
    s.mem_clock = d.clock_info(Clock::Memory).ok();
    s.fan_pct = d.fan_speed(0).ok();
    s.enc = d.encoder_utilization().ok().map(|u| u.utilization as f64);
    s.dec = d.decoder_utilization().ok().map(|u| u.utilization as f64);

    // per-process VRAM
    let mut mem: HashMap<u32, u64> = HashMap::new();
    let compute = d.running_compute_processes().unwrap_or_default();
    let graphics = d.running_graphics_processes().unwrap_or_default();
    for p in compute.into_iter().chain(graphics) {
        let b = match p.used_gpu_memory {
            UsedGpuMemory::Used(b) => b,
            UsedGpuMemory::Unavailable => 0,
        };
        let e = mem.entry(p.pid).or_default();
        *e = (*e).max(b);
    }
    // per-process SM utilisation since the last sample we saw
    let mut util: HashMap<u32, (u64, u64)> = HashMap::new();
    if let Ok(samples) = d.process_utilization_stats(*last_ts) {
        let mut newest = *last_ts;
        for x in samples {
            newest = newest.max(x.timestamp);
            let e = util.entry(x.pid).or_default();
            e.0 += x.sm_util as u64;
            e.1 += 1;
        }
        *last_ts = newest;
    }
    for pid in mem.keys().chain(util.keys()).copied().collect::<Vec<_>>() {
        let e = procs.entry(pid).or_default();
        e.vram += mem.get(&pid).copied().unwrap_or(0);
        if let Some((sum, n)) = util.get(&pid) {
            e.util = (e.util + *sum as f64 / *n as f64).min(100.0);
        }
    }
    s
}
