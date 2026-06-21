# Integrations

`brim json` emits compact, stable JSON keyed by provider and window. Pipe it
anywhere. The shape:

```json
{
  "version": "0.2.0",
  "usage": {
    "claude": { "session": { "remaining_pct": 0.42, "resets_at": "2026-06-21T18:00:00Z" } }
  }
}
```

`remaining_pct` is 0.0–1.0. All snippets below assume `jq` is installed.

## tmux status bar

```tmux
set -g status-right '#(brim json | jq -r ".usage.claude.session.remaining_pct * 100 | floor | \"claude \\(.)%\"") '
set -g status-interval 60
```

## Starship (custom command module)

```toml
[custom.brim]
command = "brim json | jq -r '.usage.claude.session.remaining_pct * 100 | floor | \"\\(.)%\"'"
when = "command -v brim"
format = "🔋 [$output]($style) "
```

## Waybar (custom module)

```json
"custom/brim": {
  "exec": "brim json | jq -c '{text: ((.usage.claude.session.remaining_pct * 100 | floor | tostring) + \"%\"), tooltip: \"Claude session quota\"}'",
  "return-type": "json",
  "interval": 60
}
```

## Polybar / scripts

```bash
brim json | jq -r '.usage | to_entries[] | "\(.key): \((.value.session.remaining_pct // 0) * 100 | floor)%"'
```

> Keep `brim autosync` enabled so these read cached data instantly instead of
> hitting provider APIs on every status-bar refresh.
