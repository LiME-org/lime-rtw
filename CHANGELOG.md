# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
