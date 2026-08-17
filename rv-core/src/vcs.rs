//! jj access, in process, read only.
//!
//! This is the only module in `rv` allowed to import `jj_lib`; everything else
//! consumes the plain types in [`crate::model`]. Two rules shape the code here:
//!
//! * **Nothing is read from the user's jj config.** `settings` builds a
//!   [`StackedConfig`] from jj-lib's compiled-in defaults plus one literal layer,
//!   so `rv` resolves the same revsets on every machine. `trunk()` is a jj-*cli*
//!   alias that jj-lib does not ship, so `trunk_expression` rebuilds vanilla jj's
//!   definition with typed constructors instead of an alias table.
//! * **Nothing is mutated.** No transaction is started, no working copy is
//!   snapshotted, nothing under `.jj/` is written.

use std::path::Path;
use std::sync::Arc;

use futures::TryStreamExt as _;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::default_backend_factories::default_backend_factories;
use jj_lib::default_backend_factories::default_working_copy_factories;
use jj_lib::object_id::ObjectId as _;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo::StoreLoadError;
use jj_lib::revset::RemoteRefSymbolExpression;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::SymbolResolver;
use jj_lib::revset::SymbolResolverExtension;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::settings::UserSettings;
use jj_lib::str_util::StringExpression;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::WorkspaceLoadError;

use crate::model::ChangeRef;

/// The jj-lib version `rv` is built against. Reported when a repository turns out
/// to be written in a format this build cannot read.
pub const LINKED_JJ_LIB: &str = "0.44";

/// The revision a caller gets when it does not name a base.
const DEFAULT_BASE: &str = "trunk()";
/// The revision a caller gets when it does not name a head. jj's *parser*, not its
/// symbol resolver, is what understands `@`, so `rv` has to translate it itself.
const WORKING_COPY: &str = "@";

/// Errors from talking to jj.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no jj workspace at {0} (run `jj git init --colocate` there?)")]
    NotAWorkspace(String),
    #[error(
        "this repository is not readable by jj-lib {linked}, which rv is built against: \
         {source_message}"
    )]
    Incompatible {
        linked: String,
        source_message: String,
    },
    #[error("unresolved revset: {0}")]
    Unresolved(String),
    #[error("revset {base}..{head} is empty: there is nothing to review")]
    EmptyRange { base: String, head: String },
    #[error("jj: {0}")]
    Jj(String),
}

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
    /// Each endpoint is a revision *symbol* — a bookmark, change id or commit id,
    /// or `@` — not a revset expression: `rv` resolves symbols itself rather than
    /// running jj's parser, which would need the user's alias table.
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

    /// Resolves and evaluates `expression`, newest change first.
    fn evaluate(&self, expression: &Arc<UserRevsetExpression>) -> Result<Vec<ChangeRef>, Error> {
        let repo = self.repo.as_ref();
        let extensions: &[Box<dyn SymbolResolverExtension>] = &[];
        let resolver = SymbolResolver::new(repo, extensions);
        let resolved = expression
            .resolve_user_expression(repo, &resolver)
            .map_err(|error| Error::Unresolved(chain(&error)))?;
        let revset = RevsetExpression::evaluate(resolved, repo)
            .map_err(|error| Error::Jj(chain(&error)))?;
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

    /// Turns a user-supplied revision into an expression, falling back to
    /// `default` when none was given.
    fn parse_or_default(
        &self,
        revision: Option<&str>,
        default: Arc<UserRevsetExpression>,
    ) -> Arc<UserRevsetExpression> {
        match revision {
            None => default,
            Some(WORKING_COPY) => self.working_copy_expression(),
            Some(symbol) => RevsetExpression::symbol(symbol.to_owned()),
        }
    }

    fn working_copy_expression(&self) -> Arc<UserRevsetExpression> {
        RevsetExpression::working_copy(self.workspace_name.clone())
    }
}

/// Settings built entirely in process. Deliberately never reads a config file.
fn settings() -> Result<UserSettings, Error> {
    let mut config = StackedConfig::with_defaults();
    let layer = ConfigLayer::parse(
        ConfigSource::Default,
        "user.name = \"rv\"\nuser.email = \"rv@localhost\"\n",
    )
    .map_err(|error| Error::Jj(chain(&error)))?;
    config.add_layer(layer);
    UserSettings::from_config(config).map_err(|error| Error::Jj(chain(&error)))
}

/// Vanilla jj's `trunk()`, built with typed constructors — no alias table and no
/// config. It always resolves, degrading to `root()` when no remote is present.
fn trunk_expression() -> Arc<UserRevsetExpression> {
    let mut candidates = Vec::new();
    for remote in ["origin", "upstream"] {
        for name in ["main", "master", "trunk"] {
            candidates.push(RevsetExpression::remote_bookmarks(
                RemoteRefSymbolExpression {
                    name: StringExpression::exact(name),
                    remote: StringExpression::exact(remote),
                },
                None,
            ));
        }
    }
    candidates.push(RevsetExpression::root());
    RevsetExpression::union_all(&candidates).latest(1)
}

fn load_error(path: &Path, error: WorkspaceLoadError) -> Error {
    match &error {
        WorkspaceLoadError::NoWorkspaceHere(at) | WorkspaceLoadError::RepoDoesNotExist(at) => {
            Error::NotAWorkspace(at.display().to_string())
        }
        // An unknown backend or op-store type means the repo was written by a jj
        // that knows a format this build does not.
        WorkspaceLoadError::StoreLoadError(StoreLoadError::UnsupportedType { .. }) => {
            Error::Incompatible {
                linked: LINKED_JJ_LIB.to_owned(),
                source_message: chain(&error),
            }
        }
        _ => Error::Jj(format!("{}: {}", path.display(), chain(&error))),
    }
}

/// Renders an error together with its `source` chain: jj-lib's outer messages are
/// often generic ("Cannot read the repo") and the cause carries the detail.
fn chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut previous = message.clone();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        // Skip `#[error(transparent)]` links, which repeat their cause verbatim.
        if text != previous {
            message.push_str(": ");
            message.push_str(&text);
        }
        previous = text;
        source = cause.source();
    }
    message
}
