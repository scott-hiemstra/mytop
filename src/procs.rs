use crate::gpu::ProcGpu;
use std::{collections::HashMap, fs, os::unix::fs::MetadataExt};

pub struct Proc {
    pub pid: u32,
    pub user: String,
    pub cmd: String,
    pub state: char,
    pub threads: u32,
    pub rss: u64,
    pub cpu: f64,
    pub mem: f64,
    pub gpu: Option<f64>,
    pub vram: Option<u64>,
}

pub struct Procs {
    /// pid -> (starttime, utime+stime) from the previous sample
    prev: HashMap<u32, (u64, u64)>,
    users: HashMap<u32, String>,
    cmd_cache: HashMap<(u32, u64), String>,
    pub list: Vec<Proc>,
    clk_tck: f64,
    page_size: u64,
}

fn load_users() -> HashMap<u32, String> {
    fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            Some((f.get(2)?.parse().ok()?, f.first()?.to_string()))
        })
        .collect()
}

fn read_cmd(pid: u32, comm: &str) -> String {
    let raw = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let args: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    match args.split_first() {
        None => format!("[{comm}]"), // kernel thread
        Some((argv0, rest)) => {
            let base = argv0.rsplit('/').next().unwrap_or(argv0);
            std::iter::once(base)
                .chain(rest.iter().map(|s| s.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

impl Procs {
    pub fn new() -> Self {
        let clk = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Self {
            prev: HashMap::new(),
            users: load_users(),
            cmd_cache: HashMap::new(),
            list: vec![],
            clk_tck: if clk > 0 { clk as f64 } else { 100.0 },
            page_size: if page > 0 { page as u64 } else { 4096 },
        }
    }

    pub fn update(&mut self, dt: f64, mem_total: u64, gpu: &HashMap<u32, ProcGpu>) {
        let mut list = Vec::with_capacity(self.prev.len().max(256));
        let mut next_prev = HashMap::with_capacity(self.prev.len());
        let mut next_cmd = HashMap::with_capacity(self.cmd_cache.len());

        for e in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let (Some(lp), Some(rp)) = (stat.find('('), stat.rfind(')')) else {
                continue;
            };
            let comm = &stat[lp + 1..rp];
            let f: Vec<&str> = stat[rp + 1..].split_whitespace().collect();
            // f[0] is field 3 (state) in proc(5) numbering
            let n = |field: usize| {
                f.get(field - 3)
                    .and_then(|x| x.parse::<u64>().ok())
                    .unwrap_or(0)
            };
            let state = f.first().and_then(|s| s.chars().next()).unwrap_or('?');
            let ticks = n(14) + n(15);
            let start = n(22);
            let rss = n(24) * self.page_size;

            let cpu = match self.prev.get(&pid) {
                Some(&(pstart, pticks)) if pstart == start && dt > 0.0 => {
                    ticks.saturating_sub(pticks) as f64 / self.clk_tck / dt * 100.0
                }
                _ => 0.0,
            };
            next_prev.insert(pid, (start, ticks));

            let uid = e.metadata().map(|m| m.uid()).unwrap_or(0);
            let user = self
                .users
                .get(&uid)
                .cloned()
                .unwrap_or_else(|| uid.to_string());

            let key = (pid, start);
            let cmd = self
                .cmd_cache
                .remove(&key)
                .unwrap_or_else(|| read_cmd(pid, comm));
            next_cmd.insert(key, cmd.clone());

            let g = gpu.get(&pid);
            list.push(Proc {
                pid,
                user,
                cmd,
                state,
                threads: n(20) as u32,
                rss,
                cpu,
                mem: if mem_total > 0 {
                    rss as f64 / mem_total as f64 * 100.0
                } else {
                    0.0
                },
                gpu: g.map(|g| g.util),
                vram: g.map(|g| g.vram),
            });
        }
        self.prev = next_prev;
        self.cmd_cache = next_cmd;
        self.list = list;
    }
}
