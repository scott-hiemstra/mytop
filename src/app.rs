use crate::{
    config::Settings, cpu::Cpu, disk::Disk, gpu::Gpus, mem::Mem, net::Net, procs::Procs,
    sensors::Sensors,
};
use ratatui::widgets::TableState;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Cpu,
    Mem,
    Gpu,
    Pid,
    Name,
}

impl SortKey {
    pub fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Mem,
            Self::Mem => Self::Gpu,
            Self::Gpu => Self::Pid,
            Self::Pid => Self::Name,
            Self::Name => Self::Cpu,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Mem => "mem",
            Self::Gpu => "gpu",
            Self::Pid => "pid",
            Self::Name => "name",
        }
    }
    pub fn from_label(s: &str) -> Option<Self> {
        [Self::Cpu, Self::Mem, Self::Gpu, Self::Pid, Self::Name]
            .into_iter()
            .find(|k| k.label() == s)
    }
}

pub const PANELS: [&str; 7] = ["cpu", "mem", "gpu", "net", "disk", "sensors", "procs"];

pub struct App {
    pub cpu: Cpu,
    pub mem: Mem,
    pub net: Net,
    pub disk: Disk,
    pub sensors: Sensors,
    pub gpus: Gpus,
    pub procs: Procs,
    pub interval: Duration,
    last_update: Instant,
    pub sort: SortKey,
    pub reverse: bool,
    pub table: TableState,
    pub selected_pid: Option<u32>,
    pub show: [bool; 7],
    pub paused: bool,
    pub is_root: bool,
    pub pending_kill: Option<(u32, String, i32)>,
    pub status: Option<String>,
    pub timings: Vec<(&'static str, Duration)>,
}

impl App {
    pub fn new(settings: Settings) -> Self {
        let mut app = Self {
            cpu: Cpu::new(),
            mem: Mem::new(),
            net: Net::new(),
            disk: Disk::new(),
            sensors: Sensors::new(),
            gpus: Gpus::new(),
            procs: Procs::new(),
            interval: settings.interval,
            last_update: Instant::now(),
            sort: settings.sort,
            reverse: settings.reverse,
            table: TableState::default().with_selected(0),
            selected_pid: None,
            show: settings.show,
            paused: false,
            is_root: unsafe { libc::geteuid() } == 0,
            pending_kill: None,
            status: None,
            timings: vec![],
        };
        // prime the delta-based collectors so the first frame has real numbers
        std::thread::sleep(Duration::from_millis(250));
        app.update();
        app
    }

    /// The view settings that are remembered between runs.
    pub fn settings(&self) -> Settings {
        Settings {
            interval: self.interval,
            sort: self.sort,
            reverse: self.reverse,
            show: self.show,
        }
    }

    pub fn apply(&mut self, s: Settings) {
        self.interval = s.interval;
        self.sort = s.sort;
        self.reverse = s.reverse;
        self.show = s.show;
        self.selected_pid = None;
        self.table.select(Some(0));
        self.sort_procs();
    }

    pub fn update(&mut self) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_update)
            .as_secs_f64()
            .max(0.001);
        self.last_update = now;
        let mut t = Instant::now();
        let mut lap = |name: &'static str, timings: &mut Vec<(&'static str, Duration)>| {
            timings.push((name, t.elapsed()));
            t = Instant::now();
        };
        let mut tm = Vec::with_capacity(8);
        self.cpu.update();
        lap("cpu", &mut tm);
        self.mem.update();
        lap("mem", &mut tm);
        self.net.update(dt);
        lap("net", &mut tm);
        self.disk.update(dt);
        lap("disk", &mut tm);
        self.sensors.update();
        lap("sensors", &mut tm);
        self.gpus.update();
        lap("gpu", &mut tm);
        self.procs.update(dt, self.mem.total, &self.gpus.procs);
        lap("procs", &mut tm);
        self.sort_procs();
        lap("sort", &mut tm);
        self.timings = tm;
    }

    pub fn sort_procs(&mut self) {
        let l = &mut self.procs.list;
        match self.sort {
            SortKey::Cpu => l.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.rss.cmp(&a.rss))),
            SortKey::Mem => l.sort_by_key(|p| std::cmp::Reverse(p.rss)),
            SortKey::Gpu => l.sort_by(|a, b| {
                b.gpu
                    .unwrap_or(-1.0)
                    .total_cmp(&a.gpu.unwrap_or(-1.0))
                    .then(b.vram.cmp(&a.vram))
            }),
            SortKey::Pid => l.sort_by_key(|p| p.pid),
            SortKey::Name => l.sort_by_key(|a| a.cmd.to_lowercase()),
        }
        if self.reverse {
            l.reverse();
        }
        // keep the cursor on the same process across refreshes
        let idx = self
            .selected_pid
            .and_then(|pid| l.iter().position(|p| p.pid == pid))
            .unwrap_or_else(|| {
                self.table
                    .selected()
                    .unwrap_or(0)
                    .min(l.len().saturating_sub(1))
            });
        self.table.select(Some(idx));
        self.selected_pid = l.get(idx).map(|p| p.pid);
    }

    pub fn move_selection(&mut self, delta: isize) {
        let len = self.procs.list.len();
        if len == 0 {
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as isize;
        let idx = (cur + delta).clamp(0, len as isize - 1) as usize;
        self.table.select(Some(idx));
        self.selected_pid = self.procs.list.get(idx).map(|p| p.pid);
    }

    pub fn request_kill(&mut self, sig: i32) {
        if let Some(p) = self.table.selected().and_then(|i| self.procs.list.get(i)) {
            let name = p.cmd.split_whitespace().next().unwrap_or("").to_string();
            self.pending_kill = Some((p.pid, name, sig));
        }
    }

    pub fn confirm_kill(&mut self) {
        if let Some((pid, name, sig)) = self.pending_kill.take() {
            let r = unsafe { libc::kill(pid as i32, sig) };
            let sname = if sig == libc::SIGKILL {
                "SIGKILL"
            } else {
                "SIGTERM"
            };
            self.status = Some(if r == 0 {
                format!("sent {sname} to {pid} ({name})")
            } else {
                format!("kill {pid} failed: {}", std::io::Error::last_os_error())
            });
        }
    }
}
