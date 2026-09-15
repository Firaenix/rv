//! Saving a flag: the same one construction path [`super::comments`] gives a
//! comment, so the TUI and `rv flag` cannot anchor differently.

use anyhow::Context as _;
use anyhow::Result;
use rv_core::anchor;
use rv_core::model::Side;
use rv_core::store::Flag;

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
