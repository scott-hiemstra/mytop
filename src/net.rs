use crate::history::History;
use std::{collections::HashMap, fs, path::Path};

pub struct Iface {
    pub name: String,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub rx_total: u64,
    pub tx_total: u64,
    pub physical: bool,
}

pub struct Net {
    prev: HashMap<String, (u64, u64)>,
    pub ifaces: Vec<Iface>,
    pub rx: f64,
    pub tx: f64,
    pub rx_hist: History,
    pub tx_hist: History,
}

fn read() -> Vec<(String, u64, u64)> {
    let s = fs::read_to_string("/proc/net/dev").unwrap_or_default();
    s.lines()
        .skip(2)
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let f: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            Some((name.trim().to_string(), *f.first()?, *f.get(8)?))
        })
        .filter(|(n, _, _)| n != "lo")
        .collect()
}

impl Net {
    pub fn new() -> Self {
        let prev = read().into_iter().map(|(n, r, t)| (n, (r, t))).collect();
        Self {
            prev,
            ifaces: vec![],
            rx: 0.0,
            tx: 0.0,
            rx_hist: History::new(1000),
            tx_hist: History::new(1000),
        }
    }

    pub fn update(&mut self, dt: f64) {
        let cur = read();
        let mut ifaces: Vec<Iface> = cur
            .iter()
            .map(|(name, rx, tx)| {
                let (prx, ptx) = self.prev.get(name).copied().unwrap_or((*rx, *tx));
                Iface {
                    name: name.clone(),
                    rx_rate: rx.saturating_sub(prx) as f64 / dt,
                    tx_rate: tx.saturating_sub(ptx) as f64 / dt,
                    rx_total: *rx,
                    tx_total: *tx,
                    physical: Path::new(&format!("/sys/class/net/{name}/device")).exists(),
                }
            })
            .collect();
        self.prev = cur.into_iter().map(|(n, r, t)| (n, (r, t))).collect();

        // Totals count only physical NICs so bridges/veths don't double count.
        let any_phys = ifaces.iter().any(|i| i.physical);
        let counted = ifaces.iter().filter(|i| i.physical || !any_phys);
        let (rx, tx) = counted.fold((0.0, 0.0), |(r, t), i| (r + i.rx_rate, t + i.tx_rate));
        self.rx = rx;
        self.tx = tx;
        self.rx_hist.push(rx);
        self.tx_hist.push(tx);

        ifaces.sort_by(|a, b| {
            b.physical
                .cmp(&a.physical)
                .then((b.rx_rate + b.tx_rate).total_cmp(&(a.rx_rate + a.tx_rate)))
                .then(a.name.cmp(&b.name))
        });
        self.ifaces = ifaces;
    }
}
