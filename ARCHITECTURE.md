# Architecture

brim is a Cargo workspace. Each crate has one job:

| Crate | Responsibility |
|-------|----------------|
| `brim-core` | Domain types: `Provider` trait, `UsageSnapshot`, `QuotaBucket`, time windows, confidence labels. No I/O. |
| `brim-providers` | Concrete providers (Claude, Codex, Copilot), the `ProviderRegistry`, and the `SyncEngine` that fetches + stores snapshots. |
| `brim-auth` | Credential discovery, OAuth device flow, local-file probing. |
| `brim-storage` | SQLite database, TOML config, XDG paths, OS keyring. |
| `brim-cli` | `clap` command tree, the OS auto-sync service, output formatting. |
| `brim-tui` | `ratatui` dashboard. |

Dependency direction: `cli`/`tui` → `providers` → `core`, with `auth` and
`storage` as leaves. `core` depends on nothing internal.

## Data flow

```
provider API ──► Provider::fetch_usage ──► UsageSnapshot ──► SyncEngine
                                                               │
                                          SQLite (history) ◄───┤
                                                               ▼
                                          status / json / TUI / autosync
```

Every value carries a `Confidence` label (`Official`, `ProviderLocal`,
`Stale`, …) so the UI can show how trustworthy a number is — this is the spine
of the project, not an afterthought.

## Adding a provider

The `Provider` trait (`crates/core/src/provider.rs`) is the only contract. To
add one (e.g. Gemini):

1. Add a `ProviderId` variant in `crates/core/src/models.rs` and wire it into
   `ProviderId::all()`, `as_str()`, and `display_name()`.
2. Create `crates/providers/src/<name>/provider.rs` implementing `Provider`:
   - `fetch_usage` should try strategies in order (official API → CLI/local
     file → experimental) and label each value's confidence.
   - Return normalized `QuotaBucket`s keyed by `WindowKind`.
3. Register it in `crates/providers/src/registry.rs`.
4. Add unit tests that parse a sample API response into a `UsageSnapshot`.

No changes to `cli`, `tui`, or `storage` are needed — they iterate the
registry. Providers compile into the single binary; there is intentionally no
dynamic plugin loading (see the rejected-alternatives note in the PR that
introduced this file).
