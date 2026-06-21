# Changelog

All notable changes are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions follow SemVer.

## [0.3.0]

First public release. brim tracks AI assistant quotas (Claude, Codex, Copilot)
from one CLI/TUI, now installable as a prebuilt binary on Linux, macOS, and
Windows.

### Added
- GitHub Actions CI (fmt, clippy, tests on Linux/macOS/Windows).
- Release workflow publishing prebuilt binaries for Linux, macOS (Apple Silicon
  + Intel), and Windows on tagged releases.
- `cargo binstall brim-cli` support via release-asset metadata.
- Windows auto-sync via Task Scheduler (`brim autosync enable`).
- `ARCHITECTURE.md` with an "Adding a provider" guide.
- `docs/INTEGRATIONS.md` with tmux / Starship / Waybar / Polybar snippets.
- `demo.tape` for generating the README demo GIF with vhs.
- Dependabot for Cargo and GitHub Actions updates.

### Changed
- `keyring` backend is now selected per-platform, unblocking macOS and Windows
  builds (previously hardcoded to the Linux backend).

### Fixed
- Clippy `ptr_arg` lint in `brim-core::history`.

## [0.2.0]
- Interval-based auto-sync via OS service.

## [0.1.3]
- Launch interactive dashboard on bare `brim`.
