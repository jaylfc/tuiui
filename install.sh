#!/bin/sh
# tuiui installer — downloads the latest prebuilt binary for your platform,
# with optional Wayland compositor session installation on Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/jaylfc/tuiui/main/install.sh | sh
#
set -eu

REPO="jaylfc/tuiui"
BIN_DIR="${TUIUI_BIN_DIR:-$HOME/.local/bin}"

# Detect platform for binary download
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

# Detect if running under Wayland or if display manager is available
detect_wayland_or_dm() {
    if [ "$os" != "Linux" ]; then return 1; fi
    
    # Check XDG_SESSION_TYPE for Wayland
    if [ "${XDG_SESSION_TYPE:-}" = "wayland" ]; then return 0; fi
    
    # Check WAYLAND_DISPLAY variable
    if [ -n "${WAYLAND_DISPLAY:-}" ]; then return 0; fi
    
    # Check for display manager processes (use grep since pgrep -x matches exact names only)
    if pgrep -f "gdm\|lightdm\|sddm\|gdm3" >/dev/null 2>&1; then return 0; fi
    
    # Check for display manager sockets/directories
    if [ -S /run/systemd/display-manager ] || [ -d /run/gdm ]; then return 0; fi
    
    # Check for systemd login manager (covers GDM, SDDM, LightDM)
    if systemctl list-units --type=service --state=running 2>/dev/null | grep -qE "(gdm|lightdm|sddm|gdm3|display-manager)"; then return 0; fi
    
    return 1
}

# Check if user is in video group for KMS/DRM access
check_video_group() {
    if groups 2>/dev/null | grep -qw "video"; then return 0; fi
    return 1
}

# Validate paths to prevent injection attacks
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

# Install Wayland session file and optionally polkit rules
install_compositor_session() {
    if [ "${TUIUI_COMPOSITOR:-0}" != "1" ]; then return 0; fi

    EXE_PATH="${TUIUI_EXE_PATH:-$BIN_DIR/tuiui}"
    if ! validate_install_path "$EXE_PATH"; then return 1; fi

    # Prefer tuiui-compositor binary if available, otherwise use tuiui --compositor
    if [ -f "$BIN_DIR/tuiui-compositor" ]; then
        EXE_PATH="$BIN_DIR/tuiui-compositor"
    fi

    # Install desktop session file to system location (requires root for system-wide)
    DESKTOP_DST_DIR="/usr/share/wayland-sessions"
    DESKTOP_DST="$DESKTOP_DST_DIR/tuiui.desktop"
    
    if [ -w "$DESKTOP_DST_DIR" ] 2>/dev/null || [ -w /usr/share ] 2>/dev/null; then
        desktop_exec=$(desktop_field_path "$EXE_PATH")
        desktop_tryexec=$(desktop_field_path "$EXE_PATH")
        {
            printf '[Desktop Entry]\n'
            printf 'Name=tuiui\n'
            printf 'Comment=A tiling terminal-based window manager for Wayland\n'
            printf 'Exec=%s\n' "$desktop_exec"
            printf 'Type=Application\n'
            printf 'DesktopNames=tuiui\n'
            printf 'TryExec=%s\n' "$desktop_tryexec"
        } > "$DESKTOP_DST"
        chmod 644 "$DESKTOP_DST"
        echo "tuiui: installed Wayland session file: $DESKTOP_DST"
    else
        echo "tuiui: compositor session files downloaded but need root to install to $DESKTOP_DST_DIR"
        echo "tuiui: install manually: sudo cp tuiui.desktop $DESKTOP_DST"
        return 0
    fi

    # Install systemd user service for compositor
    SERVICE_DST="$HOME/.config/systemd/user/tuiui-compositor.service"
    if ! validate_install_path "$EXE_PATH"; then return 1; fi
    if ! exe_escaped=$(systemd_escape_exec_path "$EXE_PATH"); then
        echo "tuiui: TUIUI_EXE_PATH contains characters unsupported by systemd ExecStart, got: $EXE_PATH" >&2
        return 1
    fi
    mkdir -p "$(dirname "$SERVICE_DST")"
    {
        printf '[Unit]\n'
        printf 'Description=tuiui Wayland compositor (tiling window manager for the terminal)\n'
        printf 'Documentation=https://github.com/jaylfc/tuiui\n'
        printf '\n'
        printf '[Service]\n'
        printf 'Type=simple\n'
        printf 'ExecStart=%s\n' "$exe_escaped"
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
}

# Install polkit rules for KMS/DRM access when user requests them
install_polkit_rules() {
    POLKIT_RULES_DIR="/etc/polkit-1/rules.d"
    POLKIT_RULES_FILE="$POLKIT_RULES_DIR/50-tuiui-drm.rules"
    
    if [ ! -d "$POLKIT_RULES_DIR" ]; then
        echo "tuiui: polkit rules directory not found at $POLKIT_RULES_DIR"
        return 0
    fi

    # Check if we can write to the directory
    if [ ! -w "$POLKIT_RULES_DIR" ]; then
        echo "tuiui: polkit rules need root to install"
        echo "tuiui: create $POLKIT_RULES_FILE with the following content:"
        echo "---"
        printf 'polkit.addRule(function(action, subject) {\n'
        printf '    if (action.id == "org.freedesktop.devicekit.dri.device-access" &&\n'
        printf '        subject.isInGroup("video")) {\n'
        printf '        return polkit.Result.YES;\n'
        printf '    }\n'
        printf '});\n'
        return 0
    fi

    {
        printf 'polkit.addRule(function(action, subject) {\n'
        printf '    if (action.id == "org.freedesktop.devicekit.dri.device-access" &&\n'
        printf '        subject.isInGroup("video")) {\n'
        printf '        return polkit.Result.YES;\n'
        printf '    }\n'
        printf '});\n'
    } > "$POLKIT_RULES_FILE"
    chmod 644 "$POLKIT_RULES_FILE"
    echo "tuiui: installed polkit rules for DRM access: $POLKIT_RULES_FILE"
}

# Check KMS/DRM permissions and advise user
check_drm_permissions() {
    if [ "$os" != "Linux" ]; then return 0; fi
    
    # Check if /dev/dri exists
    if [ ! -d /dev/dri ]; then
        echo "tuiui: no /dev/dri directory found (no DRM devices)"
        return 0
    fi

    # Check if user can access DRM devices
    if [ ! -r /dev/dri/card0 ] || [ ! -w /dev/dri/card0 ]; then
        if ! check_video_group; then
            echo "tuiui: WARNING: No access to KMS/DRM devices (/dev/dri/card0)"
            echo "tuiui: For compositor mode, either:"
            echo "tuiui:   1. Add your user to the 'video' group: sudo usermod -aG video \$USER"
            echo "tuiui:   2. Or install polkit rules (see --help-polkit)"
        fi
        return 1
    fi
    return 0
}

# Parse arguments
MODE="auto"
while [ $# -gt 0 ]; do
    case "$1" in
        --compositor) MODE="compositor" ;;
        --tui) MODE="tui" ;;
        --help-polkit)
            echo "tuiui: Polkit rules for DRM access allow users in the 'video' group to access /dev/dri/*"
            echo "tuiui: Create /etc/polkit-1/rules.d/50-tuiui-drm.rules with:"
            echo '  polkit.addRule(function(action, subject) {'
            echo '      if (action.id == "org.freedesktop.devicekit.dri.device-access" &&'
            echo '          subject.isInGroup("video")) {'
            echo '          return polkit.Result.YES;'
            echo '      }'
            echo '  });'
            exit 0
            ;;
        --help)
            echo "tuiui: Usage: install.sh [options]"
            echo "tuiui: Options:"
            echo "tuiui:   --compositor  Install for Wayland compositor mode (requires Linux with DRM)"
            echo "tuiui:   --tui         Install for TUI (terminal) mode only"
            echo "tuiui:   --help-polkit Show polkit rules for DRM access"
            echo "tuiui: Environment variables:"
            echo "tuiui:   TUIUI_BIN_DIR     Install directory (default: ~/.local/bin)"
            echo "tuiui:   TUIUI_INSTALL_DEPS Install optional dependencies (default: 0)"
            echo "tuiui:   TUIUI_SKIP_DEPS     Skip optional dependencies (default: 0)"
            exit 0
            ;;
        *) echo "tuiui: unknown option: $1" >&2; exit 1 ;;
    esac
    shift
done

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
if ! curl -fsSL "$url" | tar -xz -C "$BIN_DIR" 2>/dev/null || [ ! -f "$BIN_DIR/tuiui" ]; then
    echo "tuiui: no prebuilt binary for $target in $tag."
    echo "Install with Rust instead:  cargo install --git https://github.com/$REPO"
    exit 1
fi
chmod +x "$BIN_DIR/tuiui"
echo "tuiui: installed $tag -> $BIN_DIR/tuiui"

# Determine installation mode on Linux
if [ "$os" = "Linux" ] && [ "$MODE" = "auto" ]; then
    if detect_wayland_or_dm; then
        echo "tuiui: Wayland or display manager detected."
        
        # In non-interactive mode, default to TUI unless TUIUI_COMPOSITOR is set
        if [ -t 0 ] 2>/dev/null; then
            printf "tuiui: Install as Wayland compositor? [y/N] "
            read -r reply
            case "$reply" in
                [Yy]* ) TUIUI_COMPOSITOR=1 ;;
            esac
        elif [ "${TUIUI_COMPOSITOR:-0}" = "1" ]; then
            echo "tuiui: TUIUI_COMPOSITOR=1: installing compositor session"
        fi
    fi
fi

# Install compositor session files on Linux
install_compositor_session

# Check DRM permissions if compositor mode
if [ "${TUIUI_COMPOSITOR:-0}" = "1" ]; then
    check_drm_permissions
    
    # Offer to install polkit rules if user wants them
    if [ -t 0 ] 2>/dev/null && [ ! -r /dev/dri/card0 ]; then
        printf "tuiui: Install polkit rules for DRM access? [y/N] "
        read -r reply
        case "$reply" in
            [Yy]* ) install_polkit_rules ;;
        esac
    fi
fi

# Optional, OS-aware dependency step. Installs helpers some tray controls need
# (currently blueutil for macOS Bluetooth). Transparent and skippable: it prints
# what it runs, skips silently with no package manager, honours TUIUI_SKIP_DEPS,
# and in a non-interactive `curl | sh` requires explicit TUIUI_INSTALL_DEPS=1 so
# piping the installer never surprises you with package installs.
install_optional_deps() {
  [ "${TUIUI_SKIP_DEPS:-0}" = "1" ] && return 0
  if [ ! -t 0 ] && [ "${TUIUI_INSTALL_DEPS:-0}" != "1" ]; then return 0; fi
  case "$(uname -s)" in
    Darwin)
      if command -v brew >/dev/null 2>&1 && ! command -v blueutil >/dev/null 2>&1; then
        echo "tuiui: installing optional dependency blueutil (Bluetooth control)…"
        brew install blueutil || echo "tuiui: blueutil install skipped (run 'brew install blueutil' later for Bluetooth control)"
      fi ;;
    Linux) : ;; # bluetoothctl/rfkill ship with the distro
  esac
}
install_optional_deps

case ":$PATH:" in
  *":$BIN_DIR:"*) echo "Run it with:  tuiui" ;;
  *) echo "Add $BIN_DIR to your PATH, then run:  tuiui"
     echo "  e.g.  echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.zprofile" ;;
esac

if [ "${TUIUI_COMPOSITOR:-0}" = "1" ]; then
    echo ""
    echo "tuiui: compositor session installed. To start tuiui as your Wayland compositor:"
    echo "tuiui:   Log out and select 'tuiui' from your display manager"
    echo "tuiui:   Or run: systemctl --user start tuiui-compositor"
fi