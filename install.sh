#!/bin/sh
# tuiui installer — downloads the latest prebuilt binary for your platform.
#
#   curl -fsSL https://raw.githubusercontent.com/jaylfc/tuiui/main/install.sh | sh
#
# Override the install directory with TUIUI_BIN_DIR (default: ~/.local/bin).
set -eu

REPO="jaylfc/tuiui"
BIN_DIR="${TUIUI_BIN_DIR:-$HOME/.local/bin}"

# Stock Debian often has wget but not curl; accept either.
fetch() {
  if command -v curl >/dev/null 2>&1; then curl -fsSL "$1"
  elif command -v wget >/dev/null 2>&1; then wget -qO- "$1"
  else echo "tuiui: need curl or wget to download" >&2; return 1
  fi
}

os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Darwin/arm64)        target="aarch64-apple-darwin" ;;
  Darwin/x86_64)       target="x86_64-apple-darwin" ;;
  Linux/x86_64)        target="x86_64-unknown-linux-gnu" ;;
  Linux/aarch64)       target="aarch64-unknown-linux-gnu" ;;
  *)
    echo "tuiui: no prebuilt binary for $os/$arch."
    echo "Install with Rust instead:  cargo install --git https://github.com/$REPO"
    exit 1 ;;
esac

echo "tuiui: finding latest release…"
tag="$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' | head -1 | cut -d'"' -f4)"
if [ -z "${tag:-}" ]; then
  echo "tuiui: no published release yet."
  echo "Install with Rust instead:  cargo install --git https://github.com/$REPO"
  exit 1
fi

url="https://github.com/$REPO/releases/download/$tag/tuiui-$target.tar.gz"
echo "tuiui: downloading $tag ($target)…"
mkdir -p "$BIN_DIR"
if ! fetch "$url" | tar -xz -C "$BIN_DIR" 2>/dev/null || [ ! -f "$BIN_DIR/tuiui" ]; then
  echo "tuiui: no prebuilt binary for $target in $tag."
  echo "Install with Rust instead:  cargo install --git https://github.com/$REPO"
  exit 1
fi
chmod +x "$BIN_DIR/tuiui"

echo "tuiui: installed $tag -> $BIN_DIR/tuiui"

# Optional, OS-aware dependency step. Installs helpers some features need:
#   blueutil (macOS)  — Bluetooth tray control
#   gpm (Linux)       — mouse on a bare console / VT
#   sshpass           — automates the one-time password for Systems → Add Remote
# Transparent and skippable: it prints what it runs, skips silently with no
# package manager, honours TUIUI_SKIP_DEPS, and in a non-interactive
# `curl | sh` requires explicit TUIUI_INSTALL_DEPS=1 so piping the installer
# never surprises you with package installs.
install_optional_deps() {
  [ "${TUIUI_SKIP_DEPS:-0}" = "1" ] && return 0
  if [ ! -t 0 ] && [ "${TUIUI_INSTALL_DEPS:-0}" != "1" ]; then return 0; fi
  case "$(uname -s)" in
    Darwin)
      if command -v brew >/dev/null 2>&1; then
        if ! command -v blueutil >/dev/null 2>&1; then
          echo "tuiui: installing optional dependency blueutil (Bluetooth control)…"
          brew install blueutil || echo "tuiui: blueutil install skipped (run 'brew install blueutil' later for Bluetooth control)"
        fi
        if ! command -v sshpass >/dev/null 2>&1; then
          # sshpass is not in homebrew-core; use the common tap, skip on failure.
          echo "tuiui: installing optional dependency sshpass (remote-system setup)…"
          brew install sshpass 2>/dev/null \
            || brew install esolitos/ipa/sshpass 2>/dev/null \
            || echo "tuiui: sshpass install skipped (Add Remote will prompt for the password interactively instead)"
        fi
      fi ;;
    Linux)
      # bluetoothctl/rfkill ship with the distro; gpm gives a bare-console
      # mouse and sshpass automates Systems → Add Remote key transfers.
      pkgs=""
      command -v gpm >/dev/null 2>&1 || pkgs="gpm"
      command -v sshpass >/dev/null 2>&1 || pkgs="$pkgs sshpass"
      pkgs="$(echo "$pkgs" | sed 's/^ *//')"
      if [ -n "$pkgs" ]; then
        echo "tuiui: installing optional dependencies: $pkgs …"
        sudo apt-get install -y $pkgs 2>/dev/null \
          || sudo dnf install -y $pkgs 2>/dev/null \
          || sudo pacman -S --noconfirm $pkgs 2>/dev/null \
          || sudo zypper install -y $pkgs 2>/dev/null \
          || echo "tuiui: optional deps skipped (install '$pkgs' with your package manager later)"
        # Start gpm now (and on boot) where systemd manages it.
        if command -v gpm >/dev/null 2>&1 && command -v systemctl >/dev/null 2>&1; then
          sudo systemctl enable --now gpm 2>/dev/null || true
        fi
      fi ;;
  esac
}
install_optional_deps

case ":$PATH:" in
  *":$BIN_DIR:"*) echo "Run it with:  tuiui" ;;
  *) echo "Add $BIN_DIR to your PATH, then run:  tuiui"
     echo "  e.g.  echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zprofile" ;;
esac
