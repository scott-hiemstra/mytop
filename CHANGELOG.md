# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.2.1] - 2026-11-16

### Fixed
- Intel GPU % no longer reads near zero when the load comes from another
  user's processes (e.g. a root-owned inference server). mytop now reads the
  i915 PMU engine counters when perf events are allowed, and otherwise
  falls back to the GT awake time.

## [0.2.0] - 2026-11-16

### Added
- The view is remembered between runs: refresh interval, sort column and
  order, and shown panels are saved to `~/.config/mytop/mytop.conf`
  (`$XDG_CONFIG_HOME` is honoured) whenever they change.
- `D` resets the view to the defaults; `--reset` starts from them.

### Changed
- `-i` without a number is now an error instead of being ignored.

## [0.1.0] - 2026-11-16

First release.

### Added
- CPU panel: total-usage history graph, per-core bars, model, average clock,
  CPU temperature, load average and uptime.
- Memory panel: RAM and swap bars, history graph, installed vs usable RAM.
- GPU panels, one per device:
  - NVIDIA via NVML (utilisation, VRAM, temperature, power/limit, clocks,
    fan, encoder/decoder, per-process usage).
  - Intel i915/xe via sysfs, hwmon, RC6/gt-idle residency, the i915 query
    ioctl (VRAM) and DRM fdinfo (per-engine and per-process usage).
  - AMD amdgpu via sysfs/hwmon and DRM fdinfo (untested on hardware).
  - Top two GPU processes listed in each GPU panel.
- Network panel: rx/tx graph and per-interface rates and totals.
- Disk panel: read/write graph and per-disk rates and busy %.
- Sensors panel: every hwmon temperature and fan, plus NVIDIA via NVML.
- Process table sortable by CPU, memory, GPU, PID or name, with per-process
  GPU % and VRAM, and SIGTERM/SIGKILL with confirmation.
- Panel toggles (`1`–`7`), adjustable refresh interval, pause.
- `--bench` flag to time each collector.

[0.2.1]: https://github.com/scott-hiemstra/mytop/releases/tag/v0.2.1
[0.2.0]: https://github.com/scott-hiemstra/mytop/releases/tag/v0.2.0
[0.1.0]: https://github.com/scott-hiemstra/mytop/releases/tag/v0.1.0
