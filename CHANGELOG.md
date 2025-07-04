# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.2.1] - 2025-07-XX

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
