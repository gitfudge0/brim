# brim

Track your AI assistant quotas across Codex, Claude, and Copilot from one fast terminal interface.

`brim` is a Rust CLI/TUI for checking quota status across multiple AI providers. It gives you readable terminal output for day-to-day use and machine-readable JSON for scripts, dashboards, menu bar apps, and other custom tooling.

- Track multiple providers in one place
- Human-readable terminal status
- Machine-readable JSON for scripts and apps
- Local config and state
- Auth helpers and diagnostics

## Build

```bash
git clone https://github.com/gitfudge0/brim.git
cd brim
cargo build --release
./target/release/brim --help
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

- `brim status [provider] [--fresh]` shows usage status for all providers or one provider
- `brim json [provider] [--fresh]` emits compact machine-readable usage JSON
- `brim json [provider] [--fresh] --full` emits the richer provider summary JSON
- `brim sync [provider]` fetches fresh usage data and stores it locally
- `brim auth status|login|logout` manages provider authentication
- `brim config show|init|edit` manages local config
- `brim diag` prints diagnostic information for local setup issues

## Configuration

- Linux config path: `~/.config/brim/config.toml`
- Linux state path: `~/.local/state/brim/app.db`
- Paths are platform-dependent outside Linux
- Providers are disabled by default until you enable and configure them

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

The main integration surface is `brim json`. By default it returns a compact object with `version` and a `usage` map keyed by provider. Each provider contains canonical window keys like `session`, `weekly`, `monthly`, or `daily`, and each window exposes `remaining_pct` plus `resets_at`.

Use `brim json --full` if you want the richer provider summary array with auth state, plan metadata, notes, source information, and full bucket details.

Basic checks:

```bash
brim status
brim status claude --fresh
brim json
brim json codex --fresh
brim json --full
brim sync
brim config init
brim config show
brim diag
```

Extract a remaining percentage with `jq`:

```bash
brim json codex | jq '.usage.codex.session.remaining_pct'
```

Show provider auth state:

```bash
brim json | jq '.usage | keys'
```

Print the latest notes:

```bash
brim json --full | jq '.[] | { provider, notes }'
```

Status bar or menu bar integration:

```bash
brim json --fresh | jq -r '
  .usage.claude.weekly.remaining_pct as $v
  | "Claude \((($v // 0) * 100) | floor)%"
'
```

Run periodic sync with cron:

```cron
*/10 * * * * brim sync >/dev/null 2>&1
```

Use in a shell script for alerts:

```bash
if brim json --fresh | jq -e '
  .usage
  | to_entries[]
  | .value
  | to_entries[]
  | select((.value.remaining_pct // 1) < 0.15)
' >/dev/null; then
  notify-send "brim" "Quota below 15%"
fi
```

Use from custom apps:

- Poll `brim json --fresh` from a menu bar app, dashboard, or desktop widget
- Read a single provider window directly from `.usage.<provider>.<window>`
- Trigger alerts when any provider drops below a threshold
- Feed `brim json` or `brim json --full` into tmux, i3blocks, SketchyBar, Polybar, or a local web app

## Limitations

- Provider integrations are a mix of official and internal or reverse-engineered surfaces
- Auth and quota visibility can vary by provider and account type
- Some values may be marked `experimental`, `local`, `derived`, or `stale` depending on source confidence

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md), and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
