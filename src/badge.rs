//! Dock app badges: a one-cell colored initial that identifies an app even when
//! its window has been renamed. Color comes from config (per-app override) or a
//! deterministic hash of the app key.

use crate::cell::Rgba;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    pub letter: char,
    pub color: Rgba,
}

/// First alphanumeric char of `key`, uppercased; `'?'` if none.
fn initial(key: &str) -> char {
    key.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_ascii_uppercase()).unwrap_or('?')
}

/// A small set of named colors, plus `#rrggbb`. Returns `None` if unrecognized.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Rgba::rgb(r, g, b));
        }
        return None;
    }
    let c = match s.to_ascii_lowercase().as_str() {
        "red" => (220, 60, 60),
        "orange" => (230, 130, 40),
        "amber" => (220, 160, 40),
        "yellow" => (220, 200, 50),
        "green" => (70, 180, 90),
        "teal" => (40, 180, 170),
        "cyan" => (50, 180, 210),
        "blue" => (70, 130, 230),
        "indigo" => (90, 90, 210),
        "violet" => (150, 100, 220),
        "magenta" => (200, 70, 190),
        "pink" => (230, 110, 160),
        "gray" | "grey" => (130, 130, 140),
        _ => return None,
    };
    Some(Rgba::rgb(c.0, c.1, c.2))
}

/// Deterministic fallback color from the key (stable per app).
fn hashed_color(key: &str) -> Rgba {
    // FNV-1a over the lowercased key → pick a hue from a fixed palette.
    let mut h: u32 = 2166136261;
    for b in key.to_ascii_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    const PALETTE: &[(u8, u8, u8)] = &[
        (220, 60, 60), (230, 130, 40), (220, 200, 50), (70, 180, 90),
        (40, 180, 170), (70, 130, 230), (150, 100, 220), (200, 70, 190),
    ];
    let (r, g, b) = PALETTE[(h as usize) % PALETTE.len()];
    Rgba::rgb(r, g, b)
}

/// Resolve the badge for an app group key, honoring config overrides (matched as
/// a case-insensitive substring of the key).
pub fn badge_for(key: &str, overrides: &BTreeMap<String, String>) -> Badge {
    let lower = key.to_ascii_lowercase();
    let color = overrides
        .iter()
        .find(|(kw, _)| !kw.is_empty() && lower.contains(&kw.to_ascii_lowercase()))
        .and_then(|(_, c)| parse_color(c))
        .unwrap_or_else(|| hashed_color(key));
    Badge { letter: initial(key), color }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ov(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn initial_letter() {
        assert_eq!(badge_for("Claude", &ov(&[])).letter, 'C');
        assert_eq!(badge_for("kilo", &ov(&[])).letter, 'K');
        assert_eq!(badge_for("1pass", &ov(&[])).letter, '1');
        assert_eq!(badge_for("", &ov(&[])).letter, '?');
    }

    #[test]
    fn config_override_by_substring() {
        let o = ov(&[("claude", "orange")]);
        assert_eq!(badge_for("Claude Code", &o).color, parse_color("orange").unwrap());
    }

    #[test]
    fn hash_fallback_is_stable() {
        assert_eq!(badge_for("btop", &ov(&[])).color, badge_for("btop", &ov(&[])).color);
    }

    #[test]
    fn parse_named_and_hex() {
        assert_eq!(parse_color("#ff8000"), Some(Rgba::rgb(255, 128, 0)));
        assert!(parse_color("orange").is_some());
        assert!(parse_color("nope").is_none());
    }
}
