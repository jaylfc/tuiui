#!/bin/sh
# tuiui installer — downloads the latest prebuilt binary for your platform.
#
#   curl -fsSL https://raw.githubusercontent.com/jaylfc/tuiui/main/install.sh | sh
#
# Override the install directory with TUIUI_BIN_DIR (default: ~/.local/bin).
set -eu

REPO="jaylfc/tuiui"
BIN_DIR="${TUIUI_BIN_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Darwin/arm64)        target="aarch64-apple-darwin" ;;
  Darwin/x86_64)       target="x86_64-apple-darwin" ;;
  Linux/x86_64)        target="x86_64-unknown-linux-gnu" ;;
  *)
    echo "tuiui: no prebuilt binary for $os/$arch."
    echo "Install with Rust instead:  cargo install --git https://github.com/$REPO"
    exit 1 ;;
esac

echo "tuiui: finding latest release…"
tag="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' | head -1 | cut -d'"' -f4)"
if [ -z "${tag:-}" ]; then
  echo "tuiui: no published release yet."
  echo "Install with Rust instead:  cargo install --git https://github.com/$REPO"
  exit 1
fi

url="https://github.com/$REPO/releases/download/$tag/tuiui-$target.tar.gz"
echo "tuiui: downloading $tag ($target)…"
mkdir -p "$BIN_DIR"
curl -fsSL "$url" | tar -xz -C "$BIN_DIR"
chmod +x "$BIN_DIR/tuiui"

echo "tuiui: installed $tag -> $BIN_DIR/tuiui"
case ":$PATH:" in
  *":$BIN_DIR:"*) echo "Run it with:  tuiui" ;;
  *) echo "Add $BIN_DIR to your PATH, then run:  tuiui"
     echo "  e.g.  echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zprofile" ;;
esac
