use crate::{
    app::{App, PANELS, SortKey},
    gpu::{Gpu, Vendor},
    history::History,
    util::*,
};
use ratatui::{
    prelude::*,
    symbols::Marker,
    widgets::{Axis, Block, BorderType, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table},
};

// ── palette ────────────────────────────────────────────────────────────────
const BORDER: Color = Color::Rgb(88, 91, 112);
const DIM: Color = Color::Rgb(127, 132, 156);
const TRACK: Color = Color::Rgb(55, 57, 72);
const C_CPU: Color = Color::Rgb(137, 180, 250);
const C_MEM: Color = Color::Rgb(249, 226, 175);
const C_NET_RX: Color = Color::Rgb(166, 227, 161);
const C_NET_TX: Color = Color::Rgb(245, 194, 231);
const C_DISK_R: Color = Color::Rgb(148, 226, 213);
const C_DISK_W: Color = Color::Rgb(250, 179, 135);
const C_SENS: Color = Color::Rgb(243, 139, 168);
const C_PROC: Color = Color::Rgb(203, 166, 247);
const C_NV: Color = Color::Rgb(118, 185, 0);
const C_AMD: Color = Color::Rgb(237, 28, 36);
const C_INTEL: Color = Color::Rgb(0, 174, 239);
const C_VRAM: Color = Color::Rgb(180, 190, 254);

/// green → yellow → red for 0..100
fn grad(p: f64) -> Color {
    let p = (p / 100.0).clamp(0.0, 1.0);
    let (a, b, t) = if p < 0.5 {
        ((80.0, 200.0, 120.0), (235.0, 200.0, 70.0), p * 2.0)
    } else {
        ((235.0, 200.0, 70.0), (235.0, 75.0, 75.0), (p - 0.5) * 2.0)
    };
    let l = |x: f64, y: f64| (x + (y - x) * t) as u8;
    Color::Rgb(l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}

fn temp_color(t: f64) -> Color {
    grad((t - 35.0) / 55.0 * 100.0)
}

fn bar(width: usize, ratio: f64, color: Color) -> Vec<Span<'static>> {
    if width == 0 {
        return vec![];
    }
    const PART: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
    let cells = ratio.clamp(0.0, 1.0) * width as f64;
    let full = cells.floor() as usize;
    let mut s = "█".repeat(full);
    let mut used = full;
    if full < width {
        let idx = ((cells - full as f64) * 8.0) as usize;
        if idx > 0 {
            s.push_str(PART[idx]);
            used += 1;
        }
    }
    vec![
        Span::styled(s, Style::new().fg(color).bg(TRACK)),
        Span::styled(" ".repeat(width - used), Style::new().bg(TRACK)),
    ]
}

fn panel<'a>(title: Vec<Span<'a>>, accent: Color, key: usize) -> Block<'a> {
    let mut t = vec![
        Span::styled(format!("{}", key + 1), Style::new().fg(DIM)),
        Span::raw(" "),
    ];
    t.extend(title);
    t.push(Span::raw(" "));
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(Line::from(t))
        .title_style(Style::new().fg(accent))
}

fn bold(s: impl Into<String>, c: Color) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(c).add_modifier(Modifier::BOLD))
}
fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(DIM))
}
fn col(s: impl Into<String>, c: Color) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(c))
}

/// Braille history chart. Returns the y-axis max used.
fn chart(
    f: &mut Frame,
    area: Rect,
    series: &[(&History, Color, GraphType)],
    ymax: Option<f64>,
) -> f64 {
    if area.width < 2 || area.height < 1 {
        return 0.0;
    }
    let n = area.width as usize * 2;
    let data: Vec<Vec<(f64, f64)>> = series.iter().map(|(h, _, _)| h.points(n)).collect();
    let ymax = ymax.unwrap_or_else(|| {
        let m = data.iter().flatten().map(|p| p.1).fold(0.0, f64::max);
        (m * 1.15).max(1024.0)
    });
    let ds = series
        .iter()
        .zip(&data)
        .map(|((_, c, g), d)| {
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(*g)
                .style(Style::new().fg(*c))
                .data(d)
        })
        .collect();
    let ch = Chart::new(ds)
        .x_axis(Axis::default().bounds([0.0, (n - 1) as f64]))
        .y_axis(Axis::default().bounds([0.0, ymax]))
        .legend_position(None);
    f.render_widget(ch, area);
    ymax
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [main, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    let s = app.show;
    let ncores = app.cpu.cores.len();
    let top_h = ((ncores.div_ceil(4) + 2) as u16).clamp(9, 14);
    let has_gpu = s[2] && (!app.gpus.list.is_empty() || app.gpus.nvml_error.is_some());

    // (constraint, kind) for each visible row
    let mut rows: Vec<(u16, u8)> = vec![];
    if s[0] || s[1] {
        rows.push((top_h, 0));
    }
    if has_gpu {
        rows.push((10, 1));
    }
    if s[3] || s[4] || s[5] {
        rows.push((10, 2));
    }
    if s[6] {
        rows.push((0, 3));
    }
    let constraints: Vec<Constraint> = rows
        .iter()
        .map(|(h, k)| {
            if *k == 3 || !s[6] {
                Constraint::Fill(1)
            } else {
                Constraint::Length(*h)
            }
        })
        .collect();
    let areas = Layout::vertical(constraints).split(main);

    for ((_, kind), area) in rows.iter().zip(areas.iter()) {
        match kind {
            0 => {
                let parts: Vec<(usize, Constraint)> =
                    [(0, Constraint::Fill(65)), (1, Constraint::Fill(35))]
                        .into_iter()
                        .filter(|(i, _)| s[*i])
                        .collect();
                let a = Layout::horizontal(parts.iter().map(|p| p.1)).split(*area);
                for ((i, _), r) in parts.iter().zip(a.iter()) {
                    if *i == 0 {
                        draw_cpu(f, *r, app)
                    } else {
                        draw_mem(f, *r, app)
                    }
                }
            }
            1 => draw_gpus(f, *area, app),
            2 => {
                let parts: Vec<usize> = [3, 4, 5].into_iter().filter(|i| s[*i]).collect();
                let a = Layout::horizontal(parts.iter().map(|_| Constraint::Fill(1))).split(*area);
                for (i, r) in parts.iter().zip(a.iter()) {
                    match i {
                        3 => draw_net(f, *r, app),
                        4 => draw_disk(f, *r, app),
                        _ => draw_sensors(f, *r, app),
                    }
                }
            }
            _ => draw_procs(f, *area, app),
        }
    }
    draw_footer(f, footer, app);
}

fn draw_cpu(f: &mut Frame, area: Rect, app: &App) {
    let c = &app.cpu;
    let mut title = vec![
        bold("cpu ", C_CPU),
        col(format!("{:.0}% ", c.total), grad(c.total)),
        dim(c.model.clone()),
    ];
    if let Some(mhz) = c.freq_mhz {
        title.push(dim(format!(" · {:.2} GHz", mhz / 1000.0)));
    }
    if let Some(t) = app.sensors.cpu_temp {
        title.push(dim(" · "));
        title.push(col(format!("{t:.0}°C"), temp_color(t)));
    }
    let block = panel(title, C_CPU, 0).title_bottom(
        Line::from(dim(format!(
            " load {:.2} {:.2} {:.2} · up {} ",
            c.load[0],
            c.load[1],
            c.load[2],
            fmt_uptime(c.uptime)
        )))
        .right_aligned(),
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 {
        return;
    }

    let n = c.cores.len();
    let cols = n.div_ceil(inner.height as usize).max(1);
    let rows = n.div_ceil(cols).max(1);
    let avail = (inner.width as usize * 45 / 100).max(1);
    let cellw = (avail / cols).clamp(12, 20);
    let barw = cellw - 11;
    let grid_w = (cols * cellw).min(inner.width as usize - 1) as u16;
    let [chart_area, grid] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(grid_w)]).areas(inner);

    chart(
        f,
        chart_area,
        &[(&c.hist, C_CPU, GraphType::Bar)],
        Some(100.0),
    );

    let lines: Vec<Line> = (0..rows)
        .map(|r| {
            let mut spans = vec![Span::raw(" ")];
            for cl in 0..cols {
                let i = cl * rows + r;
                if let Some(p) = c.cores.get(i) {
                    spans.push(dim(format!("{i:>3} ")));
                    spans.extend(bar(barw, p / 100.0, grad(*p)));
                    spans.push(col(format!("{p:>4.0}%"), grad(*p)));
                    spans.push(Span::raw(" "));
                }
            }
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), grid);
}

fn draw_mem(f: &mut Frame, area: Rect, app: &App) {
    let m = &app.mem;
    let pct = |u: u64, t: u64| if t > 0 { u as f64 / t as f64 } else { 0.0 };
    let block = panel(
        match m.installed {
            Some(i) => vec![
                bold("mem ", C_MEM),
                Span::raw(format!("{} installed", fmt_bytes(i as f64))),
                dim(format!(" · {} usable", fmt_bytes(m.total as f64))),
            ],
            None => vec![
                bold("mem ", C_MEM),
                dim(format!("{} total", fmt_bytes(m.total as f64))),
            ],
        },
        C_MEM,
        1,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;
    let label_w = 5;
    let val_w = 13;
    let bw = w.saturating_sub(label_w + val_w + 2);
    let gauge = |name: &str, u: u64, t: u64, c: Color| {
        let mut s = vec![dim(format!("{name:<label_w$}"))];
        s.extend(bar(bw, pct(u, t), c));
        s.push(Span::raw(format!(
            " {:>val_w$}",
            format!("{}/{}", fmt_bytes(u as f64), fmt_bytes(t as f64))
        )));
        Line::from(s)
    };
    let ram_p = pct(m.used, m.total) * 100.0;
    let lines = vec![
        gauge("ram", m.used, m.total, grad(ram_p)),
        gauge(
            "swap",
            m.swap_used,
            m.swap_total,
            grad(pct(m.swap_used, m.swap_total) * 100.0),
        ),
        Line::from(vec![
            dim("avail "),
            Span::raw(fmt_bytes(m.avail as f64)),
            dim("  cache "),
            Span::raw(fmt_bytes(m.cached as f64)),
            dim("  buf "),
            Span::raw(fmt_bytes(m.buffers as f64)),
        ]),
    ];
    let [text, ch] = Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(inner);
    f.render_widget(Paragraph::new(lines), text);
    chart(f, ch, &[(&m.hist, C_MEM, GraphType::Bar)], Some(100.0));
}

fn draw_gpus(f: &mut Frame, area: Rect, app: &App) {
    if app.gpus.list.is_empty() {
        let block = panel(vec![bold("gpu", C_NV)], C_NV, 2);
        let msg = app
            .gpus
            .nvml_error
            .clone()
            .unwrap_or_else(|| "no supported GPU found".into());
        f.render_widget(Paragraph::new(dim(msg)).block(block), area);
        return;
    }
    let n = app.gpus.list.len();
    let areas = Layout::horizontal((0..n).map(|_| Constraint::Fill(1))).split(area);
    for (i, (g, a)) in app.gpus.list.iter().zip(areas.iter()).enumerate() {
        draw_gpu(f, *a, i, g, app.is_root);
    }
}

fn draw_gpu(f: &mut Frame, area: Rect, idx: usize, g: &Gpu, is_root: bool) {
    let accent = match g.vendor {
        Vendor::Nvidia => C_NV,
        Vendor::Amd => C_AMD,
        Vendor::Intel => C_INTEL,
    };
    let s = &g.s;
    let util = s.util.unwrap_or(0.0);
    let mut bottom = format!(" {} · {} ", g.driver, g.pci);
    if g.vendor != Vendor::Nvidia && !is_root {
        bottom = format!(" own procs only · {} · {} ", g.driver, g.pci);
    }
    let block = panel(
        vec![
            bold(format!("gpu{idx} "), accent),
            col(format!("{util:.0}% "), grad(util)),
            Span::raw(g.name.clone()),
        ],
        accent,
        2,
    )
    .title_bottom(Line::from(dim(bottom)).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if let Some(e) = &s.error {
        f.render_widget(Paragraph::new(col(e.clone(), Color::Red)), inner);
        return;
    }

    let stats_w = (inner.width / 2).clamp(24, 40);
    let [ch, stats] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(stats_w)]).areas(inner);
    let top_h = if ch.height >= 6 { 2 } else { 0 };
    let [ch, top_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(top_h)]).areas(ch);
    draw_gpu_top(f, top_area, g, is_root);
    chart(
        f,
        ch,
        &[
            (&g.util_hist, accent, GraphType::Bar),
            (&g.mem_hist, C_VRAM, GraphType::Line),
        ],
        Some(100.0),
    );

    let w = stats_w as usize;
    let bw = w.saturating_sub(5 + 12);
    let mut lines = vec![];
    let mut l = vec![dim(" gpu ")];
    l.extend(bar(bw, util / 100.0, grad(util)));
    l.push(col(format!("{:>11}", format!("{util:.0}%")), grad(util)));
    lines.push(Line::from(l));

    let mut l = vec![dim(" mem ")];
    match (s.mem_used, s.mem_total) {
        (Some(u), Some(t)) => {
            l.extend(bar(bw, u as f64 / t as f64, C_VRAM));
            l.push(Span::raw(format!(
                "{:>11}",
                format!("{}/{}", fmt_bytes(u as f64), fmt_bytes(t as f64))
            )));
        }
        (Some(u), None) => l.push(Span::raw(format!("{} used", fmt_bytes(u as f64)))),
        (None, Some(t)) => l.push(Span::raw(format!("?/{}", fmt_bytes(t as f64)))),
        _ => l.push(dim("n/a")),
    }
    lines.push(Line::from(l));

    let mut l = vec![Span::raw(" ")];
    if let Some(t) = s.temp {
        l.push(col(format!("{t:.0}°C  "), temp_color(t)));
    }
    if let Some(p) = s.power {
        l.push(Span::raw(format!("{p:.0}W")));
        if let Some(lim) = s.power_limit {
            l.push(dim(format!("/{lim:.0}W")));
        }
        l.push(Span::raw("  "));
    }
    if let Some(r) = s.fan_rpm {
        l.push(dim("fan "));
        l.push(Span::raw(format!("{r}rpm")));
    } else if let Some(p) = s.fan_pct {
        l.push(dim("fan "));
        l.push(Span::raw(format!("{p}%")));
    }
    lines.push(Line::from(l));

    let mut l = vec![Span::raw(" ")];
    if let Some(c) = s.clock {
        l.push(dim("clk "));
        l.push(Span::raw(format!("{c}MHz  ")));
    }
    if let Some(c) = s.mem_clock {
        l.push(dim("mclk "));
        l.push(Span::raw(format!("{c}MHz")));
    }
    lines.push(Line::from(l));

    let mut l = vec![Span::raw(" ")];
    if let Some(e) = s.enc {
        l.push(dim("enc "));
        l.push(Span::raw(format!("{e:.0}%  ")));
    }
    if let Some(d) = s.dec {
        l.push(dim("dec "));
        l.push(Span::raw(format!("{d:.0}%  ")));
    }
    for (k, v) in &s.extra {
        l.push(dim(format!("{k} ")));
        l.push(Span::raw(format!("{v}  ")));
    }
    lines.push(Line::from(l));

    // per-engine mini bars (fdinfo), two per line
    let ew = w / 2;
    let ebw = ew.saturating_sub(8 + 5);
    for pair in s.engines.chunks(2) {
        let mut l = vec![];
        for (name, v) in pair {
            let short: String = name
                .replace("video-enhance", "vecs")
                .chars()
                .take(7)
                .collect();
            l.push(dim(format!(" {short:<7}")));
            l.extend(bar(ebw, v / 100.0, grad(*v)));
            l.push(Span::raw(format!("{v:>4.0}%")));
        }
        lines.push(Line::from(l));
    }
    f.render_widget(Paragraph::new(lines), stats);
}

fn draw_gpu_top(f: &mut Frame, area: Rect, g: &Gpu, is_root: bool) {
    if area.height == 0 {
        return;
    }
    // name, gpu%, vram, then pid last so narrow panels only lose the pid
    let name_w = (area.width as usize).saturating_sub(3 + 5 + 7).clamp(6, 16);
    let mut lines: Vec<Line> = g
        .top
        .iter()
        .map(|p| {
            let vram = if p.vram > 0 {
                fmt_bytes(p.vram as f64)
            } else {
                String::new()
            };
            Line::from(vec![
                dim(" ▸ "),
                Span::styled(
                    format!("{:<name_w$.name_w$}", p.name),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                col(format!("{:>4.0}%", p.util), grad(p.util)),
                col(format!("{vram:>7}"), C_VRAM),
                dim(format!("  pid {}", p.pid)),
            ])
        })
        .collect();
    if lines.is_empty() {
        let msg = if g.vendor != Vendor::Nvidia && !is_root {
            " no visible GPU processes (run as root to see all)"
        } else {
            " no GPU processes"
        };
        lines.push(Line::from(dim(msg)));
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn io_panel(
    f: &mut Frame,
    area: Rect,
    title: Vec<Span<'static>>,
    accent: Color,
    key: usize,
    series: [(&History, Color); 2],
    lines: Vec<Line<'static>>,
) {
    let block = panel(title, accent, key);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let text_h = (lines.len() as u16).min(inner.height.saturating_sub(3));
    let [ch, txt] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(text_h)]).areas(inner);
    let ymax = chart(
        f,
        ch,
        &[
            (series[0].0, series[0].1, GraphType::Line),
            (series[1].0, series[1].1, GraphType::Line),
        ],
        None,
    );
    // scale label in the top-right corner of the chart
    let lbl = format!("{} ", fmt_rate(ymax));
    if ch.width as usize > lbl.len() + 2 && ch.height > 0 {
        let r = Rect {
            x: ch.right() - lbl.len() as u16,
            y: ch.y,
            width: lbl.len() as u16,
            height: 1,
        };
        f.render_widget(Paragraph::new(dim(lbl)), r);
    }
    f.render_widget(Paragraph::new(lines), txt);
}

fn draw_net(f: &mut Frame, area: Rect, app: &App) {
    let n = &app.net;
    let title = vec![
        bold("net ", C_NET_RX),
        col(format!("↓{} ", fmt_rate(n.rx)), C_NET_RX),
        col(format!("↑{}", fmt_rate(n.tx)), C_NET_TX),
    ];
    let lines = n
        .ifaces
        .iter()
        .filter(|i| i.physical || i.rx_rate + i.tx_rate > 0.0)
        .take(4)
        .map(|i| {
            Line::from(vec![
                Span::raw(format!(" {:<10.10} ", i.name)),
                col(format!("↓{:>8}", fmt_rate(i.rx_rate)), C_NET_RX),
                col(format!(" ↑{:>8}", fmt_rate(i.tx_rate)), C_NET_TX),
                dim(format!(
                    "  Σ {}/{}",
                    fmt_bytes(i.rx_total as f64),
                    fmt_bytes(i.tx_total as f64)
                )),
            ])
        })
        .collect();
    io_panel(
        f,
        area,
        title,
        C_NET_RX,
        3,
        [(&n.rx_hist, C_NET_RX), (&n.tx_hist, C_NET_TX)],
        lines,
    );
}

fn draw_disk(f: &mut Frame, area: Rect, app: &App) {
    let d = &app.disk;
    let title = vec![
        bold("disk ", C_DISK_R),
        col(format!("R {} ", fmt_rate(d.read)), C_DISK_R),
        col(format!("W {}", fmt_rate(d.write)), C_DISK_W),
    ];
    let lines = d
        .devs
        .iter()
        .take(4)
        .map(|x| {
            Line::from(vec![
                Span::raw(format!(" {:<9.9}", x.name)),
                col(format!("R{:>8}", fmt_rate(x.read_rate)), C_DISK_R),
                col(format!(" W{:>8}", fmt_rate(x.write_rate)), C_DISK_W),
                dim("  busy "),
                col(format!("{:.0}%", x.busy), grad(x.busy)),
            ])
        })
        .collect();
    io_panel(
        f,
        area,
        title,
        C_DISK_R,
        4,
        [(&d.read_hist, C_DISK_R), (&d.write_hist, C_DISK_W)],
        lines,
    );
}

fn draw_sensors(f: &mut Frame, area: Rect, app: &App) {
    let block = panel(vec![bold("sensors", C_SENS)], C_SENS, 5);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;
    let bw = w.saturating_sub(10 + 11 + 9).min(30);
    let mut lines = vec![];
    for chip in &app.sensors.chips {
        for (label, t) in &chip.temps {
            let mut l = vec![
                dim(format!(" {:<9.9}", chip.name)),
                Span::raw(format!("{label:<10.10} ")),
            ];
            l.extend(bar(bw, (t - 20.0) / 80.0, temp_color(*t)));
            l.push(col(format!("{:>6.1}°C", t), temp_color(*t)));
            lines.push(Line::from(l));
        }
        for (label, rpm) in &chip.fans {
            lines.push(Line::from(vec![
                dim(format!(" {:<9.9}", chip.name)),
                Span::raw(format!("{label:<10.10} ")),
                Span::raw(format!("{:>w$}", format!("{rpm} rpm"), w = bw + 8)),
            ]));
        }
    }
    // NVIDIA isn't in hwmon — add it from NVML
    for (i, g) in app
        .gpus
        .list
        .iter()
        .enumerate()
        .filter(|(_, g)| g.vendor == Vendor::Nvidia)
    {
        if let Some(t) = g.s.temp {
            let mut l = vec![
                dim(format!(" {:<9.9}", format!("nvidia{i}"))),
                Span::raw(format!("{:<10.10} ", "gpu")),
            ];
            l.extend(bar(bw, (t - 20.0) / 80.0, temp_color(t)));
            l.push(col(format!("{:>6.1}°C", t), temp_color(t)));
            lines.push(Line::from(l));
        }
        if let Some(p) = g.s.fan_pct {
            lines.push(Line::from(vec![
                dim(format!(" {:<9.9}", format!("nvidia{i}"))),
                Span::raw(format!("{:<10.10} ", "fan")),
                Span::raw(format!("{:>w$}", format!("{p} %"), w = bw + 8)),
            ]));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(dim(" no hwmon sensors found")));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_procs(f: &mut Frame, area: Rect, app: &mut App) {
    let arrow = if app.reverse { "▲" } else { "▼" };
    let block = panel(
        vec![
            bold("processes ", C_PROC),
            dim(format!("{} · sort ", app.procs.list.len())),
            Span::raw(format!("{} {arrow}", app.sort.label())),
        ],
        C_PROC,
        6,
    );
    let hdr = |name: &str, key: Option<SortKey>| {
        let s = if key == Some(app.sort) {
            format!("{name}{arrow}")
        } else {
            name.to_string()
        };
        let style = if key == Some(app.sort) {
            Style::new().fg(C_PROC).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(DIM)
        };
        Cell::from(s).style(style)
    };
    let header = Row::new(vec![
        hdr("PID", Some(SortKey::Pid)),
        hdr("USER", None),
        hdr("CPU%", Some(SortKey::Cpu)),
        hdr("MEM%", Some(SortKey::Mem)),
        hdr("RSS", None),
        hdr("GPU%", Some(SortKey::Gpu)),
        hdr("VRAM", None),
        hdr("THR", None),
        hdr("S", None),
        hdr("COMMAND", Some(SortKey::Name)),
    ]);

    // Only build rows near the viewport; the list can be thousands long.
    let visible = area.height.saturating_sub(3) as usize;
    let sel = app.table.selected().unwrap_or(0);
    let offset = app
        .table
        .offset()
        .min(sel)
        .max(sel.saturating_sub(visible.saturating_sub(1)));
    let end = (offset + visible).min(app.procs.list.len());
    let rows: Vec<Row> = app.procs.list[offset..end]
        .iter()
        .map(|p| {
            let gpu = p.gpu.map(|g| format!("{g:.0}")).unwrap_or_default();
            let vram = p
                .vram
                .filter(|v| *v > 0)
                .map(|v| fmt_bytes(v as f64))
                .unwrap_or_default();
            Row::new(vec![
                Cell::from(p.pid.to_string()),
                Cell::from(p.user.clone()).style(Style::new().fg(DIM)),
                Cell::from(format!("{:.1}", p.cpu)).style(Style::new().fg(grad(p.cpu.min(100.0)))),
                Cell::from(format!("{:.1}", p.mem)),
                Cell::from(fmt_bytes(p.rss as f64)),
                Cell::from(gpu).style(Style::new().fg(grad(p.gpu.unwrap_or(0.0)))),
                Cell::from(vram).style(Style::new().fg(C_VRAM)),
                Cell::from(p.threads.to_string()).style(Style::new().fg(DIM)),
                Cell::from(p.state.to_string()).style(Style::new().fg(DIM)),
                Cell::from(p.cmd.clone()),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(4),
        Constraint::Length(2),
        Constraint::Fill(1),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(
            Style::new()
                .bg(Color::Rgb(69, 71, 90))
                .add_modifier(Modifier::BOLD),
        );
    let mut st = ratatui::widgets::TableState::default().with_selected(Some(sel - offset));
    f.render_stateful_widget(table, area, &mut st);
    *app.table.offset_mut() = offset;
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let line = if let Some((pid, name, sig)) = &app.pending_kill {
        let s = if *sig == libc::SIGKILL {
            "SIGKILL"
        } else {
            "SIGTERM"
        };
        Line::from(vec![
            bold(format!(" send {s} to {pid} ({name})? "), Color::Red),
            Span::raw("y/n"),
        ])
    } else {
        let key = |k: &str, d: &str| vec![bold(format!(" {k}"), C_PROC), dim(format!(" {d} "))];
        let mut spans = vec![];
        spans.extend(key("q", "quit"));
        spans.extend(key("s", "sort"));
        spans.extend(key("r", "reverse"));
        spans.extend(key("↑↓", "select"));
        spans.extend(key("k/K", "term/kill"));
        spans.extend(key("+/-", &format!("{}ms", app.interval.as_millis())));
        spans.extend(key("space", if app.paused { "resume" } else { "pause" }));
        spans.push(dim(" │"));
        for (i, p) in PANELS.iter().enumerate() {
            // hidden panels are dimmed out
            let (name, key) = if app.show[i] {
                (
                    Style::new().fg(Color::White),
                    Style::new().fg(C_PROC).add_modifier(Modifier::BOLD),
                )
            } else {
                (Style::new().fg(TRACK), Style::new().fg(TRACK))
            };
            spans.push(Span::styled(format!(" {p} "), name));
            spans.push(Span::styled(format!("({})", i + 1), key));
        }
        if let Some(s) = &app.status {
            spans.push(dim("  │ "));
            spans.push(Span::raw(s.clone()));
        }
        Line::from(spans)
    };
    f.render_widget(Paragraph::new(line), area);
}
