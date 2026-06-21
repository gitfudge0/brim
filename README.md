<p align="center">
  <img src="assets/brim-logo.svg" alt="brim — AI quota tracker" width="320">
</p>

# brim

<p align="center">
  <a href="https://github.com/gitfudge0/brim/actions/workflows/ci.yml"><img src="https://github.com/gitfudge0/brim/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT"></a>
</p>

Track your AI assistant quotas across Codex, Claude, and Copilot from one fast terminal interface.

`brim` is a Rust CLI/TUI for checking quota status across multiple AI providers. It gives you readable terminal output for day-to-day use and machine-readable JSON for scripts, dashboards, menu bar apps, and other custom tooling.

- Track multiple providers in one place
- Human-readable terminal status
- Machine-readable JSON for scripts and apps
- Local config and state
- Auth helpers and diagnostics

## Demo

<p align="center">
  <img src="assets/demo.gif" alt="brim terminal demo" width="700">
</p>

> The GIF is generated from [`demo.tape`](demo.tape) with
> [vhs](https://github.com/charmbracelet/vhs): `vhs demo.tape`. Run it after
> building to refresh `assets/demo.gif`.

## Interactive Dashboard

Run `brim` with no subcommand to open an interactive guided dashboard (TUI). It shows live usage status for enabled providers, lets you add or remove providers, and runs auth flows inline. If stdout is not a terminal (piped or redirected), bare `brim` prints the same output as `brim status`.

## Install

Pick whichever you have on hand — all of them give you the same `brim` binary.

**Prebuilt binary (no toolchain needed)** — download for your platform from the
[latest release](https://github.com/gitfudge0/brim/releases/latest), or:

```bash
# Cargo users — fetches a prebuilt binary, no compiling:
cargo binstall brim-cli

# From source via Cargo:
cargo install --git https://github.com/gitfudge0/brim brim-cli
```

Prebuilt binaries are published for Linux (x86_64), macOS (Apple Silicon and
Intel), and Windows (x86_64) on every tagged release.

**From a clone**, into `~/.local/bin` (override with `PREFIX`):

```bash
git clone https://github.com/gitfudge0/brim.git
cd brim
./install.sh                      # or: PREFIX=/tmp/brim-test ./install.sh
```

## Uninstall

Remove the locally installed binary:

```bash
brim uninstall
```

This removes the installed `brim` binary and the background auto-sync service
(systemd unit / launchd agent). It does not remove config, state, database
contents, or stored credentials.

If you installed via Cargo instead, use:

```bash
cargo uninstall brim
```

## Quick Start

```bash
brim config init
brim auth login claude
brim auth login codex
brim auth login copilot
brim sync
brim status
```

## Commands

- `brim` (no subcommand) opens the interactive dashboard, or prints `brim status` output when stdout is not a terminal
- `brim status [provider] [--fresh]` shows usage status for all providers or one provider
- `brim json [provider] [--fresh] [--full]` emits machine-readable usage JSON (`--full` for the richer summary)
- `brim sync [provider]` fetches fresh usage data and stores it locally
- `brim provider list|enable|disable <provider>` lists or toggles which providers are active
- `brim autosync enable|disable|status` runs background syncing on a schedule via an OS service (systemd on Linux, launchd on macOS, Task Scheduler on Windows)
- `brim autosync interval [secs] [--provider <provider>]` shows or sets the sync cadence (changes apply to a running auto-sync within seconds, no restart)
- `brim auth status|login|logout` manages provider authentication
- `brim config show|init|edit` manages local config
- `brim diag` prints diagnostic information for local setup issues
- `brim uninstall` removes the locally installed binary and the auto-sync service (keeps config, state, and credentials)

## Configuration

- Linux config path: `~/.config/brim/config.toml`
- Linux state path: `~/.local/share/state/brim/app.db`
- Paths are platform-dependent outside Linux
- Providers are disabled by default; enable them with `brim provider enable <provider>` or by editing the config
- `install.sh` enables background auto-sync automatically; opt out with `BRIM_NO_SERVICE=1 ./install.sh`, or turn it off later with `brim autosync disable`

Example config:

```toml
[general]
poll_interval_secs = 300
stale_threshold_secs = 600
prune_after_days = 30
log_level = "info"

[providers.codex]
enabled = true
poll_interval_secs = 300

[providers.claude]
enabled = true
poll_interval_secs = 300

[providers.copilot]
enabled = false
poll_interval_secs = 300
```

## Build Your Own With `brim`

The main integration surface is `brim json`. By default it returns a compact object with `version` and a `usage` map keyed by provider. Each provider has canonical window keys (`session`, `weekly`, `monthly`, `daily`), each exposing `remaining_pct` and `resets_at`. Add `--full` for the richer summary array (auth state, plan, notes, source, full bucket details).

Read a single window:

```bash
brim json codex | jq '.usage.codex.session.remaining_pct'
```

Alert when any provider drops below 15%:

```bash
if brim json --fresh | jq -e '.usage[][] | select((.remaining_pct // 1) < 0.15)' >/dev/null; then
  notify-send "brim" "Quota below 15%"
fi
```

Sync periodically with cron:

```cron
*/10 * * * * brim sync >/dev/null 2>&1
```

Feed `brim json` into tmux, i3blocks, SketchyBar, Polybar, a menu bar app, or any local dashboard. Note that `status`/`auth status` show all supported providers, while `sync` without a provider targets enabled ones only.

## Community

Contribution and support guidelines live in [CONTRIBUTING.md](CONTRIBUTING.md),
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), and
[SECURITY.md](SECURITY.md).

## Limitations

- Provider integrations are a mix of official and internal or reverse-engineered surfaces
- Auth and quota visibility can vary by provider and account type
- Some values may be marked `experimental`, `local`, `derived`, or `stale` depending on source confidence

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md), and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
