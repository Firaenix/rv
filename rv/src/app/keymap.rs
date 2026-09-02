//! The runtime keymap: the default [`BINDINGS`] table, patched by the user's
//! `keybindings.toml`.
//!
//! The config is a **patch**: rebinding a command moves it off its old key,
//! `command = ""` unbinds it, and an untouched command keeps its default.
//! Collisions inside the user's own file are errors — they wrote something
//! that cannot mean anything — while collisions between the file and the
//! defaults resolve in the user's favour plus a startup alert, because
//! overriding defaults is the file's entire purpose.

mod vocab;

use std::collections::HashMap;

use anyhow::Result;
use anyhow::bail;
use crossterm::event::KeyCode;

use super::Context;
use super::bindings::{BINDINGS, Binding, Command, Group, Leader};
use crate::config::{Bind, Config};

/// The panes a binding can be scoped to — the browse contexts, without the
/// modal typing states.
const ALL_PANES: &[Context] = &[
    Context::Files,
    Context::Commits,
    Context::Comments,
    Context::Diff,
    Context::Stack,
];

#[derive(Debug)]
pub struct RuntimeBinding {
    pub keys_label: String,
    pub group: Group,
    pub leader: Option<Leader>,
    /// Empty means every pane, mirroring [`Binding::contexts`].
    pub contexts: Vec<Context>,
    pub what: &'static str,
    pub codes: Vec<KeyCode>,
    pub(super) command: Command,
    /// Whether the user's config touched this row — what decides steal versus
    /// error when two rows claim one key.
    user: bool,
}

#[derive(Debug)]
pub struct Keymap {
    bindings: Vec<RuntimeBinding>,
    leader_keys: HashMap<Leader, char>,
    warnings: Vec<String>,
}

impl Default for Keymap {
    fn default() -> Self {
        let bindings = BINDINGS.iter().map(RuntimeBinding::from_static).collect();
        let leader_keys = Leader::ALL
            .iter()
            .map(|leader| (*leader, leader.key()))
            .collect();
        Self {
            bindings,
            leader_keys,
            warnings: Vec::new(),
        }
    }
}

impl RuntimeBinding {
    fn from_static(binding: &Binding) -> Self {
        Self {
            keys_label: binding.keys.to_owned(),
            group: binding.group,
            leader: binding.leader,
            contexts: binding.contexts.to_vec(),
            what: binding.what,
            codes: binding.codes.to_vec(),
            command: binding.command,
            user: false,
        }
    }
}

impl Keymap {
    pub fn from_config(config: &Config) -> Result<Self> {
        let mut keymap = Self::default();
        keymap.apply(config)?;
        keymap.settle_collisions()?;
        Ok(keymap)
    }

    /// The startup alerts the patch earned: every default the user's file
    /// moved aside, said out loud once.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    pub fn bindings(&self) -> &[RuntimeBinding] {
        &self.bindings
    }

    pub fn leader_key(&self, leader: Leader) -> char {
        self.leader_keys
            .get(&leader)
            .copied()
            .unwrap_or_else(|| leader.key())
    }

    /// The row `key` runs from `context`, scoped rows ahead of global ones so
    /// a pane-specific bind beats an everywhere bind whatever order the table
    /// holds them in.
    pub(super) fn lookup(
        &self,
        leader: Option<Leader>,
        key: KeyCode,
        context: Context,
    ) -> Option<&RuntimeBinding> {
        self.bindings
            .iter()
            .find(|row| {
                row.leader == leader && row.contexts.contains(&context) && row.codes.contains(&key)
            })
            .or_else(|| {
                self.bindings.iter().find(|row| {
                    row.leader == leader && row.contexts.is_empty() && row.codes.contains(&key)
                })
            })
    }

    fn apply(&mut self, config: &Config) -> Result<()> {
        for (leader, key) in &config.leaders {
            self.leader_keys.insert(*leader, *key);
        }
        for (index, first) in Leader::ALL.iter().enumerate() {
            for second in &Leader::ALL[index + 1..] {
                if self.leader_key(*first) == self.leader_key(*second) {
                    bail!(
                        "the {} and {} leaders both open on {:?} — one submenu would be unreachable",
                        first.label(),
                        second.label(),
                        self.leader_key(*first),
                    );
                }
            }
        }
        for bind in &config.binds {
            self.apply_bind(bind)?;
        }
        Ok(())
    }

    fn apply_bind(&mut self, bind: &Bind) -> Result<()> {
        let Some(command) = vocab::command_by_name(&bind.command) else {
            let names: Vec<&str> = vocab::ALL_COMMANDS
                .iter()
                .map(|command| vocab::config_name(*command))
                .collect();
            match crate::config::closest(&bind.command, &names) {
                Some(meant) => {
                    bail!(
                        "unknown command {:?} — did you mean {meant:?}?",
                        bind.command
                    )
                }
                None => bail!(
                    "unknown command {:?} — `rv keymap` lists every command",
                    bind.command
                ),
            }
        };
        if bind.keys.is_empty() {
            self.unbind(bind.leader, bind.pane, command);
            return Ok(());
        }
        match bind.pane {
            // A global bind re-keys every row of the (leader, command) cell —
            // move semantics: the old keys are freed.
            None => {
                let mut rebound = false;
                for row in self
                    .bindings
                    .iter_mut()
                    .filter(|row| row.leader == bind.leader && row.command == command)
                {
                    row.codes = bind.keys.clone();
                    row.keys_label = keys_label(&bind.keys);
                    row.user = true;
                    rebound = true;
                }
                if !rebound {
                    self.push_row(bind.leader, Vec::new(), command, bind.keys.clone());
                }
            }
            // A scoped bind replaces an exactly-matching scoped row, and
            // otherwise *adds* one: narrowing an inherited broader row on a
            // one-pane patch would take keys away the user never mentioned.
            Some(pane) => {
                if let Some(row) = self.bindings.iter_mut().find(|row| {
                    row.leader == bind.leader && row.command == command && row.contexts == [pane]
                }) {
                    row.codes = bind.keys.clone();
                    row.keys_label = keys_label(&bind.keys);
                    row.user = true;
                } else {
                    self.push_row(bind.leader, vec![pane], command, bind.keys.clone());
                }
            }
        }
        Ok(())
    }

    fn push_row(
        &mut self,
        leader: Option<Leader>,
        contexts: Vec<Context>,
        command: Command,
        codes: Vec<KeyCode>,
    ) {
        let (group, what) = vocab::meta_for(command);
        self.bindings.push(RuntimeBinding {
            keys_label: keys_label(&codes),
            group,
            leader,
            contexts,
            what,
            codes,
            command,
            user: true,
        });
    }

    fn unbind(&mut self, leader: Option<Leader>, pane: Option<Context>, command: Command) {
        match pane {
            None => self
                .bindings
                .retain(|row| !(row.leader == leader && row.command == command)),
            Some(pane) => {
                let mut emptied = Vec::new();
                for (index, row) in self.bindings.iter_mut().enumerate() {
                    if row.leader != leader || row.command != command {
                        continue;
                    }
                    if row.contexts.is_empty() {
                        // A global row survives everywhere but here.
                        row.contexts = ALL_PANES
                            .iter()
                            .copied()
                            .filter(|other| *other != pane)
                            .collect();
                        row.user = true;
                    } else {
                        row.contexts.retain(|other| *other != pane);
                        if row.contexts.is_empty() {
                            emptied.push(index);
                        }
                    }
                }
                for index in emptied.into_iter().rev() {
                    self.bindings.remove(index);
                }
            }
        }
    }

    /// Applies the collision policy: a user row and a default row claiming
    /// one key resolve by **steal** (the default yields, with an alert); two
    /// user rows claiming one key are a contradiction and an error.
    fn settle_collisions(&mut self) -> Result<()> {
        loop {
            let Some((user_index, default_index, key)) = self.next_collision()? else {
                break;
            };
            let winner = vocab::config_name(self.bindings[user_index].command);
            let user_contexts = self.bindings[user_index].contexts.clone();
            let loser_name = vocab::config_name(self.bindings[default_index].command);
            self.warnings.push(format!(
                "{} now runs {winner}; {loser_name} moved aside — `rv keymap` shows the result",
                spec(key),
            ));
            let loser = &mut self.bindings[default_index];
            let survives_nowhere = if user_contexts.is_empty() {
                // A global steal takes the key itself.
                loser.codes.retain(|code| *code != key);
                loser.keys_label = keys_label(&loser.codes);
                loser.codes.is_empty()
            } else {
                // A scoped steal narrows the loser out of the winner's panes,
                // so the key keeps meaning what it always did elsewhere.
                if loser.contexts.is_empty() {
                    loser.contexts = ALL_PANES.to_vec();
                }
                loser.contexts.retain(|pane| !user_contexts.contains(pane));
                loser.contexts.is_empty()
            };
            if survives_nowhere {
                self.bindings.remove(default_index);
            }
        }
        for leader in Leader::ALL {
            let key = KeyCode::Char(self.leader_key(*leader));
            if let Some(row) = self
                .bindings
                .iter()
                .find(|row| row.leader.is_none() && row.codes.contains(&key))
            {
                self.warnings.push(format!(
                    "the {} leader shadows the direct key for {} — leaders are answered first",
                    leader.label(),
                    vocab::config_name(row.command),
                ));
            }
        }
        Ok(())
    }

    /// The first user-versus-default collision, or the error a user-versus-
    /// user one earns. `None` when the map is settled.
    fn next_collision(&self) -> Result<Option<(usize, usize, KeyCode)>> {
        for (first_index, first) in self.bindings.iter().enumerate() {
            if !first.user {
                continue;
            }
            for (second_index, second) in self.bindings.iter().enumerate() {
                if first_index == second_index
                    || first.leader != second.leader
                    || first.command == second.command
                    || !overlaps(&first.contexts, &second.contexts)
                {
                    continue;
                }
                let Some(shared) = first.codes.iter().find(|code| second.codes.contains(code))
                else {
                    continue;
                };
                if second.user {
                    bail!(
                        "the config binds {shared:?} to both {} and {} in the same place",
                        vocab::config_name(first.command),
                        vocab::config_name(second.command),
                    );
                }
                return Ok(Some((first_index, second_index, *shared)));
            }
        }
        Ok(None)
    }
}

fn overlaps(first: &[Context], second: &[Context]) -> bool {
    first.is_empty() || second.is_empty() || first.iter().any(|pane| second.contains(pane))
}

/// How the popup spells a set of codes, `/`-joined for a multi-key bind.
fn keys_label(codes: &[KeyCode]) -> String {
    codes
        .iter()
        .map(|code| label(*code))
        .collect::<Vec<_>>()
        .join("/")
}

fn label(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Up => "↑".to_owned(),
        KeyCode::Down => "↓".to_owned(),
        KeyCode::Left => "←".to_owned(),
        KeyCode::Right => "→".to_owned(),
        other => spec(other),
    }
}

/// How the config file spells one code — the inverse of the parser, so a
/// dumped keymap reloads to itself.
fn spec(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Esc => "Esc".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::BackTab => "BackTab".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Delete => "Delete".to_owned(),
        KeyCode::Insert => "Insert".to_owned(),
        KeyCode::Up => "Up".to_owned(),
        KeyCode::Down => "Down".to_owned(),
        KeyCode::Left => "Left".to_owned(),
        KeyCode::Right => "Right".to_owned(),
        KeyCode::Home => "Home".to_owned(),
        KeyCode::End => "End".to_owned(),
        KeyCode::PageUp => "PgUp".to_owned(),
        KeyCode::PageDown => "PgDn".to_owned(),
        KeyCode::F(n) => format!("F{n}"),
        _ => "?".to_owned(),
    }
}

mod render;

#[cfg(test)]
mod tests;
