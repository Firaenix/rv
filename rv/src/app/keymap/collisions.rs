//! The collision policy: what happens when two rows claim one key.

use anyhow::Result;
use anyhow::bail;
use crossterm::event::KeyCode;

use super::ALL_PANES;
use super::Keymap;
use super::keys_label;
use super::spec;
use super::vocab;
use crate::app::Context;
use crate::app::Leader;

impl Keymap {
    /// Applies the collision policy: a user row and a default row claiming
    /// one key resolve by **steal** (the default yields, with an alert); two
    /// user rows claiming one key are a contradiction and an error.
    pub(super) fn settle_collisions(&mut self) -> Result<()> {
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
