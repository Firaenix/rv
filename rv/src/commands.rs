//! What each subcommand does, once the arguments are parsed.

mod configcmd;
mod diffcmd;
mod report;

pub use configcmd::edit_config;
pub use configcmd::print_keymap;
pub use diffcmd::diff;
pub use report::check;
pub use report::comments;
pub use report::status;

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use rv::session;
use rv::session::Review;
use rv_core::model::Side;
use rv_core::store::CommentState;
use rv_core::store::SettledBy;

/// Settles `id` into `state` — or back to open where it is already there,
/// which is what makes the command its own undo. A polling worker sees the
/// change through `rv status` or `rv comments`; the export is not refreshed,
/// because nothing reads it back.
pub fn settle(review: &Review, id: &str, state: CommentState, by: SettledBy) -> Result<()> {
    // The *store*, not the range-filtered view: `.review/` outlives any one
    // revset, and a worker must be able to tick off a comment whose file has
    // left the range it is currently standing in.
    let comments = review
        .store
        .comments()
        .context("could not read the review's comments")?;
    let comment = comments
        .iter()
        .find(|comment| comment.id == id)
        .with_context(|| format!("no comment {id} in this review — ids are in `rv comments`"))?;
    let (target, said) = if comment.state == state {
        (CommentState::Open, "reopened")
    } else {
        match state {
            CommentState::Resolved => (state, "resolved"),
            _ => (state, "abandoned"),
        }
    };
    review.store.settle_comment(id, target, by)?;
    println!(
        "{said} {id} at {}:{}",
        comment.anchor.file, comment.anchor.line
    );
    Ok(())
}

/// `rv reply`: stores an answer through [`session::reply`] and says where it
/// landed.
pub fn reply(review: &Review, id: &str, body: &str) -> Result<()> {
    let comment = session::reply(review, id, body)?;
    println!(
        "replied to {id} at {}:{}",
        comment.anchor.file, comment.anchor.line
    );
    Ok(())
}

/// `--by` as clap sees it.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ByArg {
    Agent,
    User,
}

impl From<ByArg> for SettledBy {
    fn from(by: ByArg) -> Self {
        match by {
            ByArg::Agent => SettledBy::Agent,
            ByArg::User => SettledBy::User,
        }
    }
}

/// `--side` as clap sees it.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum SideArg {
    Left,
    Right,
}

impl From<SideArg> for Side {
    fn from(side: SideArg) -> Self {
        match side {
            SideArg::Left => Side::Left,
            SideArg::Right => Side::Right,
        }
    }
}

/// `rv render`: the review as markdown, to stdout — a projection for reading,
/// which nothing reads back, so where it lands is the caller's business.
///
/// `--out <path>` writes a file instead, for whoever wants an artefact to
/// attach or archive; the file carries no round-trip duty.
pub fn render(review: &Review, out: Option<&Path>) -> Result<()> {
    match out {
        None => {
            print!("{}", session::render_markdown(review)?);
            Ok(())
        }
        Some(path) if path == review.store.markdown_path() => {
            // The store's own path takes the store's atomic write.
            session::write_markdown(review)?;
            println!("wrote {}", path.display());
            Ok(())
        }
        Some(path) => {
            std::fs::write(path, session::render_markdown(review)?)
                .with_context(|| format!("could not write {}", path.display()))?;
            println!("wrote {}", path.display());
            Ok(())
        }
    }
}
