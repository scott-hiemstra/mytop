use crate::util::*;
use std::{fs, path::Path};

pub struct Chip {
    pub name: String,
    pub temps: Vec<(String, f64)>,
    pub fans: Vec<(String, u64)>,
}

/// Every hwmon chip on the system: CPU (k10temp/coretemp/zenpower), NVMe,
/// GPUs (amdgpu/i915/xe), motherboard Super-I/O (nct67xx/it87) etc.
pub struct Sensors {
    pub chips: Vec<Chip>,
    pub cpu_temp: Option<f64>,
}

fn read_chip(dir: &Path) -> Option<Chip> {
    let name = read_str(dir.join("name"))?;
    let label = |kind: &str, i: u32| {
        read_str(dir.join(format!("{kind}{i}_label"))).unwrap_or_else(|| format!("{kind}{i}"))
    };
    let temps = indexed_files(dir, "temp", "_input")
        .into_iter()
        .filter_map(|(i, p)| Some((label("temp", i), read_num::<f64>(p)? / 1000.0)))
        .filter(|(_, t)| *t > -50.0 && *t < 200.0) // drop bogus readings
        .collect();
    let fans = indexed_files(dir, "fan", "_input")
        .into_iter()
        .filter_map(|(i, p)| Some((label("fan", i), read_num::<u64>(p)?)))
        .collect();
    Some(Chip { name, temps, fans })
}

impl Sensors {
    pub fn new() -> Self {
        let mut s = Self {
            chips: vec![],
            cpu_temp: None,
        };
        s.update();
        s
    }

    pub fn update(&mut self) {
        let mut dirs: Vec<_> = fs::read_dir("/sys/class/hwmon")
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        dirs.sort_by_key(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.trim_start_matches("hwmon").parse::<u32>().ok())
                .unwrap_or(0)
        });
        self.chips = dirs
            .iter()
            .filter_map(|d| read_chip(d))
            .filter(|c| !c.temps.is_empty() || !c.fans.is_empty())
            .collect();

        self.cpu_temp = None;
        for (chip, prefs) in [
            ("k10temp", &["Tctl", "Tdie"][..]),
            ("zenpower", &["Tdie", "Tctl"][..]),
            ("coretemp", &["Package id 0"][..]),
            ("cpu_thermal", &[][..]),
        ] {
            if let Some(c) = self.chips.iter().find(|c| c.name == chip) {
                self.cpu_temp = prefs
                    .iter()
                    .find_map(|p| c.temps.iter().find(|(l, _)| l == p))
                    .or(c.temps.first())
                    .map(|(_, t)| *t);
                if self.cpu_temp.is_some() {
                    break;
                }
            }
        }
    }
}
