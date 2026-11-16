use std::{fs, path::Path};

pub fn read_str(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

pub fn read_num<T: std::str::FromStr>(p: impl AsRef<Path>) -> Option<T> {
    read_str(p)?.parse().ok()
}

/// Human-readable binary size: 512B, 1.5K, 23M, 7.8G …
pub fn fmt_bytes(b: f64) -> String {
    const U: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut v = b.max(0.0);
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0}{}", U[i])
    } else if v < 10.0 {
        format!("{v:.1}{}", U[i])
    } else {
        format!("{v:.0}{}", U[i])
    }
}

pub fn fmt_rate(b: f64) -> String {
    format!("{}/s", fmt_bytes(b))
}

pub fn fmt_uptime(secs: f64) -> String {
    let s = secs as u64;
    let (d, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Sorted list of (index, path) for files like `temp3_input` in a hwmon dir.
pub fn indexed_files(dir: &Path, prefix: &str, suffix: &str) -> Vec<(u32, std::path::PathBuf)> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let idx = name
                .strip_prefix(prefix)?
                .strip_suffix(suffix)?
                .parse()
                .ok()?;
            Some((idx, e.path()))
        })
        .collect();
    v.sort_by_key(|(i, _)| *i);
    v
}
