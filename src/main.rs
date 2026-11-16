mod app;
mod config;
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
use config::Settings;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::time::{Duration, Instant};

struct Args {
    interval_ms: Option<u64>,
    bench: bool,
    reset: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        interval_ms: None,
        bench: false,
        reset: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-i" | "--interval" => match args.next().and_then(|v| v.parse().ok()) {
                Some(ms) => a.interval_ms = Some(ms),
                None => {
                    eprintln!("{arg} needs a number of milliseconds");
                    std::process::exit(2);
                }
            },
            "--bench" => a.bench = true,
            "--reset" => a.reset = true,
            "-h" | "--help" => {
                println!("mytop — CPU / memory / GPU / network / disk / sensor monitor\n");
                println!(
                    "usage: mytop [-i MS] [--reset]\n\n  -i, --interval MS   refresh interval in milliseconds (default 1000)\n      --reset         start from the default view instead of the remembered one\n      --bench         time each collector and exit"
                );
                let file = config::path().map_or("~/.config/mytop/mytop.conf".into(), |p| {
                    p.display().to_string()
                });
                println!(
                    "\nThe refresh interval, sort order and shown panels are remembered in\n{file}.\nPress D in mytop, or run mytop --reset, to go back to the defaults."
                );
                println!(
                    "\nRun as root to see per-process GPU usage of other users' processes (AMD/Intel)."
                );
                std::process::exit(0);
            }
            _ => {
                eprintln!("unknown argument: {arg} (try --help)");
                std::process::exit(2);
            }
        }
    }
    a
}

fn bench() {
    let mut app = App::new(Settings::default());
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
    let args = parse_args();
    if args.bench {
        bench();
        return Ok(());
    }
    let mut settings = if args.reset {
        Settings::default()
    } else {
        Settings::load()
    };
    if let Some(ms) = args.interval_ms {
        settings.interval = config::clamp_interval(ms);
    }
    let mut app = App::new(settings);
    // --reset and -i are remembered like any change made with the keys
    let save_err = settings.save().err();
    if let Some(e) = &save_err {
        app.status = Some(format!("can't save settings: {e}"));
    }
    let mut terminal = ratatui::init();
    let res = run(&mut terminal, &mut app, settings, save_err.is_none());
    ratatui::restore();
    res
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    mut saved: Settings,
    mut can_save: bool,
) -> anyhow::Result<()> {
    let mut next_tick = Instant::now() + app.interval;
    loop {
        // Remember the view as soon as it changes, so it survives the terminal being closed too.
        let now = app.settings();
        if now != saved {
            saved = now;
            if can_save && let Err(e) = now.save() {
                app.status = Some(format!("can't save settings: {e}"));
                can_save = false; // say it once, not on every change
            }
        }
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
                KeyCode::Char('D') => {
                    app.apply(Settings::default());
                    next_tick = Instant::now();
                    app.status = Some("view reset to defaults".into());
                }
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
