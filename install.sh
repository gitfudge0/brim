#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required to build brim" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
DEST="$BIN_DIR/brim"

echo "Installing brim to $DEST"

cargo build --release -p brim-cli

SRC="$SCRIPT_DIR/target/release/brim"
if [[ ! -x "$SRC" ]]; then
  echo "release binary not found at $SRC" >&2
  exit 1
fi

mkdir -p "$BIN_DIR"
cp "$SRC" "$DEST"
chmod 755 "$DEST"

echo "Built release binary successfully"
echo "Installed $("$DEST" --version)"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    echo
    echo "Add this to your shell config:"
    echo "export PATH=\"$BIN_DIR:\$PATH\""
    ;;
esac
