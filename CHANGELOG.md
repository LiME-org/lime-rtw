# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.4.0] - 2026-06-16

### Added

- PID namespace-aware tracing: LiME now detects the current PID namespace at startup (via `/proc/self/ns/pid` and the `NS_GET_ID` ioctl) and automatically translates kernel-level PIDs into the target namespace when running inside a container. A new `--target-pid-ns INODE` flag overrides this detection, which is needed when LiME runs on the host but the traced process is inside a container.
- Allow tracing an existing process or thread by passing a numeric PID/TID as the trace target, instead of a command to execute. When a single numeric argument is given, LiME attaches to the existing process rather than spawning a new one.
- Compile-time feature flags: `tui` and `proto` features let users opt out of the TUI viewer or protobuf support to reduce build dependencies. Both are documented in the README with example build commands.

### Changed

- The TUI viewer (`tui` feature) is now a **default feature** — included by default, disabled with `--no-default-features`.
- Protobuf output support is now gated behind the **`proto` Cargo feature**, making `protobuf-compiler` an optional dependency.
- Replaced `__u32`/`__u64` with `u32`/`u64` in BPF probe code, using standard BPF CO-RE type aliases.
- Added a NULL-pointer guard on the `tsp` (timespec) argument in the `ppoll` eBPF handler.
- Hardened BPF file descriptor lookups with more robust error handling.
- Clamped BPF process-info chunk reads to silence eBPF verifier false-positive out-of-bounds errors.
- Removed dead code from the BPF probes.

### Fixed

- Fixed `on_sched_yield` and `on_sys_exit_sched_yield` probes to use `get_sched_policy()` instead of reading `t->policy` directly, which was bypassing CO-RE relocation.
- Renamed a misleading function name in the BPF probes.

## [0.3.0] - 2026-05-06

### Added

- Add a Gitlab CI benchmark job for the `extract` command using the cyclictest benchmark trace.

### Changed

- Switch periodic, sporadic, and arrival-curve extraction to the standalone `lime-model-extractors` crate.
- Refactor arrival-curve extraction so the sporadic MIT model and delta-min/delta-max arrival curve are produced from the same extractor state.
- Refactor task mapping and task metadata updates for scheduler, priority, affinity, throttle-release, and process-info events.
- Update crate dependencies, including `libbpf-rs`/`libbpf-cargo` 0.26 and newer `clap`, `serde`, and `duration-str` versions.
- Silence the warning of eBPF pointer-size reads reported by clang-tidy.
- Simplify SysV semop argument handling when detecting potentially blocking calls.

### Fixed

- Preserve sparse CPU affinity masks when clipping eBPF affinity snapshots.
- Avoid emitting scheduler and affinity follow-up metadata events for tasks filtered out of the trace.

## [0.2.4] - 2026-03-04

### Added

- Add env `LIME_TARGET_KERNEL_VERSION` for selecting the target kernel at compile time. By default it targets the build system kernel version.

### Changed

- Make policy/affinity snapshotting more robust.

## [0.2.3] - 2026-01-28

### Fixed

- Restore sched_yield tracing support.

### Changed

- Updated crate dependencies to newer versions.

## [0.2.2] - 2025-09-22

### Added

- Store the first and last event times of each task in both ISO8601 format and as a `CLOCK_BOOTTIME` timestamp within `*.info.json`.
- Store the LiME start time, along with the system’s scheduling configuration, the Linux kernel version, the LiME version, and the arguments used to invoke `lime-rtw`, in a new per-trace file `sysinfo.json`.

## [0.2.1] - 2025-07-22

### Added

- `extract` command: Added two flags `--after` and `--before` to limit offline extraction to sub-intervals of the entire trace.
  - `--after TIME` — skip all events with timestamps earlier than `TIME`
  - `--before TIME` — skip all events with timestamps later than `TIME`
  - In both cases, `TIME` is a [`CLOCK_BOOTTIME`](https://linux.die.net/man/2/clock_gettime) timestamp, i.e., a timestamp relative to when the system started up, which is the clock source used by LiME's eBPF probes.

### Removed

- Removed the `frequency`, `mean`, and `variance` fields from the JSON output of periodic models. These were leftovers from protyping work that are no more relevant in the current version.

## [0.2.0] - 2025-06-03

### Added

- TUI viewer: Provides a Text User Interface (TUI) to view extracted results
  - Use command: lime-rtw view 'results-folder'
- Enhanced pselect6/select eBPF probes: Added NULL pointer detection for timespec parameters
  - Note: Model extraction remains unchanged; planned improvements for future releases will incorporate this enhancement

### Fixed

- Arrival Curve: Prevent emission of invalid arrival curves with zero upper bounds

## [0.1.0] - 2025-05-09

First public release
