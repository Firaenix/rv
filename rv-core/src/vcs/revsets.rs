//! Revision syntax and the settings the revsets resolve under.
//!
//! Everything here is built in process from jj-lib's compiled-in defaults, so
//! `rv` resolves the same revsets on every machine.

use std::sync::Arc;

use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::revset::RemoteRefSymbolExpression;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::settings::UserSettings;
use jj_lib::str_util::StringExpression;

use super::WORKING_COPY;
use super::errors::Error;
use super::errors::chain;

/// How many generations back from the working copy a revision points, for the one
/// piece of revset *syntax* `rv` accepts: `@` is `Some(0)`, `@-` is `Some(1)`,
/// `@--` is `Some(2)`. Everything else is `None` and is looked up as a symbol.
///
/// `rv` resolves revisions itself instead of running jj's parser, which would need
/// the user's alias table; `@-` is idiomatic enough in jj to be worth the four
/// lines.
pub(super) fn working_copy_generations(revision: &str) -> Option<usize> {
    let ancestors = revision.strip_prefix(WORKING_COPY)?;
    if ancestors.bytes().all(|byte| byte == b'-') {
        Some(ancestors.len())
    } else {
        None
    }
}

/// Settings built entirely in process. Deliberately never reads a config file.
pub(super) fn settings() -> Result<UserSettings, Error> {
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
pub(super) fn trunk_expression() -> Arc<UserRevsetExpression> {
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
