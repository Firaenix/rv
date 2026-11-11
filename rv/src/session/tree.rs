//! Assembling a tree review: the files at one revision, each against itself.

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use rv_core::model::ChangeKind;
use rv_core::model::FileChange;
use rv_core::store::Session;
use rv_core::store::Store;
use rv_core::vcs::Repository;

use super::DEFAULT_HEAD;
use super::Review;
use super::ReviewKind;
use super::kept_start;

/// Opens the tree review of `head` (the working copy by default), listing only
/// the files under `subpath` when one is given, and records its session.
pub fn browse(repo_root: &Path, head: Option<&str>, subpath: Option<&str>) -> Result<Review> {
    let review = resolve(repo_root, head, subpath)?;
    review
        .store
        .write_review(&review.session)
        .context("could not write .review/session.toml")?;
    Ok(review)
}

/// The tree review of `head`, over the whole tree, writing nothing — what the
/// subcommands run with `--tree` read.
pub fn read_tree(repo_root: &Path, head: Option<&str>) -> Result<Review> {
    resolve(repo_root, head, None)
}

fn resolve(repo_root: &Path, head: Option<&str>, subpath: Option<&str>) -> Result<Review> {
    let repo = Repository::open(repo_root)?;
    let head_commit = repo.resolve(head)?;
    let owner = repo
        .change_of(&head_commit)
        .context("could not read the change under review")?;
    let subpath = subpath
        .map(normalized)
        .filter(|subpath| !subpath.is_empty() && subpath != ".");
    let files: Vec<FileChange> = repo
        .tree(&head_commit)
        .context("could not list the tree")?
        .into_iter()
        .filter(|entry| {
            subpath
                .as_deref()
                .is_none_or(|under| is_under(&entry.path, under))
        })
        .map(|entry| FileChange {
            path: entry.path,
            source_path: None,
            kind: ChangeKind::Unchanged,
            binary: false,
        })
        .collect();
    let revset = format!("tree({})", head.unwrap_or(DEFAULT_HEAD));
    if files.is_empty() {
        match &subpath {
            Some(under) => anyhow::bail!("{revset} has no files under {under}"),
            None => anyhow::bail!("{revset} has no files to browse"),
        }
    }

    let store = Store::open(repo_root, &rv_core::store::tree_review_key(head))
        .with_context(|| format!("could not open {}/.review", repo_root.display()))?;
    store
        .ensure_excluded()
        .context("could not add /.review/ to .git/info/exclude")?;
    let stored = store
        .read_review()
        .context("could not read .review/session.toml")?;

    let session = Session {
        started_at: kept_start(&stored, &revset),
        revset,
        base_commit: head_commit.clone(),
        head_commit,
        changes: Vec::new(),
        comments: stored.comments,
        flags: stored.flags,
        reviewed: stored.reviewed,
        agent_session: stored.agent_session,
    };
    Ok(Review {
        repo,
        store,
        session,
        files,
        asked: (None, head.map(str::to_owned)),
        kind: ReviewKind::Tree {
            head: owner,
            subpath,
        },
    })
}

fn normalized(subpath: &str) -> String {
    subpath
        .trim_start_matches("./")
        .trim_matches('/')
        .to_owned()
}

fn is_under(path: &str, directory_or_file: &str) -> bool {
    path == directory_or_file
        || path
            .strip_prefix(directory_or_file)
            .is_some_and(|rest| rest.starts_with('/'))
}
