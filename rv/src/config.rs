//! `~/.config/rv/keybindings.toml`: a patch over the default keymap.
//!
//! The file maps **commands to keys** — `diff_next_hunk = "Down"` — because
//! the commands are the finite, validated vocabulary and the key is the
//! user's choice. Sections name the leader (`[keys]` is "no leader"), and a
//! dotted section scopes its binds to one pane: `[keys.files]` binds direct
//! keys that answer only while the cursor is in the file list.
//!
//! This module owns the file's *shape*: sections, panes, leader names and key
//! spellings. The command vocabulary and the collision rules live in
//! [`crate::app::keymap`], which is where the default table is.

mod settings;

pub use settings::Settings;
pub use settings::SortName;
pub use settings::load_settings;
pub use settings::settings_path;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use crossterm::event::KeyCode;
use serde::Deserialize;

use crate::app::Context;
use crate::app::Leader;

#[derive(Debug, Default)]
pub struct Config {
    pub leaders: Vec<(Leader, char)>,
    pub binds: Vec<Bind>,
}

/// One line of the file: this command, under this leader, in this pane (or
/// everywhere), now answers these keys. Empty `keys` is an unbind.
#[derive(Debug)]
pub struct Bind {
    pub leader: Option<Leader>,
    pub pane: Option<Context>,
    pub command: String,
    pub keys: Vec<KeyCode>,
}

/// The panes a bind can be scoped to. `writing`/`confirming`/`finding` are
/// modal text handlers, not table-driven, and are rejected by name.
pub const PANES: &[(&str, Context)] = &[
    ("files", Context::Files),
    ("commits", Context::Commits),
    ("comments", Context::Comments),
    ("diff", Context::Diff),
    ("stack", Context::Stack),
];

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    leaders: BTreeMap<String, String>,
    #[serde(default)]
    keys: BTreeMap<String, RawEntry>,
    #[serde(default)]
    actions: BTreeMap<String, RawEntry>,
    #[serde(default)]
    goto: BTreeMap<String, RawEntry>,
    #[serde(default)]
    comment: BTreeMap<String, RawEntry>,
    #[serde(default)]
    view: BTreeMap<String, RawEntry>,
    #[serde(default)]
    mode: BTreeMap<String, RawEntry>,
}

/// A section entry is either a bind (`command = "key"` / `["j", "Down"]`) or
/// a pane sub-table (`[keys.files]`). Serde tells them apart by type.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawEntry {
    One(String),
    Many(Vec<String>),
    Pane(BTreeMap<String, RawKeys>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawKeys {
    One(String),
    Many(Vec<String>),
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rv").join("keybindings.toml"))
}

/// Reads the config, or the empty patch when there is no file — a reviewer
/// without one gets the defaults, silently.
pub fn load() -> Result<Config> {
    let Some(path) = config_path() else {
        return Ok(Config::default());
    };
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    parse(&text).with_context(|| format!("could not parse {}", path.display()))
}

pub fn parse(text: &str) -> Result<Config> {
    let raw: RawConfig = toml::from_str(text)?;

    let mut leaders = Vec::new();
    for (name, spec) in &raw.leaders {
        let leader = leader_named(name)?;
        leaders.push((leader, leader_key(spec)?));
    }

    let mut binds = Vec::new();
    let sections: [(Option<Leader>, &BTreeMap<String, RawEntry>); 6] = [
        (None, &raw.keys),
        (Some(Leader::Context), &raw.actions),
        (Some(Leader::Goto), &raw.goto),
        (Some(Leader::Comment), &raw.comment),
        (Some(Leader::View), &raw.view),
        (Some(Leader::Mode), &raw.mode),
    ];
    for (leader, entries) in sections {
        for (name, entry) in entries {
            match entry {
                RawEntry::One(spec) => binds.push(bind(leader, None, name, spec_keys(spec)?)),
                RawEntry::Many(specs) => binds.push(bind(leader, None, name, list_keys(specs)?)),
                RawEntry::Pane(scoped) => {
                    let pane = pane_named(name)?;
                    for (command, keys) in scoped {
                        let keys = match keys {
                            RawKeys::One(spec) => spec_keys(spec)?,
                            RawKeys::Many(specs) => list_keys(specs)?,
                        };
                        binds.push(bind(leader, Some(pane), command, keys));
                    }
                }
            }
        }
    }

    Ok(Config { leaders, binds })
}

fn bind(leader: Option<Leader>, pane: Option<Context>, command: &str, keys: Vec<KeyCode>) -> Bind {
    Bind {
        leader,
        pane,
        command: command.to_owned(),
        keys,
    }
}

/// `""` unbinds, anything else is one key.
fn spec_keys(spec: &str) -> Result<Vec<KeyCode>> {
    if spec.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![parse_key(spec)?])
}

fn list_keys(specs: &[String]) -> Result<Vec<KeyCode>> {
    specs.iter().map(|spec| parse_key(spec)).collect()
}

fn leader_named(name: &str) -> Result<Leader> {
    const LEADERS: &[(&str, Leader)] = &[
        ("actions", Leader::Context),
        ("goto", Leader::Goto),
        ("comment", Leader::Comment),
        ("view", Leader::View),
        ("mode", Leader::Mode),
    ];
    if let Some((_, leader)) = LEADERS.iter().find(|(known, _)| *known == name) {
        return Ok(*leader);
    }
    bail!(
        "unknown leader {name:?} — the leaders are {}",
        LEADERS
            .iter()
            .map(|(known, _)| *known)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn pane_named(name: &str) -> Result<Context> {
    if let Some((_, pane)) = PANES.iter().find(|(known, _)| *known == name) {
        return Ok(*pane);
    }
    if matches!(name, "writing" | "confirming" | "finding") {
        bail!("{name:?} is a typing mode, not a pane — its keys are not remappable");
    }
    let panes = PANES.iter().map(|(known, _)| *known).collect::<Vec<_>>();
    match closest(name, &panes) {
        Some(suggestion) => bail!("unknown pane {name:?} — did you mean {suggestion:?}?"),
        None => bail!("unknown pane {name:?} — the panes are {}", panes.join(", ")),
    }
}

fn leader_key(spec: &str) -> Result<char> {
    if spec == "Space" {
        return Ok(' ');
    }
    let mut chars = spec.chars();
    let key = chars.next().context("a leader key cannot be empty")?;
    if chars.next().is_some() {
        bail!("a leader key is a single character or \"Space\", not {spec:?}");
    }
    Ok(key)
}

fn parse_key(spec: &str) -> Result<KeyCode> {
    let code = match spec {
        "Enter" | "Return" => KeyCode::Enter,
        "Esc" | "Escape" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Backspace" | "BS" => KeyCode::Backspace,
        "Delete" | "Del" => KeyCode::Delete,
        "Insert" | "Ins" => KeyCode::Insert,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" | "PgUp" => KeyCode::PageUp,
        "PageDown" | "PgDn" | "PgDown" => KeyCode::PageDown,
        "Space" => KeyCode::Char(' '),
        _ if spec.starts_with('F')
            && spec.len() > 1
            && spec[1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            let n: u8 = spec[1..]
                .parse()
                .with_context(|| format!("bad F-key {spec:?}"))?;
            KeyCode::F(n)
        }
        _ => {
            let mut chars = spec.chars();
            let key = chars
                .next()
                .with_context(|| format!("empty key name in {spec:?}"))?;
            if chars.next().is_some() {
                bail!("unknown key name {spec:?}");
            }
            KeyCode::Char(key)
        }
    };
    Ok(code)
}

/// The closest of `known` within an edit distance of 2, for did-you-mean.
pub fn closest<'a>(name: &str, known: &[&'a str]) -> Option<&'a str> {
    known
        .iter()
        .map(|candidate| (levenshtein(name, candidate), *candidate))
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b_chars.len()).collect();
    for (i, a_char) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, b_char) in b_chars.iter().enumerate() {
            let cost = if a_char == *b_char { 0 } else { 1 };
            let value = (previous + cost).min(row[j] + 1).min(row[j + 1] + 1);
            previous = row[j + 1];
            row[j + 1] = value;
        }
    }
    row[b_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_the_empty_patch() {
        let config = parse("").unwrap();
        assert!(config.binds.is_empty());
        assert!(config.leaders.is_empty());
    }

    #[test]
    fn a_direct_global_bind() {
        let config = parse("[keys]\ncursor_next_row = \"j\"\n").unwrap();
        assert_eq!(config.binds.len(), 1);
        let bind = &config.binds[0];
        assert_eq!(bind.leader, None);
        assert_eq!(bind.pane, None);
        assert_eq!(bind.command, "cursor_next_row");
        assert_eq!(bind.keys, vec![KeyCode::Char('j')]);
    }

    #[test]
    fn arrays_bind_several_keys() {
        let config = parse("[keys]\ncursor_next_row = [\"j\", \"Down\"]\n").unwrap();
        assert_eq!(
            config.binds[0].keys,
            vec![KeyCode::Char('j'), KeyCode::Down]
        );
    }

    #[test]
    fn a_pane_scoped_bind() {
        let config = parse("[keys.files]\ndiff_toggle_full_context = \"f\"\n").unwrap();
        let bind = &config.binds[0];
        assert_eq!(bind.pane, Some(Context::Files));
        assert_eq!(bind.command, "diff_toggle_full_context");
    }

    #[test]
    fn a_leader_scoped_bind() {
        let config = parse("[goto.diff]\ncursor_last_row = \"e\"\n").unwrap();
        let bind = &config.binds[0];
        assert_eq!(bind.leader, Some(Leader::Goto));
        assert_eq!(bind.pane, Some(Context::Diff));
    }

    #[test]
    fn empty_string_unbinds() {
        let config = parse("[keys]\napp_quit = \"\"\n").unwrap();
        assert!(config.binds[0].keys.is_empty());
    }

    #[test]
    fn leaders_rename_their_key() {
        let config = parse("[leaders]\ngoto = \"G\"\nactions = \"Space\"\n").unwrap();
        assert!(config.leaders.contains(&(Leader::Goto, 'G')));
        assert!(config.leaders.contains(&(Leader::Context, ' ')));
    }

    #[test]
    fn a_typing_mode_is_refused_by_name() {
        let error = parse("[keys.writing]\ncomment_write = \"w\"\n").unwrap_err();
        assert!(error.to_string().contains("typing mode"), "{error}");
    }

    #[test]
    fn a_misspelled_pane_gets_a_suggestion() {
        let error = parse("[keys.filse]\nfiles_next = \"n\"\n").unwrap_err();
        assert!(
            error.to_string().contains("did you mean \"files\""),
            "{error}"
        );
    }

    #[test]
    fn an_unknown_section_is_an_error() {
        assert!(parse("[keyz]\napp_quit = \"q\"\n").is_err());
    }

    #[test]
    fn special_key_names_parse() {
        let config =
            parse("[keys]\na = \"Enter\"\nb = \"Esc\"\nc = \"PgDn\"\nd = \"F3\"\ne = \"Space\"\n")
                .unwrap();
        let keys: Vec<KeyCode> = config.binds.iter().map(|bind| bind.keys[0]).collect();
        assert!(keys.contains(&KeyCode::Enter));
        assert!(keys.contains(&KeyCode::Esc));
        assert!(keys.contains(&KeyCode::PageDown));
        assert!(keys.contains(&KeyCode::F(3)));
        assert!(keys.contains(&KeyCode::Char(' ')));
    }

    #[test]
    fn an_unknown_key_name_is_an_error() {
        assert!(parse("[keys]\napp_quit = \"FooBar\"\n").is_err());
    }
}
