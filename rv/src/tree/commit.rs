//! How a change's row reads: its two short ids, the prefix that selects it,
//! and its subject.

use super::Group;
use super::ID_SHORT;
use super::NO_DESCRIPTION;

/// `change_id subject`, with the subject being the first line of the
/// description — a change's row is one row, and jj descriptions are written
/// with that convention already.
pub(super) fn commit_label(group: &Group<'_>) -> String {
    format!(
        "{} {} {}",
        short(group.change_id),
        short(group.commit_id),
        subject_of(group)
    )
}

/// A change's subject: the first line of its description, or a stand-in.
pub(super) fn subject_of(group: &Group<'_>) -> String {
    let subject = group.description.lines().next().unwrap_or_default().trim();
    if subject.is_empty() {
        NO_DESCRIPTION.to_owned()
    } else {
        subject.to_owned()
    }
}

/// The first [`ID_SHORT`] characters of an id, or all of it where it is shorter.
pub(super) fn short(id: &str) -> String {
    id.chars().take(ID_SHORT).collect()
}

/// How many leading characters of `id` no other id in `all` shares.
///
/// At least one, so a lone change still shows a highlighted character: the
/// highlight means "this is what you type", and typing nothing selects nothing.
/// Never more than [`ID_SHORT`] — a prefix longer than the row prints could not
/// be highlighted on screen anyway, and two ids agreeing that far are not going
/// to be told apart by this row.
pub(super) fn unique_prefix(id: &str, all: &[&str]) -> usize {
    let others: Vec<&&str> = all.iter().filter(|other| **other != id).collect();
    (1..=ID_SHORT)
        .find(|length| {
            let prefix: String = id.chars().take(*length).collect();
            !others
                .iter()
                .any(|other| other.starts_with(prefix.as_str()))
        })
        .unwrap_or(ID_SHORT)
}
