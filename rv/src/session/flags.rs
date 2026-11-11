//! Saving a flag: the same one construction path [`super::comments`] gives a
//! comment, so the TUI and `rv flag` cannot anchor differently.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::agent::Focus;
use rv_core::anchor;
use rv_core::model::Side;
use rv_core::store::Flag;
use rv_core::store::FlaggedBy;

use super::Review;
use super::comments::owning_change;

pub fn save_flag(
    review: &Review,
    path: &str,
    side: Side,
    line: u32,
    commit: &str,
    reason: &str,
) -> Result<Flag> {
    save_flag_by(review, path, side, line, commit, reason, FlaggedBy::User)
}

fn save_flag_by(
    review: &Review,
    path: &str,
    side: Side,
    line: u32,
    commit: &str,
    reason: &str,
    by: FlaggedBy,
) -> Result<Flag> {
    let reason = reason.trim();
    if reason.is_empty() {
        anyhow::bail!("a flag with no reason points at nothing — nothing saved");
    }
    let change = owning_change(review, path)?;
    let blob = review
        .repo
        .read_blob(commit, path)
        .with_context(|| format!("could not read {path} to anchor the flag"))?;
    let text = blob.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

    let flag = Flag {
        id: crate::app::comment_id(&change.change_id, path, side, line, reason),
        change_id: change.change_id.clone(),
        commit_id: commit.to_owned(),
        anchor: anchor::create(path, side, line, text.as_deref().unwrap_or_default()),
        reason: reason.to_owned(),
        acknowledged: false,
        by,
    };
    review
        .store
        .append_flag(&flag)
        .context("could not save the flag")?;
    Ok(flag)
}

/// `rv flag`: resolves the CLI's arguments to a side-specific location and
/// saves through [`save_flag`].
pub fn add_flag(review: &Review, path: &str, side: Side, line: u32, reason: &str) -> Result<Flag> {
    let (anchored_path, commit) = super::comments::locate(review, path, side, line)?;
    save_flag(review, &anchored_path, side, line, &commit, reason)
}

#[derive(Debug, Default)]
pub struct FocusSaved {
    pub saved: Vec<Flag>,
    pub already_flagged: usize,
    /// Each entry the review could not anchor, with why: a path outside the
    /// review, a line past the file's end.
    pub skipped: Vec<(Focus, String)>,
}

/// Files an agent's focus list as flags by `agent`. An entry already flagged
/// by the agent at the same path, line — as stored or as re-anchored after an
/// edit — and reason is not filed again, so re-asking never duplicates.
pub fn save_agent_focus(review: &Review, focus: &[Focus]) -> Result<FocusSaved> {
    let stored = review
        .store
        .flags()
        .context("could not read the review's flags")?;
    let mut placed = stored.clone();
    crate::stale::place_flags(review, &mut placed);
    let mut known: Vec<Flag> = stored.into_iter().chain(placed).collect();
    let mut outcome = FocusSaved::default();
    for entry in focus {
        let repeats = |flag: &Flag| {
            flag.by == FlaggedBy::Agent
                && flag.anchor.file == entry.path
                && flag.anchor.line == entry.line
                && flag.reason == entry.why
        };
        if known.iter().any(repeats) {
            outcome.already_flagged += 1;
            continue;
        }
        let (path, commit) =
            match super::comments::locate(review, &entry.path, Side::Right, entry.line) {
                Ok(located) => located,
                Err(error) => {
                    outcome.skipped.push((entry.clone(), format!("{error:#}")));
                    continue;
                }
            };
        let change = owning_change(review, &path)?;
        let id = crate::app::comment_id(
            &change.change_id,
            &path,
            Side::Right,
            entry.line,
            &entry.why,
        );
        if known.iter().any(|flag| flag.id == id) {
            outcome.already_flagged += 1;
            continue;
        }
        let flag = save_flag_by(
            review,
            &path,
            Side::Right,
            entry.line,
            &commit,
            &entry.why,
            FlaggedBy::Agent,
        )?;
        known.push(flag.clone());
        outcome.saved.push(flag);
    }
    Ok(outcome)
}

/// `rv ack`: marks a flag seen. An unknown id is an error, as for a reply:
/// the caller named something, and a silent no-op would hide the typo.
pub fn acknowledge(review: &Review, id: &str) -> Result<()> {
    if !review
        .store
        .acknowledge_flag(id, true)
        .with_context(|| format!("could not acknowledge flag {id}"))?
    {
        anyhow::bail!("no flag {id} in this review — ids are in `rv flags`");
    }
    Ok(())
}

pub fn in_range(review: &Review, flags: Vec<Flag>) -> Vec<Flag> {
    flags
        .into_iter()
        .filter(|flag| {
            review.files.iter().any(|file| {
                file.path == flag.anchor.file
                    || file.source_path.as_deref() == Some(flag.anchor.file.as_str())
            })
        })
        .collect()
}
