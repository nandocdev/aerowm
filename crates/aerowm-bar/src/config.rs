//! Bar configuration loaded from Luau (`~/.config/aerowm/bar.luau`).
//!
//! ```luau
//! return {
//!     height = 28,
//!     widgets = { "workspaces", "clock" },
//!     background = "#1a1b26",
//!     foreground = "#c0caf5",
//!     accent = "#7aa2f7",
//!     inactive = "#565f89",
//! }
//! ```

use std::path::PathBuf;

/// Widget names understood by the bar. Unknown entries are ignored
/// with a warning so configs stay forward compatible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Widget {
    Workspaces,
    Clock,
}

impl Widget {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "workspaces" => Some(Widget::Workspaces),
            "clock" => Some(Widget::Clock),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BarConfig {
    pub height: u32,
    pub widgets: Vec<Widget>,
    /// ARGB colors (alpha always 0xFF for now).
    pub background: u32,
    pub foreground: u32,
    pub accent: u32,
    pub inactive: u32,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            height: 28,
            widgets: vec![Widget::Workspaces, Widget::Clock],
            background: 0xFF_1A_1B_26,
            foreground: 0xFF_C0_CA_F5,
            accent: 0xFF_7A_A2_F7,
            inactive: 0xFF_56_5F_89,
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let mut p = PathBuf::from(xdg);
        p.push("aerowm/bar.luau");
        return p;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let mut p = PathBuf::from(home);
    p.push(".config/aerowm/bar.luau");
    p
}

fn parse_color(raw: &str, fallback: u32) -> u32 {
    let hex = raw.trim().trim_start_matches('#');
    if hex.len() == 6
        && let Ok(rgb) = u32::from_str_radix(hex, 16)
    {
        return 0xFF00_0000 | rgb;
    }
    log::warn!("invalid color '{raw}', using fallback");
    fallback
}

pub fn load() -> BarConfig {
    let defaults = BarConfig::default();
    let path = config_path();
    let src = match std::fs::read_to_string(&path) {
        Ok(src) => src,
        Err(_) => {
            log::info!("no bar config at {}, using defaults", path.display());
            return defaults;
        }
    };

    let lua = mlua::Lua::new();
    let table: mlua::Result<mlua::Table> = lua.load(&src).eval();
    let table = match table {
        Ok(t) => t,
        Err(e) => {
            log::warn!("bar config parse error ({e}), using defaults");
            return defaults;
        }
    };

    let height: u32 = table.get::<u32>("height").unwrap_or(defaults.height).clamp(16, 64);

    let mut widgets = Vec::new();
    if let Ok(list) = table.get::<mlua::Table>("widgets") {
        for item in list.sequence_values::<String>() {
            match item {
                Ok(name) => match Widget::parse(&name) {
                    Some(w) => widgets.push(w),
                    None => log::warn!("unknown widget '{name}', ignoring"),
                },
                Err(e) => log::warn!("bad widget entry: {e}"),
            }
        }
    }
    if widgets.is_empty() {
        widgets = defaults.widgets.clone();
    }

    let color = |key: &str, fallback: u32| -> u32 {
        table
            .get::<String>(key)
            .map(|raw| parse_color(&raw, fallback))
            .unwrap_or(fallback)
    };

    BarConfig {
        height,
        widgets,
        background: color("background", defaults.background),
        foreground: color("foreground", defaults.foreground),
        accent: color("accent", defaults.accent),
        inactive: color("inactive", defaults.inactive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_widgets_cover_workspaces_and_clock() {
        let cfg = BarConfig::default();
        assert!(cfg.widgets.contains(&Widget::Workspaces));
        assert!(cfg.widgets.contains(&Widget::Clock));
    }

    #[test]
    fn parses_colors() {
        assert_eq!(parse_color("#1a1b26", 0), 0xFF_1A_1B_26);
        assert_eq!(parse_color("notacolor", 42), 42);
    }

    #[test]
    fn parses_widget_names() {
        assert_eq!(Widget::parse("workspaces"), Some(Widget::Workspaces));
        assert_eq!(Widget::parse("clock"), Some(Widget::Clock));
        assert_eq!(Widget::parse("frobnicator"), None);
    }
}
