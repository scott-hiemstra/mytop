# Architecture

mytop is a single-threaded Rust program built on
[ratatui](https://ratatui.rs) (rendering) and crossterm (terminal input). All
data comes straight from the Linux kernel's `/proc` and `/sys` interfaces,
plus NVML for NVIDIA GPUs. There is no background thread and no external
command is ever executed.

## Source layout

```
src/
├── main.rs        argument parsing, terminal setup, event loop, key handling, --bench
├── app.rs         App state: collectors, sort order, selection, panel toggles
├── ui.rs          all drawing: layout, panels, charts, bars, process table
├── history.rs     fixed-size ring buffer feeding the graphs
├── util.rs        small helpers: sysfs reads, byte/rate/uptime formatting
├── cpu.rs         /proc/stat, cpufreq, loadavg, uptime
├── mem.rs         /proc/meminfo, installed RAM from /sys/firmware/memmap
├── net.rs         /proc/net/dev
├── disk.rs        /proc/diskstats
├── sensors.rs     /sys/class/hwmon/*
├── procs.rs       /proc/<pid>/{stat,cmdline}
└── gpu/
    ├── mod.rs     GPU discovery, common sample model, per-GPU history, top processes
    ├── nvidia.rs  NVML backend
    ├── intel.rs   i915 / xe backend (sysfs, hwmon, RC6, i915 query ioctl)
    ├── amd.rs     amdgpu backend (sysfs, hwmon)
    └── fdinfo.rs  DRM fdinfo scanner: per-engine and per-process usage
```

## Main loop

```
App::new()           create collectors, take a baseline sample, sleep 250 ms, sample again
loop:
    draw()           render the current state (~0.7 ms)
    poll(timeout)    wait for a key until the next tick
    on tick:         App::update() → every collector refreshes, process list re-sorted
```

Every collector that reports a rate (CPU %, network, disk, per-process CPU,
GPU energy and engine time) stores the previous raw counter and divides the
difference by the measured elapsed time, so changing the interval or a late
tick never distorts the numbers.

`--bench` runs ten updates and draws into an off-screen `TestBackend`,
printing the average time per collector. Typical numbers on a 16-thread
desktop with two GPUs and ~550 processes:

```
     cpu:    0.33 ms     sensors:  4.4 ms   (NVMe temperature reads are slow)
     mem:    0.04 ms     gpu:     19   ms   (NVML calls + fdinfo scan)
     net:    0.29 ms     procs:    8.3 ms
    disk:    0.11 ms     draw:     0.7 ms
```

## How each metric is computed

### CPU (`cpu.rs`)
`/proc/stat` lines `cpu` and `cpuN`: busy = user + nice + system + irq +
softirq + steal; total = busy + idle + iowait. Usage = Δbusy / Δtotal. The
clock is the mean of `cpufreq/scaling_cur_freq` across CPUs. CPU temperature
is picked from the sensors collector (`k10temp` Tctl/Tdie, `zenpower`,
`coretemp` "Package id 0" or `cpu_thermal`).

### Memory (`mem.rs`)
`used = MemTotal − MemAvailable`. "Installed" sums the `System RAM` ranges
in `/sys/firmware/memmap/*` (the firmware/e820 map, readable by any user) and
rounds up to a whole GiB.

### Network (`net.rs`)
`/proc/net/dev` byte counters, excluding `lo`. An interface is *physical* if
`/sys/class/net/<if>/device` exists; header totals only count physical
interfaces, unless there are none.

### Disk (`disk.rs`)
`/proc/diskstats`, only for whole disks backed by hardware
(`/sys/block/<dev>/device` exists), so partitions, loop, dm and zram devices
don't double count. Sectors are always 512 bytes in this file. Busy % is
Δ(ms doing I/O) / Δt.

### Sensors (`sensors.rs`)
Every `/sys/class/hwmon/hwmonN`: `tempN_input` (millidegrees) and
`fanN_input` (RPM), labelled with `tempN_label` / `fanN_label` when present.
NVIDIA's temperature and fan are appended by the UI from NVML, because the
proprietary driver doesn't register an hwmon device.

### Processes (`procs.rs`)
`/proc/<pid>/stat` is parsed from the *last* `)` so command names containing
spaces or parentheses work. CPU % = Δ(utime + stime) / CLK_TCK / Δt (per
core, like `top`). RSS = field 24 × page size (both from `sysconf`). Previous
samples are keyed by (pid, starttime) so a reused PID never inherits the old
process's counters. Command lines are cached per (pid, starttime).

## GPUs

### Discovery (`gpu/mod.rs`)
- `/sys/class/drm/cardN` entries are matched by driver
  (`device/driver` symlink): `amdgpu` → AMD backend, `i915`/`xe` → Intel
  backend, `nvidia` → skipped (handled by NVML).
- NVML (`libnvidia-ml.so.1`) is loaded with `dlopen` by the `nvml-wrapper`
  crate at start-up. If it isn't installed, NVIDIA GPUs aren't shown (and a
  message appears if an `nvidia`-driven card exists).
- Names come from `product_name` (AMD) or the system `pci.ids` database, so
  "DG2 [Arc A770]" becomes "Intel Arc A770".
- GPUs are sorted by PCI address so the order is stable.

Every backend returns the same `GpuSample` (utilisation, VRAM, temperature,
power, clocks, fan, encoder/decoder, per-engine list, extra key/values), so
the UI has no vendor-specific code apart from the accent colour.

### NVIDIA (`gpu/nvidia.rs`)
Straight NVML calls: `utilization_rates`, `memory_info`, `temperature`,
`power_usage`, `enforced_power_limit`, `clock_info`, `fan_speed`,
`encoder_utilization`, `decoder_utilization`. Per-process VRAM comes from
the running compute and graphics process lists; per-process utilisation
from `process_utilization_stats(last_timestamp)`, which returns only samples
newer than the last one seen.

### Intel (`gpu/intel.rs`)
- **Utilisation**: the busiest engine from DRM fdinfo (see below), matching
  nvtop and `intel_gpu_top -p`.
- **Awake %**: 100 − Δ(RC6 residency ms) / Δt. Read from
  `gt/gtN/rc6_residency_ms` (i915) or `tileN/gtN/gtidle/idle_residency_ms`
  (xe). This is device-wide and needs no privileges.
- **Clock**: `gt/gt0/rps_act_freq_mhz` (i915) or `tile0/gt0/freq0/act_freq`
  (xe).
- **Power**: Δ`energy1_input` (µJ) / Δt from the GPU's hwmon; limit from
  `power1_max`.
- **VRAM**: `DRM_IOCTL_I915_QUERY` with `DRM_I915_QUERY_MEMORY_REGIONS` on
  the render node gives `probed_size` (total) and `unallocated_size` (free,
  only meaningful with CAP_PERFMON). Otherwise VRAM used is the sum of
  visible clients' `drm-resident-local*`.

### AMD (`gpu/amd.rs`)
`gpu_busy_percent`, `mem_busy_percent`, `mem_info_vram_used/total`, and from
hwmon: `temp1_input` (edge), `temp2_input` (junction), `power1_average` or
`power1_input`, `power1_cap`, `freq1_input`/`freq2_input` (sclk/mclk, Hz),
`fan1_input`. Falls back to the `*` line of `pp_dpm_sclk`/`pp_dpm_mclk` for
clocks.

### DRM fdinfo (`gpu/fdinfo.rs`)
The kernel's
[DRM client usage stats](https://docs.kernel.org/gpu/drm-usage-stats.html)
are exposed for every open DRM file descriptor in
`/proc/<pid>/fdinfo/<fd>`. Each refresh:

1. For every process, read the `/proc/<pid>/fd` symlinks and keep those
   pointing into `/dev/dri/`.
2. Parse `drm-pdev` (PCI address), `drm-client-id`, `drm-engine-<name>`
   (ns busy), `drm-engine-capacity-<name>`, `drm-cycles-<name>` /
   `drm-total-cycles-<name>` (xe), and `drm-resident-<region>` /
   `drm-memory-vram` for VRAM.
3. De-duplicate by (pdev, client-id), since one client can be reachable
   through several dup'd fds or processes.
4. Engine busy % = Δns / (Δt × capacity), or Δcycles / Δtotal-cycles for xe.
5. Sum per device (per-engine bars, capped at 100) and per PID (process
   GPU % = that process's busiest engine).

## Adding a GPU backend

1. Create `src/gpu/<vendor>.rs` with a struct holding any state you need
   between samples and a `sample(&mut self, du: Option<&DevUsage>) -> GpuSample`
   method.
2. Add a variant to `Backend` and a `Vendor` value in `gpu/mod.rs`, match the
   driver name in `Gpus::new`, and call your `sample` in `Gpus::update`.
3. Give the vendor an accent colour in `ui.rs` (`draw_gpu`).

If the driver supports DRM fdinfo you get per-engine and per-process usage
for free through `DevUsage`.
