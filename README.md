# mytop

A terminal system monitor (Linux) with braille graphs for CPU, memory, GPU,
network, disk I/O and hardware sensors, plus a process table with per-process
GPU usage.

```
cargo build --release
./target/release/mytop            # 1s refresh
./target/release/mytop -i 500     # 500ms refresh
./target/release/mytop --bench    # time each collector
sudo ./target/release/mytop       # see GPU usage of every user's processes
```

## Keys

| key | action |
|---|---|
| `q` / `Esc` / `Ctrl-C` | quit |
| `s` / `r` | cycle sort column / reverse |
| `c` `m` `g` `p` `n` | sort by cpu / mem / gpu / pid / name |
| `↑` `↓` `PgUp` `PgDn` `Home` `End` | move selection |
| `k` / `K` | SIGTERM / SIGKILL selected process (asks y/n) |
| `+` / `-` | refresh interval ±250ms |
| `space` | pause |
| `1`–`7` | toggle panels: cpu mem gpu net disk sensors procs |

## Where the data comes from

| panel | source |
|---|---|
| CPU | `/proc/stat`, `/proc/loadavg`, `cpufreq/scaling_cur_freq` |
| Memory | `/proc/meminfo` |
| Network | `/proc/net/dev` (totals count physical NICs only) |
| Disk | `/proc/diskstats` (whole hardware disks only) |
| Sensors | every chip in `/sys/class/hwmon` + NVIDIA via NVML |
| Processes | `/proc/<pid>/stat`, `cmdline` |

### GPUs

| vendor | device stats | per-process / per-engine |
|---|---|---|
| NVIDIA | NVML (`libnvidia-ml.so`, loaded at runtime — no build dependency) | NVML process lists + `process_utilization_stats` |
| Intel i915 / xe | hwmon (temp, fan, energy→power), `rps_act_freq_mhz` / `act_freq`, RC6/gt-idle residency ("awake"), `DRM_IOCTL_I915_QUERY` for VRAM | DRM fdinfo |
| AMD amdgpu | `gpu_busy_percent`, `mem_info_vram_*`, hwmon (temp/power/clock/fan), `pp_dpm_*` | DRM fdinfo |

Notes:

* **AMD support is untested** (no hardware available). It is written against the
  documented amdgpu sysfs interface; please report anything that looks off.
* **xe driver** (Lunar Lake, Battlemage) paths are implemented but untested;
  VRAM total for xe is not read yet (needs `DRM_IOCTL_XE_DEVICE_QUERY`).
* fdinfo is only readable for your own processes unless run as root, so Intel/AMD
  per-process numbers show "own procs only" when not root.
* Intel overall utilisation = busiest engine (as nvtop does); "awake" is the
  device-wide non-RC6 time, which does see every process.
* Motherboard temps/fans (CPU fan, chassis fans, VRM) require a Super-I/O driver
  such as `nct6775` or `it87` — run `sensors-detect` / `modprobe nct6775` if
  the sensors panel only shows CPU/NVMe/GPU.
