//! The keymap as its own config file: what `rv keymap` prints, and what
//! `rv config` seeds a first file with. The output reloads to itself, which
//! is what makes the dump an answer to "what did my config actually do".

use std::collections::BTreeMap;

use super::Keymap;
use super::RuntimeBinding;
use super::spec;
use super::vocab;
use crate::app::Context;
use crate::app::Leader;

/// What the seeded template says before any binding, because the file is the
/// first place a reviewer meets the vocabulary.
const HEADER: &str = "\
# rv keybindings — a patch over the defaults. Delete a line to restore it.
#
# command = \"key\"        one key      command = [\"j\", \"Down\"]   several
# command = \"\"           unbind
#
# Bare [section] binds everywhere; [section.<pane>] binds in one pane.
# Panes: files, commits, comments, diff, stack.
#
# The command prefix says what is acted on:
#   cursor_*                    the focused pane's cursor (focus-relative)
#   diff_* / files_* / comment_* that thing, wherever the focus is
#   pane_* / layout_* / app_*   the screen or the program
#
# Not remappable: Ctrl+C (always quits), Shift+arrows (sideways scroll),
# and the keys of the ? popup and the comment editor.
";

/// The six sections, in file order: the direct keys, then one per leader.
const SECTIONS: &[(&str, Option<Leader>)] = &[
    ("keys", None),
    ("actions", Some(Leader::Context)),
    ("goto", Some(Leader::Goto)),
    ("comment", Some(Leader::Comment)),
    ("view", Some(Leader::View)),
    ("mode", Some(Leader::Mode)),
];

impl Keymap {
    /// The fully-commented default keymap: `rv config`'s seed for a first
    /// file, listing every command so the vocabulary is met in the editor
    /// rather than in an error message.
    #[must_use]
    pub fn template() -> String {
        format!("{HEADER}\n{}", Self::default().render_toml(true))
    }

    /// The effective keymap — defaults plus the user's patch — as TOML.
    #[must_use]
    pub fn render_toml(&self, commented: bool) -> String {
        let prefix = if commented { "# " } else { "" };
        let mut out = String::new();

        out.push_str("[leaders]\n");
        for (name, leader) in SECTIONS
            .iter()
            .filter_map(|(name, leader)| leader.map(|leader| (*name, leader)))
        {
            let key = self.leader_key(leader);
            let spelled = if key == ' ' {
                "Space".to_owned()
            } else {
                key.to_string()
            };
            out.push_str(&format!("{prefix}{name} = \"{spelled}\"\n"));
        }

        for (name, leader) in SECTIONS {
            let global = self.merged_rows(*leader, None);
            let mut body = String::new();
            if !global.is_empty() {
                body.push_str(&format!("\n[{name}]\n"));
                push_binds(&mut body, prefix, &global);
            }
            for (pane_name, pane) in crate::config::PANES {
                let scoped = self.merged_rows(*leader, Some(*pane));
                if !scoped.is_empty() {
                    body.push_str(&format!("\n[{name}.{pane_name}]\n"));
                    push_binds(&mut body, prefix, &scoped);
                }
            }
            out.push_str(&body);
        }
        out
    }

    /// The section's binds as `command → keys`, rows for one scope merged so
    /// a command two rows reach (a user's pane bind plus a default that also
    /// covers the pane) prints once with every key that works.
    fn merged_rows(
        &self,
        leader: Option<Leader>,
        pane: Option<Context>,
    ) -> Vec<(&'static str, Vec<String>)> {
        let mut order: Vec<&'static str> = Vec::new();
        let mut merged: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
        let in_scope = |row: &RuntimeBinding| match pane {
            None => row.contexts.is_empty(),
            Some(pane) => row.contexts.contains(&pane),
        };
        for row in self
            .bindings
            .iter()
            .filter(|row| row.leader == leader && in_scope(row))
        {
            let name = vocab::config_name(row.command);
            let keys = merged.entry(name).or_insert_with(|| {
                order.push(name);
                Vec::new()
            });
            for code in &row.codes {
                let spelled = spec(*code);
                if !keys.contains(&spelled) {
                    keys.push(spelled);
                }
            }
        }
        order
            .into_iter()
            .map(|name| (name, merged.remove(name).unwrap_or_default()))
            .collect()
    }
}

fn push_binds(out: &mut String, prefix: &str, binds: &[(&'static str, Vec<String>)]) {
    for (name, keys) in binds {
        let value = match keys.as_slice() {
            [only] => format!("\"{only}\""),
            many => format!(
                "[{}]",
                many.iter()
                    .map(|key| format!("\"{key}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        out.push_str(&format!("{prefix}{name} = {value}\n"));
    }
}
