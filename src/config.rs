//! The view settings mytop remembers between runs: refresh interval, sort column and order, and which panels
//! are shown. Stored as `key = value` lines in `$XDG_CONFIG_HOME/mytop/mytop.conf` (default
//! `~/.config/mytop/mytop.conf`). A missing file, unknown key or bad value falls back to the default.

use crate::app::{PANELS, SortKey};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};

pub const MIN_INTERVAL_MS: u64 = 100;
pub const MAX_INTERVAL_MS: u64 = 10_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    pub interval: Duration,
    pub sort: SortKey,
    pub reverse: bool,
    pub show: [bool; 7],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(1000),
            sort: SortKey::Cpu,
            reverse: false,
            show: [true; 7],
        }
    }
}

pub fn clamp_interval(ms: u64) -> Duration {
    Duration::from_millis(ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS))
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mytop").join("mytop.conf"))
}

impl Settings {
    pub fn load() -> Self {
        path()
            .and_then(|p| fs::read_to_string(p).ok())
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "interval_ms" => {
                    if let Ok(ms) = v.parse() {
                        s.interval = clamp_interval(ms);
                    }
                }
                "sort" => {
                    if let Some(k) = SortKey::from_label(v) {
                        s.sort = k;
                    }
                }
                "reverse" => {
                    if let Ok(b) = v.parse() {
                        s.reverse = b;
                    }
                }
                // panels are listed by name, so the file survives panels being added or reordered
                "hidden" => {
                    s.show = [true; 7];
                    for name in v.split(',').map(str::trim) {
                        if let Some(i) = PANELS.iter().position(|p| *p == name) {
                            s.show[i] = false;
                        }
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn render(&self) -> String {
        let hidden: Vec<&str> = PANELS
            .iter()
            .zip(self.show)
            .filter(|(_, shown)| !shown)
            .map(|(p, _)| *p)
            .collect();
        format!(
            "# mytop view settings, saved automatically. Press D in mytop or run `mytop --reset` for the defaults.\n\
             interval_ms = {}\nsort = {}\nreverse = {}\nhidden = {}\n",
            self.interval.as_millis(),
            self.sort.label(),
            self.reverse,
            hidden.join(","),
        )
    }

    pub fn save(&self) -> io::Result<()> {
        let p =
            path().ok_or_else(|| io::Error::other("neither XDG_CONFIG_HOME nor HOME is set"))?;
        let dir = p.parent().expect("config path has a parent");
        let new_dir = !dir.exists();
        fs::create_dir_all(dir)?;
        let tmp = p.with_extension("conf.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(self.render().as_bytes())?;
        }
        fs::rename(&tmp, &p)?;
        // Under `sudo -E` the config lands in the invoking user's home; keep it theirs so they can still save.
        if unsafe { libc::geteuid() } == 0 {
            if new_dir {
                own_like_parent(dir);
            }
            own_like_parent(&p);
        }
        Ok(())
    }
}

fn own_like_parent(p: &Path) {
    if let Some(m) = p.parent().and_then(|d| fs::metadata(d).ok()) {
        let _ = std::os::unix::fs::chown(p, Some(m.uid()), Some(m.gid()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            interval: Duration::from_millis(2500),
            sort: SortKey::Gpu,
            reverse: true,
            show: [true, false, true, true, false, true, true],
        };
        assert_eq!(Settings::parse(&s.render()), s);
        assert_eq!(
            Settings::parse(&Settings::default().render()),
            Settings::default()
        );
    }

    #[test]
    fn bad_values_fall_back() {
        let s = Settings::parse(
            "interval_ms = 5\nsort = bogus\nreverse = maybe\nhidden = nope, disk\njunk\n",
        );
        assert_eq!(s.interval, Duration::from_millis(MIN_INTERVAL_MS));
        assert_eq!(s.sort, SortKey::Cpu);
        assert!(!s.reverse);
        assert_eq!(s.show, [true, true, true, true, false, true, true]);
        assert_eq!(Settings::parse(""), Settings::default());
    }
}
