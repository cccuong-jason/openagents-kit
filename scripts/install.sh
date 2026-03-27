#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-latest}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
REPO="cccuong-jason/openagents-kit"

case "$(uname -s):$(uname -m)" in
  Linux:x86_64)
    TARGET="x86_64-unknown-linux-gnu"
    ;;
  Darwin:x86_64)
    TARGET="x86_64-apple-darwin"
    ;;
  Darwin:arm64)
    TARGET="aarch64-apple-darwin"
    ;;
  *)
    echo "Unsupported platform: $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

ASSET="openagents-kit-$TARGET"
if [[ "$VERSION" == "latest" ]]; then
  URL="https://github.com/$REPO/releases/latest/download/$ASSET"
else
  URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"
fi

mkdir -p "$INSTALL_DIR"
curl -fsSL "$URL" -o "$INSTALL_DIR/openagents-kit"
chmod +x "$INSTALL_DIR/openagents-kit"

echo "Installed openagents-kit to $INSTALL_DIR/openagents-kit"
echo "Run 'openagents-kit setup' to scan local Codex, Claude, and Gemini configs."
