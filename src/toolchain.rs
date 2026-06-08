//! Toolchain detection for store installs. A catalog recipe's `method`
//! (`go` / `cargo` / `npm` / `pip` / `brew`) implies a language toolchain the
//! install needs. When that toolchain is missing the store warns and offers to
//! install it first (see [`preamble`]).
//!
//! Pure string/shell generation — the store runs the result in a PTY window.

/// A language / package toolchain a recipe depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toolchain {
    Go,
    Cargo,
    Node,
    Python,
    Brew,
}

impl Toolchain {
    /// The toolchain a recipe `method` requires, if any. `source`/`binary` and
    /// unknown methods return `None` (no single toolchain to bootstrap).
    pub fn for_method(method: &str) -> Option<Toolchain> {
        match method {
            "go" => Some(Toolchain::Go),
            "cargo" | "rust" => Some(Toolchain::Cargo),
            "npm" | "node" | "yarn" => Some(Toolchain::Node),
            "pip" | "pipx" | "python" => Some(Toolchain::Python),
            "brew" => Some(Toolchain::Brew),
            _ => None,
        }
    }

    /// The binary to probe with `command -v` to decide if the toolchain is present.
    pub fn probe_bin(self) -> &'static str {
        match self {
            Toolchain::Go => "go",
            Toolchain::Cargo => "cargo",
            Toolchain::Node => "npm",
            Toolchain::Python => "python3",
            Toolchain::Brew => "brew",
        }
    }

    /// Human-readable label shown in the prompt.
    pub fn label(self) -> &'static str {
        match self {
            Toolchain::Go => "Go",
            Toolchain::Cargo => "Rust (cargo)",
            Toolchain::Node => "Node.js (npm)",
            Toolchain::Python => "Python 3 (pip)",
            Toolchain::Brew => "Homebrew",
        }
    }

    /// A best-effort shell snippet that installs this toolchain, trying the
    /// platform's package managers in turn (Homebrew on macOS; apt/dnf/pacman on
    /// Linux). Rust uses the official cross-platform `rustup`; Homebrew uses its
    /// official bootstrap. Always a single shell command (may need `sudo`).
    pub fn install_snippet(self) -> &'static str {
        match self {
            // rustup works on macOS and Linux, no sudo, installs into ~/.cargo.
            Toolchain::Cargo => {
                "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . \"$HOME/.cargo/env\""
            }
            Toolchain::Go => {
                "( command -v brew >/dev/null 2>&1 && brew install go ) \
|| ( command -v apt-get >/dev/null 2>&1 && sudo apt-get install -y golang ) \
|| ( command -v dnf >/dev/null 2>&1 && sudo dnf install -y golang ) \
|| ( command -v pacman >/dev/null 2>&1 && sudo pacman -S --noconfirm go )"
            }
            Toolchain::Node => {
                "( command -v brew >/dev/null 2>&1 && brew install node ) \
|| ( command -v apt-get >/dev/null 2>&1 && sudo apt-get install -y nodejs npm ) \
|| ( command -v dnf >/dev/null 2>&1 && sudo dnf install -y nodejs npm ) \
|| ( command -v pacman >/dev/null 2>&1 && sudo pacman -S --noconfirm nodejs npm )"
            }
            Toolchain::Python => {
                "( command -v brew >/dev/null 2>&1 && brew install python ) \
|| ( command -v apt-get >/dev/null 2>&1 && sudo apt-get install -y python3 python3-pip ) \
|| ( command -v dnf >/dev/null 2>&1 && sudo dnf install -y python3 python3-pip ) \
|| ( command -v pacman >/dev/null 2>&1 && sudo pacman -S --noconfirm python python-pip )"
            }
            // Homebrew's official bootstrap (interactive; may prompt for sudo).
            Toolchain::Brew => {
                "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
            }
        }
    }
}

/// A shell preamble for an install of `app` whose recipe `method` needs a
/// toolchain. When the toolchain is missing it warns, prompts `y/N`, and on
/// yes installs it (aborting the window on decline or failure); when present it
/// is a no-op and the app install proceeds. Returns an empty string when the
/// method needs no single toolchain.
///
/// `homepage` is shown so the user can finish manually if they decline.
pub fn preamble(app: &str, method: &str, homepage: &str) -> String {
    let Some(tc) = Toolchain::for_method(method) else {
        return String::new();
    };
    let probe = tc.probe_bin();
    let label = tc.label();
    let snippet = tc.install_snippet();
    format!(
        "if ! command -v {probe} >/dev/null 2>&1; then \
echo '{app} needs {label}, which is not installed.'; \
printf 'Install {label} now? [y/N] '; read -r _tc_ans; \
case \"$_tc_ans\" in \
[yY]*) echo; echo 'Installing {label} …'; {snippet} || {{ echo; echo 'Could not install {label}. Get it from {home}'; echo 'Close this window (\u{2715}) when done.'; exec \"$SHELL\"; }} ;; \
*) echo; echo 'Skipped. Install {label} first, then retry {app}.'; echo '  {home}'; echo 'Close this window (\u{2715}) when done.'; exec \"$SHELL\" ;; \
esac; \
fi; ",
        home = homepage,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_maps_to_toolchain() {
        assert_eq!(Toolchain::for_method("go"), Some(Toolchain::Go));
        assert_eq!(Toolchain::for_method("cargo"), Some(Toolchain::Cargo));
        assert_eq!(Toolchain::for_method("npm"), Some(Toolchain::Node));
        assert_eq!(Toolchain::for_method("pip"), Some(Toolchain::Python));
        assert_eq!(Toolchain::for_method("brew"), Some(Toolchain::Brew));
        // No single toolchain to bootstrap for these.
        assert_eq!(Toolchain::for_method("source"), None);
        assert_eq!(Toolchain::for_method("binary"), None);
    }

    #[test]
    fn probe_and_snippet_match_toolchain() {
        assert_eq!(Toolchain::Go.probe_bin(), "go");
        assert!(Toolchain::Cargo.install_snippet().contains("rustup"));
        assert!(Toolchain::Go.install_snippet().contains("brew install go"));
        assert!(Toolchain::Node.install_snippet().contains("npm"));
        assert!(Toolchain::Brew.install_snippet().contains("Homebrew/install"));
    }

    #[test]
    fn preamble_detects_prompts_and_installs() {
        let p = preamble("sheets", "go", "https://github.com/maaslalani/sheets");
        assert!(p.contains("command -v go"));
        assert!(p.contains("sheets needs Go"));
        assert!(p.contains("read -r")); // interactive prompt
        assert!(p.contains("brew install go")); // the install snippet on yes
        assert!(p.contains("https://github.com/maaslalani/sheets")); // homepage on decline
    }

    #[test]
    fn preamble_empty_when_no_toolchain_needed() {
        assert_eq!(preamble("sfm", "source", "https://example.com"), "");
    }
}
