.PHONY: help build build-release test fmt fmt-check check clippy clean install uninstall run tui status json json-full sync diag

CARGO ?= cargo
CLI_PACKAGE := brim-cli
TUI_PACKAGE := brim-tui
ARGS ?=

help:
	@printf '%s\n' \
		'brim Makefile targets:' \
		'' \
		'  make build          Build the workspace' \
		'  make build-release  Build the workspace in release mode' \
		'  make test           Run all workspace tests' \
		'  make fmt            Format Rust sources' \
		'  make fmt-check      Check formatting without rewriting files' \
		'  make check          Run cargo check for the workspace' \
		'  make clippy         Run clippy for the workspace' \
		'  make install        Build and install brim into ~/.local/bin' \
		'  make uninstall      Remove the locally installed brim binary' \
		'  make clean          Remove build artifacts' \
		'' \
		'CLI helpers:' \
		'  make run ARGS="--help"' \
		'  make tui ARGS=""' \
		'  make status ARGS="codex --fresh"' \
		'  make json ARGS="codex"' \
		'  make json-full ARGS="codex --fresh"' \
		'  make sync ARGS="claude"' \
		'  make diag'

build:
	$(CARGO) build --workspace

build-release:
	$(CARGO) build --workspace --release

test:
	$(CARGO) test --workspace

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all --check

check:
	$(CARGO) check --workspace

clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

clean:
	$(CARGO) clean

install:
	./install.sh

uninstall:
	$(CARGO) run -p $(CLI_PACKAGE) -- uninstall

run:
	$(CARGO) run -p $(CLI_PACKAGE) -- $(ARGS)

tui:
	$(CARGO) run -p $(TUI_PACKAGE) -- $(ARGS)

status:
	$(CARGO) run -p $(CLI_PACKAGE) -- status $(ARGS)

json:
	$(CARGO) run -p $(CLI_PACKAGE) -- json $(ARGS)

json-full:
	$(CARGO) run -p $(CLI_PACKAGE) -- json --full $(ARGS)

sync:
	$(CARGO) run -p $(CLI_PACKAGE) -- sync $(ARGS)

diag:
	$(CARGO) run -p $(CLI_PACKAGE) -- diag $(ARGS)
