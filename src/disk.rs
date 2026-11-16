use crate::history::History;
use std::{collections::HashMap, fs, path::Path};

pub struct DiskDev {
    pub name: String,
    pub read_rate: f64,
    pub write_rate: f64,
    pub busy: f64,
}

pub struct Disk {
    prev: HashMap<String, (u64, u64, u64)>,
    pub devs: Vec<DiskDev>,
    pub read: f64,
    pub write: f64,
    pub read_hist: History,
    pub write_hist: History,
}

/// (name, sectors_read, sectors_written, ms_doing_io) for real block devices.
fn read() -> Vec<(String, u64, u64, u64)> {
    let s = fs::read_to_string("/proc/diskstats").unwrap_or_default();
    s.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let name = f.get(2)?.to_string();
            // whole disks backed by hardware only (skips partitions, loop, dm, zram)
            if !Path::new(&format!("/sys/block/{name}/device")).exists() {
                return None;
            }
            let n = |i: usize| f.get(i).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
            Some((name, n(5), n(9), n(12)))
        })
        .collect()
}

impl Disk {
    pub fn new() -> Self {
        let prev = read()
            .into_iter()
            .map(|(n, r, w, b)| (n, (r, w, b)))
            .collect();
        Self {
            prev,
            devs: vec![],
            read: 0.0,
            write: 0.0,
            read_hist: History::new(1000),
            write_hist: History::new(1000),
        }
    }

    pub fn update(&mut self, dt: f64) {
        let cur = read();
        // /proc/diskstats sectors are always 512 bytes regardless of device sector size
        self.devs = cur
            .iter()
            .map(|(name, r, w, b)| {
                let (pr, pw, pb) = self.prev.get(name).copied().unwrap_or((*r, *w, *b));
                DiskDev {
                    name: name.clone(),
                    read_rate: r.saturating_sub(pr) as f64 * 512.0 / dt,
                    write_rate: w.saturating_sub(pw) as f64 * 512.0 / dt,
                    busy: (b.saturating_sub(pb) as f64 / (dt * 1000.0) * 100.0).min(100.0),
                }
            })
            .collect();
        self.prev = cur.into_iter().map(|(n, r, w, b)| (n, (r, w, b))).collect();
        self.read = self.devs.iter().map(|d| d.read_rate).sum();
        self.write = self.devs.iter().map(|d| d.write_rate).sum();
        self.read_hist.push(self.read);
        self.write_hist.push(self.write);
        self.devs.sort_by(|a, b| a.name.cmp(&b.name));
    }
}
