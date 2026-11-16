use crate::history::History;
use std::{collections::HashMap, fs};

pub struct Mem {
    /// Physically installed RAM (firmware memory map, rounded up to GiB).
    pub installed: Option<u64>,
    /// MemTotal: installed minus firmware/kernel/crashkernel reservations.
    pub total: u64,
    pub used: u64,
    pub avail: u64,
    pub cached: u64,
    pub buffers: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub hist: History,
}

/// Sum of "System RAM" regions in the firmware (e820) memory map, which is
/// world-readable. Firmware keeps a few MiB, so round up to a whole GiB.
fn installed_ram() -> Option<u64> {
    const GIB: u64 = 1 << 30;
    let hex = |p: std::path::PathBuf| {
        crate::util::read_str(p)
            .and_then(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16).ok())
    };
    let total: u64 = fs::read_dir("/sys/firmware/memmap")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|d| crate::util::read_str(d.join("type")).as_deref() == Some("System RAM"))
        .filter_map(|d| Some(hex(d.join("end"))? - hex(d.join("start"))? + 1))
        .sum();
    (total > 0).then(|| total.div_ceil(GIB) * GIB)
}

impl Mem {
    pub fn new() -> Self {
        let mut m = Self {
            installed: installed_ram(),
            total: 0,
            used: 0,
            avail: 0,
            cached: 0,
            buffers: 0,
            swap_total: 0,
            swap_used: 0,
            hist: History::new(1000),
        };
        m.read();
        m
    }

    fn read(&mut self) {
        let s = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let kv: HashMap<&str, u64> = s
            .lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                Some((k, v.split_whitespace().next()?.parse::<u64>().ok()? * 1024))
            })
            .collect();
        let g = |k| kv.get(k).copied().unwrap_or(0);
        self.total = g("MemTotal");
        self.avail = g("MemAvailable");
        self.used = self.total.saturating_sub(self.avail);
        self.cached = g("Cached") + g("SReclaimable");
        self.buffers = g("Buffers");
        self.swap_total = g("SwapTotal");
        self.swap_used = self.swap_total.saturating_sub(g("SwapFree"));
    }

    pub fn update(&mut self) {
        self.read();
        let pct = if self.total > 0 {
            self.used as f64 / self.total as f64 * 100.0
        } else {
            0.0
        };
        self.hist.push(pct);
    }
}
