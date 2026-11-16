mod app;
mod cpu;
mod disk;
mod gpu;
mod history;
mod mem;
mod net;
mod procs;
mod sensors;
mod ui;
mod util;

use app::App;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::time::{Duration, Instant};

fn parse_args() -> (Duration, bool) {
    let mut ms = 1000u64;
    let mut bench = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-i" | "--interval" => {
                ms = args.next().and_then(|v| v.parse().ok()).unwrap_or(ms);
            }
            "--bench" => bench = true,
            "-h" | "--help" => {
                println!("mytop — CPU / memory / GPU / network / disk / sensor monitor\n");
                println!(
                    "usage: mytop [-i MS]\n\n  -i, --interval MS   refresh interval in milliseconds (default 1000)\n      --bench         time each collector and exit"
                );
                println!(
                    "\nRun as root to see per-process GPU usage of other users' processes (AMD/Intel)."
                );
                std::process::exit(0);
            }
            _ => {
                eprintln!("unknown argument: {a} (try --help)");
                std::process::exit(2);
            }
        }
    }
    (Duration::from_millis(ms.clamp(100, 10_000)), bench)
}

fn bench() {
    let mut app = App::new(Duration::from_secs(1));
    let mut total: Vec<(&'static str, Duration)> = vec![];
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 55)).unwrap();
    const N: u32 = 10;
    for _ in 0..N {
        std::thread::sleep(Duration::from_millis(1000));
        app.update();
        let t = Instant::now();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        app.timings.push(("draw", t.elapsed()));
        if total.is_empty() {
            total = app.timings.clone();
        } else {
            for (a, b) in total.iter_mut().zip(&app.timings) {
                a.1 += b.1;
            }
        }
    }
    let sum: Duration = total.iter().map(|t| t.1).sum();
    for (name, d) in &total {
        println!("{name:>8}: {:>7.2} ms", d.as_secs_f64() * 1000.0 / N as f64);
    }
    println!(
        "{:>8}: {:>7.2} ms per refresh",
        "total",
        sum.as_secs_f64() * 1000.0 / N as f64
    );
}

fn main() -> anyhow::Result<()> {
    let (interval, do_bench) = parse_args();
    if do_bench {
        bench();
        return Ok(());
    }
    let mut app = App::new(interval);
    let mut terminal = ratatui::init();
    let res = run(&mut terminal, &mut app);
    ratatui::restore();
    res
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    let mut next_tick = Instant::now() + app.interval;
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        let timeout = next_tick.saturating_duration_since(Instant::now());
        if event::poll(timeout)?
            && let Event::Key(k) = event::read()?
        {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            if app.pending_kill.is_some() {
                match k.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => app.confirm_kill(),
                    _ => app.pending_kill = None,
                }
                continue;
            }
            app.status = None;
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                KeyCode::Char('s') => {
                    app.sort = app.sort.next();
                    app.selected_pid = None;
                    app.table.select(Some(0));
                    app.sort_procs();
                }
                KeyCode::Char('r') => {
                    app.reverse = !app.reverse;
                    app.selected_pid = None;
                    app.table.select(Some(0));
                    app.sort_procs();
                }
                KeyCode::Char('c') => {
                    app.sort = app::SortKey::Cpu;
                    app.selected_pid = None;
                    app.table.select(Some(0));
                    app.sort_procs();
                }
                KeyCode::Char('m') => {
                    app.sort = app::SortKey::Mem;
                    app.selected_pid = None;
                    app.table.select(Some(0));
                    app.sort_procs();
                }
                KeyCode::Char('g') => {
                    app.sort = app::SortKey::Gpu;
                    app.selected_pid = None;
                    app.table.select(Some(0));
                    app.sort_procs();
                }
                KeyCode::Char('p') => {
                    app.sort = app::SortKey::Pid;
                    app.selected_pid = None;
                    app.table.select(Some(0));
                    app.sort_procs();
                }
                KeyCode::Char('n') => {
                    app.sort = app::SortKey::Name;
                    app.selected_pid = None;
                    app.table.select(Some(0));
                    app.sort_procs();
                }
                KeyCode::Char('k') => app.request_kill(libc::SIGTERM),
                KeyCode::Char('K') => app.request_kill(libc::SIGKILL),
                KeyCode::Char(' ') => app.paused = !app.paused,
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    app.interval =
                        (app.interval + Duration::from_millis(250)).min(Duration::from_secs(10))
                }
                KeyCode::Char('-') => {
                    app.interval = app
                        .interval
                        .saturating_sub(Duration::from_millis(250))
                        .max(Duration::from_millis(250))
                }
                KeyCode::Char(ch @ '1'..='7') => {
                    let i = ch as usize - '1' as usize;
                    app.show[i] = !app.show[i];
                }
                KeyCode::Up => app.move_selection(-1),
                KeyCode::Down => app.move_selection(1),
                KeyCode::PageUp => app.move_selection(-20),
                KeyCode::PageDown => app.move_selection(20),
                KeyCode::Home => app.move_selection(isize::MIN / 2),
                KeyCode::End => app.move_selection(isize::MAX / 2),
                _ => {}
            }
        }
        if Instant::now() >= next_tick {
            if !app.paused {
                app.update();
            }
            next_tick = Instant::now() + app.interval;
        }
    }
}
