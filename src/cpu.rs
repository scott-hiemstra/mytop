use crate::{history::History, util::*};
use std::fs;

#[derive(Clone, Copy, Default)]
struct Times {
    busy: u64,
    total: u64,
}

fn read_stat() -> Vec<Times> {
    let s = fs::read_to_string("/proc/stat").unwrap_or_default();
    s.lines()
        .take_while(|l| l.starts_with("cpu"))
        .map(|l| {
            // user nice system idle iowait irq softirq steal (guest is already in user)
            let v: Vec<u64> = l
                .split_whitespace()
                .skip(1)
                .filter_map(|x| x.parse().ok())
                .collect();
            let total: u64 = v.iter().take(8).sum();
            let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
            Times {
                busy: total.saturating_sub(idle),
                total,
            }
        })
        .collect()
}

fn clean_model(s: &str) -> String {
    let mut m = s
        .trim()
        .replace("(R)", "")
        .replace("(TM)", "")
        .replace("(tm)", "");
    for cut in [" with Radeon", " CPU @", " Processor"] {
        if let Some(i) = m.find(cut) {
            m.truncate(i);
        }
    }
    m.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub struct Cpu {
    prev: Vec<Times>,
    pub total: f64,
    pub cores: Vec<f64>,
    pub model: String,
    pub freq_mhz: Option<f64>,
    pub load: [f64; 3],
    pub uptime: f64,
    pub hist: History,
}

impl Cpu {
    pub fn new() -> Self {
        let prev = read_stat();
        let model = fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("model name") || l.starts_with("Model"))
                    .and_then(|l| l.split_once(':'))
                    .map(|(_, v)| clean_model(v))
            })
            .unwrap_or_else(|| "CPU".into());
        let n = prev.len().saturating_sub(1);
        Self {
            prev,
            total: 0.0,
            cores: vec![0.0; n],
            model,
            freq_mhz: None,
            load: [0.0; 3],
            uptime: 0.0,
            hist: History::new(1000),
        }
    }

    pub fn update(&mut self) {
        let cur = read_stat();
        let pct = |a: &Times, b: &Times| {
            let dt = b.total.saturating_sub(a.total);
            if dt == 0 {
                0.0
            } else {
                b.busy.saturating_sub(a.busy) as f64 / dt as f64 * 100.0
            }
        };
        if let (Some(a), Some(b)) = (self.prev.first(), cur.first()) {
            self.total = pct(a, b);
        }
        self.cores = cur
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, b)| self.prev.get(i).map(|a| pct(a, b)).unwrap_or(0.0))
            .collect();
        self.prev = cur;
        self.hist.push(self.total);

        let (mut sum, mut n) = (0.0, 0);
        for i in 0..self.cores.len() {
            if let Some(khz) = read_num::<f64>(format!(
                "/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq"
            )) {
                sum += khz;
                n += 1;
            }
        }
        self.freq_mhz = (n > 0).then(|| sum / n as f64 / 1000.0);

        if let Some(s) = read_str("/proc/loadavg") {
            let v: Vec<f64> = s
                .split_whitespace()
                .take(3)
                .filter_map(|x| x.parse().ok())
                .collect();
            if v.len() == 3 {
                self.load = [v[0], v[1], v[2]];
            }
        }
        if let Some(s) = read_str("/proc/uptime") {
            self.uptime = s
                .split_whitespace()
                .next()
                .and_then(|x| x.parse().ok())
                .unwrap_or(0.0);
        }
    }
}
