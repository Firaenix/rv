//! `~/.config/rv/Config.toml`: session defaults — the display preferences a
//! reviewer otherwise re-toggles at the top of every review. Every field is
//! optional; an absent one keeps the built-in default, so the file patches
//! the defaults the same way `keybindings.toml` patches the keymap.

use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use serde::Deserialize;

use crate::tree::Sort;

#[derive(Deserialize, Debug, Default, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Open the file list as a tree rather than a flat list.
    pub tree: Option<bool>,
    /// The file list's initial order: `natural`, `added` or `removed`.
    pub sort: Option<SortName>,
    /// Open the diff with full-file context (the default) or changes only.
    pub full_context: Option<bool>,
    /// Group each hunk's removals before its additions.
    pub grouped: Option<bool>,
    /// Tint sidebar rows by their change's proportion.
    pub tint: Option<bool>,
    /// Show the sidebar's `+n -n` column.
    pub counts: Option<bool>,
    /// Open with the sidebar put away.
    pub sidebar_hidden: Option<bool>,
    /// The share of the width the sidebar starts with, in percent — what `+`
    /// and `_` nudge. Clamped the way the keys are.
    pub split: Option<u16>,
    /// Draw the status bar's separators in ASCII — the `RV_ASCII` switch as a
    /// setting; the environment variable still wins when set.
    pub ascii: Option<bool>,
    /// The diff engine to open with: `auto` (difftastic when it is on `PATH`,
    /// the default) or `fallback` (the in-process line diff — `--no-difft` as
    /// a default, which still wins as a flag).
    pub engine: Option<EngineName>,
    /// Refresh the review by itself when the repository moves — a commit, a
    /// checkout, a rebase, an agent's edit landing. On by default.
    pub auto_refresh: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum EngineName {
    Auto,
    Fallback,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum SortName {
    Natural,
    Added,
    Removed,
}

impl From<SortName> for Sort {
    fn from(name: SortName) -> Self {
        match name {
            SortName::Natural => Sort::Natural,
            SortName::Added => Sort::Added,
            SortName::Removed => Sort::Removed,
        }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    super::config_root().map(|dir| dir.join("rv").join("Config.toml"))
}

/// Reads the settings, or the empty patch when there is no file.
pub fn load_settings() -> Result<Settings> {
    let Some(path) = settings_path() else {
        return Ok(Settings::default());
    };
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_keeps_every_default() {
        let settings: Settings = toml::from_str("").unwrap();
        assert!(settings.tree.is_none());
        assert!(settings.sort.is_none());
    }

    #[test]
    fn the_documented_fields_parse() {
        let settings: Settings = toml::from_str(
            "tree = true\nsort = \"added\"\nfull_context = false\ngrouped = true\n\
             tint = false\ncounts = false\nsidebar_hidden = true\nsplit = 40\n\
             ascii = true\nengine = \"fallback\"\n",
        )
        .unwrap();
        assert_eq!(settings.tree, Some(true));
        assert_eq!(Sort::from(settings.sort.unwrap()), Sort::Added);
        assert_eq!(settings.full_context, Some(false));
        assert_eq!(settings.grouped, Some(true));
        assert_eq!(settings.sidebar_hidden, Some(true));
        assert_eq!(settings.split, Some(40));
        assert_eq!(settings.ascii, Some(true));
        assert!(matches!(settings.engine, Some(EngineName::Fallback)));
    }

    #[test]
    fn an_unknown_engine_is_an_error() {
        assert!(toml::from_str::<Settings>("engine = \"difftastic!\"\n").is_err());
    }

    #[test]
    fn an_unknown_field_is_an_error_not_silence() {
        assert!(toml::from_str::<Settings>("tre = true\n").is_err());
    }

    #[test]
    fn an_unknown_sort_is_an_error() {
        assert!(toml::from_str::<Settings>("sort = \"biggest\"\n").is_err());
    }
}
