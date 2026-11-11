//! `~/.config/rv/Config.toml`: session defaults — the display preferences a
//! reviewer otherwise re-toggles at the top of every review. Every field is
//! optional; an absent one keeps the built-in default, so the file patches
//! the defaults the same way `keybindings.toml` patches the keymap.

use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use serde::Deserialize;

use super::agent::AgentSettings;
use crate::tree::Sort;

mod lsp;

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Open the file list as a tree rather than a flat list.
    pub tree: Option<bool>,
    /// The file list's initial order: `natural` (or `path`), `added`,
    /// `removed` or `size` — the last for `rv browse`, whose files changed
    /// nothing.
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
    /// Wrap a commit row's hash and subject onto as many rows as it takes,
    /// in the commits list, rather than clipping the subject to one. Off by
    /// default — one row per commit is what fits the most changes on screen.
    pub wrap_commit_subjects: Option<bool>,
    /// Open a symbol's hover card when the pointer rests on its name. Off by
    /// default: motion reporting is terminal-dependent, and `g i` is the key.
    pub hover: Option<bool>,
    /// Open with the line provenance gutter showing. Off by default.
    pub provenance: Option<bool>,
    /// `off` (the default) or `auto`: refine `g d`, `g r` and `g i` with a
    /// language server when the reviewed head is the working copy.
    pub lsp: Option<lsp::LspMode>,
    /// Which server runs for which language, overriding the built-in.
    pub lsp_servers: Option<lsp::LspServers>,
    /// The agent CLI rv may run; absent, rv runs none.
    pub agent: Option<AgentSettings>,
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        self.agent.as_ref().map_or(Ok(()), AgentSettings::validate)
    }

    /// What `--no-agent` and an absent `[agent]` both come to: `None`.
    #[must_use]
    pub fn agent_config(&self) -> Option<rv_core::agent::AgentConfig> {
        self.agent.as_ref().and_then(AgentSettings::to_config)
    }
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
    #[serde(alias = "path")]
    Natural,
    Added,
    Removed,
    Size,
}

impl From<SortName> for Sort {
    fn from(name: SortName) -> Self {
        match name {
            SortName::Natural => Sort::Natural,
            SortName::Added => Sort::Added,
            SortName::Removed => Sort::Removed,
            SortName::Size => Sort::Size,
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
    let settings: Settings =
        toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))?;
    settings
        .validate()
        .with_context(|| format!("could not use {}", path.display()))?;
    Ok(settings)
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
