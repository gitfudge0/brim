# Contributing

`brim` should stay small, practical, and terminal-first. Prefer scoped changes that improve the tool without adding unnecessary surface area.

## Setup

```bash
cargo fmt
cargo build
cargo test --workspace
```

## Issues

- Use the GitHub issue templates when reporting bugs or proposing features.
- Include the affected command, provider, or workflow.
- Do not open public issues for security vulnerabilities. Follow [SECURITY.md](SECURITY.md) instead.

## Before You Submit

- Keep changes focused
- Update docs when behavior or commands change
- Update docs and tests when CLI or JSON output changes
- Add or update tests when behavior changes
- Run `cargo fmt`
- Run `cargo test --workspace`

## Pull Requests

- Explain what changed
- Link the related issue when one exists
- Explain how you tested it
- Include screenshots or command output only when they add clarity
- Call out provider-specific limitations or assumptions when relevant
