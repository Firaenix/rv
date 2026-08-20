//! jj access, in process, read only.
//!
//! This is the only module in `rv` allowed to import `jj_lib`; everything else
//! consumes the plain types in [`crate::model`]. Two rules shape the code here:
//!
//! * **Nothing is read from the user's jj config.** [`revsets::settings`] builds
//!   a `StackedConfig` from jj-lib's compiled-in defaults plus one literal layer,
//!   so `rv` resolves the same revsets on every machine. `trunk()` is a jj-*cli*
//!   alias that jj-lib does not ship, so [`revsets::trunk_expression`] rebuilds
//!   vanilla jj's definition with typed constructors instead of an alias table.
//! * **Nothing is mutated.** No transaction is started, no working copy is
//!   snapshotted, nothing under `.jj/` is written.

mod errors;
mod revsets;

use std::path::Path;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::io::AsyncReadExt as _;
use jj_lib::backend::CommitId;
use jj_lib::backend::FileId;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::copies::CopyRecords;
use jj_lib::default_backend_factories::default_backend_factories;
use jj_lib::default_backend_factories::default_working_copy_factories;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merge::Diff;
use jj_lib::merge::MergedTreeValue;
use jj_lib::object_id::ObjectId as _;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::revset::Revset;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::SymbolResolver;
use jj_lib::revset::SymbolResolverExtension;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::workspace::Workspace;

pub use errors::Error;
pub use errors::LINKED_JJ_LIB;
use errors::chain;
use errors::load_error;
use revsets::settings;
use revsets::trunk_expression;
use revsets::working_copy_generations;

use crate::model::ChangeKind;
use crate::model::ChangeRef;
use crate::model::FileChange;

/// The revision a caller gets when it does not name a base.
const DEFAULT_BASE: &str = "trunk()";
/// The revision a caller gets when it does not name a head. jj's *parser*, not its
/// symbol resolver, is what understands `@`, so `rv` has to translate it itself.
const WORKING_COPY: &str = "@";
/// How much of a file is inspected before calling it binary. Matches git's own
/// sniffing window.
const BINARY_SNIFF_BYTES: u64 = 8192;

/// A read-only handle on a jj workspace.
pub struct Repository {
    repo: Arc<ReadonlyRepo>,
    workspace_name: WorkspaceNameBuf,
}

impl Repository {
    /// Loads the workspace rooted at `path` at its head operation.
    ///
    /// `path` must be the workspace root — the directory holding `.jj/` — as
    /// jj-lib does not search ancestors; anything else is [`Error::NotAWorkspace`].
    pub fn open(path: &Path) -> Result<Self, Error> {
        let settings = settings()?;
        let workspace = Workspace::load(
            &settings,
            path,
            &default_backend_factories(),
            &default_working_copy_factories(),
        )
        .map_err(|error| load_error(path, error))?;
        let workspace_name = workspace.workspace_name().to_owned();
        let repo = pollster::block_on(workspace.repo_loader().load_at_head())
            .map_err(|error| Error::Jj(chain(&error)))?;
        Ok(Self {
            repo,
            workspace_name,
        })
    }

    /// The changes in `base..head`, newest first, excluding `base` itself.
    ///
    /// `base` defaults to `trunk()` and `head` to the working-copy change, `@`.
    /// Each endpoint is a revision *symbol* — a bookmark, change id or commit id —
    /// or `@` and its `@-` ancestor shorthand, but never a revset expression: `rv`
    /// resolves revisions itself rather than running jj's parser, which would need
    /// the user's alias table.
    ///
    /// An empty range is an error rather than an empty stack: there is nothing to
    /// review, and saying so beats rendering a blank screen.
    pub fn stack(&self, base: Option<&str>, head: Option<&str>) -> Result<Vec<ChangeRef>, Error> {
        let base_expression = self.parse_or_default(base, trunk_expression());
        let head_expression = self.parse_or_default(head, self.working_copy_expression());
        let changes = self.evaluate(&base_expression.range(&head_expression))?;
        if changes.is_empty() {
            return Err(Error::EmptyRange {
                base: base.unwrap_or(DEFAULT_BASE).to_owned(),
                head: head.unwrap_or(WORKING_COPY).to_owned(),
            });
        }
        Ok(changes)
    }

    /// The two commits a review runs between, as hex commit ids: `(base, head)`.
    ///
    /// The endpoints of [`Repository::stack`] pinned to concrete commits, so that
    /// the rest of `rv` can name a revision without holding a jj type. They are
    /// commit ids by design: a diff is between snapshots, whereas a change id
    /// denotes whatever the change was most recently rewritten into.
    ///
    /// A revision that names more than one commit — `@-` of a merge, say — pins the
    /// first of them in topological order.
    pub fn endpoints(
        &self,
        base: Option<&str>,
        head: Option<&str>,
    ) -> Result<(String, String), Error> {
        let base_revision = base.unwrap_or(DEFAULT_BASE);
        let head_revision = head.unwrap_or(WORKING_COPY);
        let base_id = self.single_commit(
            base_revision,
            &self.parse_or_default(base, trunk_expression()),
        )?;
        let head_id = self.single_commit(
            head_revision,
            &self.parse_or_default(head, self.working_copy_expression()),
        )?;
        Ok((base_id.hex(), head_id.hex()))
    }

    /// Every file that differs between the two commits, sorted by head-side path.
    ///
    /// Renames come from jj's own copy records rather than from a similarity guess
    /// of `rv`'s own, so a moved file stays one entry with its `source_path` set.
    /// `binary` marks a file whose contents `rv` will not try to read as text.
    pub fn files(&self, base_commit: &str, head_commit: &str) -> Result<Vec<FileChange>, Error> {
        let base = self.commit_by_hex(base_commit)?;
        let head = self.commit_by_hex(head_commit)?;
        let base_tree = base.tree();
        let head_tree = head.tree();
        let copy_records = self.copy_records(base.id(), head.id())?;

        let mut changes = Vec::new();
        let mut entries =
            base_tree.diff_stream_with_copies(&head_tree, &EverythingMatcher, &copy_records);
        while let Some(entry) = pollster::block_on(entries.next()) {
            let target = entry.path.target;
            // A copy record whose source is the target itself is not a rename.
            let source = entry
                .path
                .source
                .map(|(path, _operation)| path)
                .filter(|path| *path != target);
            let Diff { before, after } = entry.values.map_err(|error| Error::Jj(chain(&error)))?;

            let kind = if after.is_absent() {
                ChangeKind::Removed
            } else if source.is_some() {
                ChangeKind::Renamed
            } else if before.is_absent() {
                ChangeKind::Added
            } else {
                ChangeKind::Modified
            };
            // Sniff the side that still exists: a removed file only has a base one.
            // jj folds a rename's delete entry into the rename itself, so a removal
            // is always a removal of the target path.
            let side = if after.is_absent() { &before } else { &after };
            let binary = self.value_looks_binary(&target, side)?;

            changes.push(FileChange {
                path: target.as_internal_file_string().to_owned(),
                source_path: source.map(|path| path.as_internal_file_string().to_owned()),
                kind,
                binary,
            });
        }
        changes.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(changes)
    }

    /// The bytes of `path` at `commit_id`, or `None` when the commit has no plain
    /// file there — a path that is absent, a directory, a symlink or conflicted is
    /// simply nothing to diff, not an error.
    pub fn read_blob(&self, commit_id: &str, path: &str) -> Result<Option<Vec<u8>>, Error> {
        let commit = self.commit_by_hex(commit_id)?;
        let repo_path =
            RepoPathBuf::from_internal_string(path).map_err(|error| Error::Jj(chain(&error)))?;
        let value = pollster::block_on(commit.tree().path_value(&repo_path))
            .map_err(|error| Error::Jj(chain(&error)))?;
        let Some(TreeValue::File { id, .. }) = value.as_normal() else {
            return Ok(None);
        };
        self.read_file(&repo_path, id, u64::MAX).map(Some)
    }

    /// Whether `path` is tracked: present in the working-copy commit's tree.
    ///
    /// jj has no index, so a path is tracked exactly when the working copy's
    /// commit carries it. `path` may name a directory as readily as a file —
    /// which is the point, since the caller asking is `rv status` about
    /// `.review/` — so this deliberately accepts every kind of tree entry,
    /// including a conflicted one. Only absence is untracked.
    pub fn tracks(&self, path: &str) -> Result<bool, Error> {
        let Some(commit_id) = self.repo.view().get_wc_commit_id(&self.workspace_name) else {
            return Ok(false);
        };
        let commit = pollster::block_on(self.repo.store().get_commit_async(commit_id))
            .map_err(|error| Error::Jj(chain(&error)))?;
        let repo_path =
            RepoPathBuf::from_internal_string(path).map_err(|error| Error::Jj(chain(&error)))?;
        let value = pollster::block_on(commit.tree().path_value(&repo_path))
            .map_err(|error| Error::Jj(chain(&error)))?;
        Ok(!value.is_absent())
    }

    /// Resolves and evaluates `expression`, newest change first.
    fn evaluate(&self, expression: &Arc<UserRevsetExpression>) -> Result<Vec<ChangeRef>, Error> {
        let revset = self.evaluate_revset(expression)?;
        // Revsets are async streams, and they stream in topological order with
        // children before parents — which is the newest-first order we want.
        let ids: Vec<_> = pollster::block_on(revset.commit_change_ids().try_collect())
            .map_err(|error| Error::Jj(chain(&error)))?;

        ids.into_iter()
            .map(|(commit_id, change_id)| {
                let commit = pollster::block_on(self.repo.store().get_commit_async(&commit_id))
                    .map_err(|error| Error::Jj(chain(&error)))?;
                Ok(ChangeRef {
                    change_id: change_id.reverse_hex(),
                    commit_id: commit_id.hex(),
                    // jj stores descriptions with a trailing newline.
                    description: commit.description().trim_end().to_owned(),
                })
            })
            .collect()
    }

    /// Resolves symbols in `expression` and evaluates it against the loaded
    /// operation.
    fn evaluate_revset(
        &self,
        expression: &Arc<UserRevsetExpression>,
    ) -> Result<Box<dyn Revset + '_>, Error> {
        let repo = self.repo.as_ref();
        let extensions: &[Box<dyn SymbolResolverExtension>] = &[];
        let resolver = SymbolResolver::new(repo, extensions);
        let resolved = expression
            .resolve_user_expression(repo, &resolver)
            .map_err(|error| Error::Unresolved(chain(&error)))?;
        RevsetExpression::evaluate(resolved, repo).map_err(|error| Error::Jj(chain(&error)))
    }

    /// The first commit `expression` evaluates to. `revision` is the text the user
    /// wrote, used only for the error message.
    fn single_commit(
        &self,
        revision: &str,
        expression: &Arc<UserRevsetExpression>,
    ) -> Result<CommitId, Error> {
        let revset = self.evaluate_revset(expression)?;
        let mut ids = revset.stream();
        match pollster::block_on(ids.next()) {
            Some(id) => id.map_err(|error| Error::Jj(chain(&error))),
            None => Err(Error::Unresolved(revision.to_owned())),
        }
    }

    /// Looks up the commit a hex id names, as produced by
    /// [`Repository::endpoints`].
    fn commit_by_hex(&self, hex: &str) -> Result<Commit, Error> {
        let id = CommitId::try_from_hex(hex).ok_or_else(|| Error::Unresolved(hex.to_owned()))?;
        pollster::block_on(self.repo.store().get_commit_async(&id))
            .map_err(|error| Error::Jj(chain(&error)))
    }

    /// jj's record of what was copied or renamed between the two commits. jj
    /// tracks this natively, so `rv` never guesses at renames itself.
    fn copy_records(&self, base: &CommitId, head: &CommitId) -> Result<CopyRecords, Error> {
        let stream = self
            .repo
            .store()
            .get_copy_records(None, base, head)
            .map_err(|error| Error::Jj(chain(&error)))?;
        let records: Vec<_> =
            pollster::block_on(stream.try_collect()).map_err(|error| Error::Jj(chain(&error)))?;
        let mut copy_records = CopyRecords::default();
        copy_records.add_records(records);
        Ok(copy_records)
    }

    /// Whether the file at `path` should be treated as binary, i.e. whether a NUL
    /// byte turns up in its first [`BINARY_SNIFF_BYTES`] bytes. Anything that is
    /// not a plain file has no text for `rv` to show either way.
    fn value_looks_binary(&self, path: &RepoPath, value: &MergedTreeValue) -> Result<bool, Error> {
        let Some(TreeValue::File { id, .. }) = value.as_normal() else {
            return Ok(false);
        };
        let head = self.read_file(path, id, BINARY_SNIFF_BYTES)?;
        Ok(head.contains(&0))
    }

    /// Reads at most `limit` bytes of the blob `id`; pass [`u64::MAX`] for all of
    /// it.
    fn read_file(&self, path: &RepoPath, id: &FileId, limit: u64) -> Result<Vec<u8>, Error> {
        let reader = pollster::block_on(self.repo.store().read_file(path, id))
            .map_err(|error| Error::Jj(chain(&error)))?;
        let mut contents = Vec::new();
        pollster::block_on(reader.take(limit).read_to_end(&mut contents))
            .map_err(|error| Error::Jj(chain(&error)))?;
        Ok(contents)
    }

    /// Turns a user-supplied revision into an expression, falling back to
    /// `default` when none was given.
    fn parse_or_default(
        &self,
        revision: Option<&str>,
        default: Arc<UserRevsetExpression>,
    ) -> Arc<UserRevsetExpression> {
        let Some(revision) = revision else {
            return default;
        };
        match working_copy_generations(revision) {
            Some(generations) => {
                let mut expression = self.working_copy_expression();
                for _ in 0..generations {
                    expression = expression.parents();
                }
                expression
            }
            None => RevsetExpression::symbol(revision.to_owned()),
        }
    }

    fn working_copy_expression(&self) -> Arc<UserRevsetExpression> {
        RevsetExpression::working_copy(self.workspace_name.clone())
    }
}
