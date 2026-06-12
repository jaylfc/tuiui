#!/bin/sh
# tuiui installer — downloads the latest prebuilt binary for your platform,
# with optional Wayland compositor session installation on Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/jaylfc/tuiui/main/install.sh | sh
#
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

# Install compositor session files (GDM/LightDM Wayland session) on Linux
if [ "${TUIUI_COMPOSITOR:-0}" = "1" ] && [ "$os" = "Linux" ]; then
  validate_absolute_path() {
      case "$1" in
          /*) ;;
          *) echo "tuiui: path must be absolute, got: $1" >&2; return 1 ;;
      esac
  }

  reject_control_path() {
      case "$1" in
          *[[:cntrl:]]*) echo "tuiui: path must not contain control characters, got: $1" >&2; return 1 ;;
      esac
  }

  validate_install_path() {
      validate_absolute_path "$1" && reject_control_path "$1"
  }

  desktop_escape_path() {
      printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g'
  }

  desktop_field_path() {
      escaped=$(desktop_escape_path "$1")
      case "$escaped" in
          *[[:space:]]*) printf '"%s"' "$escaped" ;;
          *) printf '%s' "$escaped" ;;
      esac
  }

  systemd_escape_exec_path() {
      escaped=$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/%/%%/g')
      case "$escaped" in
          *[[:space:]]*|*\\*|*\"*) printf '"%s"' "$escaped" ;;
          *) printf '%s' "$escaped" ;;
      esac
  }

  install_compositor_session() {
      if [ "$os" != "Linux" ]; then return 0; fi

      if ! pgrep -x "gdm|lightdm|sddm|gdm3|gdm-wayland" >/dev/null 2>&1 \
          && [ ! -S /run/systemd/display-manager ] \
          && [ ! -d /run/gdm ]; then
          echo "tuiui: no display manager detected, skipping compositor session install"
          return 0
      fi

      DESKTOP_SRC="$BIN_DIR/tuiui.desktop"
      DESKTOP_DST_DIR="/usr/share/wayland-sessions"
      DESKTOP_DST="$DESKTOP_DST_DIR/tuiui.desktop"
      if [ -f "$DESKTOP_SRC" ]; then
          if ! validate_install_path "$BIN_DIR/tuiui"; then
              return 1
          fi
          if ! mkdir -p "$DESKTOP_DST_DIR" || [ ! -w "$DESKTOP_DST_DIR" ]; then
              echo "tuiui: compositor session files downloaded but need root to install to $DESKTOP_DST_DIR"
              return 0
          fi
          desktop_exec=$(desktop_field_path "$BIN_DIR/tuiui")
          desktop_tryexec=$(desktop_field_path "$BIN_DIR/tuiui")
          {
              printf '[Desktop Entry]\n'
              printf 'Name=tuiui\n'
              printf 'Comment=A tiling terminal-based window manager for Wayland\n'
              printf 'Exec=%s --compositor\n' "$desktop_exec"
              printf 'Type=Application\n'
              printf 'DesktopNames=tuiui\n'
              printf 'TryExec=%s\n' "$desktop_tryexec"
          } > "$DESKTOP_DST"
          chmod 644 "$DESKTOP_DST"
          echo "tuiui: installed Wayland session file: $DESKTOP_DST"
      fi

      SERVICE_SRC="$BIN_DIR/tuiui-compositor.service"
      SERVICE_DST="$HOME/.config/systemd/user/tuiui-compositor.service"
      if [ -f "$SERVICE_SRC" ]; then
          mkdir -p "$(dirname "$SERVICE_DST")"
          EXE_PATH="${TUIUI_EXE_PATH:-$BIN_DIR/tuiui}"
          if ! validate_install_path "$EXE_PATH"; then
              return 1
          fi
          if ! exe_escaped=$(systemd_escape_exec_path "$EXE_PATH"); then
              echo "tuiui: TUIUI_EXE_PATH contains characters unsupported by systemd ExecStart, got: $EXE_PATH" >&2
              return 1
          fi
          {
              printf '[Unit]\n'
              printf 'Description=tuiui Wayland compositor (tiling window manager for the terminal)\n'
              printf 'Documentation=https://github.com/jaylfc/tuiui\n'
              printf '\n'
              printf '[Service]\n'
              printf 'Type=simple\n'
              printf 'ExecStart=%s --compositor\n' "$exe_escaped"
              printf 'Restart=on-failure\n'
              printf 'RestartSec=2\n'
              printf 'Environment=XDG_CURRENT_DESKTOP=tuiui\n'
              printf 'Environment=XDG_SESSION_TYPE=wayland\n'
              printf '\n'
              printf '[Install]\n'
              printf 'WantedBy=default.target\n'
          } > "$SERVICE_DST"
          chmod 644 "$SERVICE_DST"
          systemctl --user daemon-reload 2>/dev/null || true
          echo "tuiui: installed compositor service to $SERVICE_DST"
      fi
  }
  install_compositor_session
fi

# Optional, OS-aware dependency step. Installs helpers some features need:
#   blueutil (macOS) — Bluetooth tray control
#   gpm (Linux) — mouse on a bare console / VT
#   sshpass — automates the one-time password for Systems → Add Remote
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
          echo "tuiui: installing optional dependency sshpass (remote-system setup)…"
          brew install sshpass 2>/dev/null \
            || brew install esolitos/ipa/sshpass 2>/dev/null \
            || echo "tuiui: sshpass install skipped (Add Remote will prompt for the password interactively instead)"
        fi
      fi ;;
    Linux)
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
      fi ;;
  esac
}
install_optional_deps

case ":$PATH:" in
  *":$BIN_DIR:"*) echo "Run it with:  tuiui" ;;
  *) echo "Add $BIN_DIR to your PATH, then run:  tuiui"
     echo "  e.g.  echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zprofile" ;;
esac
