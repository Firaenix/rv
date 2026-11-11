//! Line provenance: which change last touched each line of a file.

use std::collections::HashMap;

use jj_lib::annotate::FileAnnotator;
use jj_lib::backend::CommitId;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::revset::ResolvedRevsetExpression;

use super::BINARY_SNIFF_BYTES;
use super::Error;
use super::Repository;
use super::chain;

/// The change a line was last touched by. `change_id` is reverse hex, as in
/// [`crate::model::ChangeRef`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineOrigin {
    pub change_id: String,
    pub commit_id: String,
    pub author: String,
    pub timestamp_secs: i64,
    pub description_first_line: String,
}

/// Each distinct origin once, and per line an index into `origins`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    pub origins: Vec<LineOrigin>,
    /// `line_origins[i]` is the origin of line `i + 1`.
    pub line_origins: Vec<usize>,
}

impl Annotation {
    /// The origin of the 1-based `line`, or `None` past the end of the file.
    pub fn origin_of_line(&self, line: usize) -> Option<&LineOrigin> {
        let index = *self.line_origins.get(line.checked_sub(1)?)?;
        self.origins.get(index)
    }

    pub fn line_count(&self) -> usize {
        self.line_origins.len()
    }
}

impl Repository {
    /// Who last touched each line of `path` as it stands at `commit_id`,
    /// walking every ancestor of that commit.
    ///
    /// `None` when there is no text to annotate: the path is absent, not a
    /// plain file, or binary.
    pub fn annotate(&self, commit_id: &str, path: &str) -> Result<Option<Annotation>, Error> {
        let Some(contents) = self.read_blob(commit_id, path)? else {
            return Ok(None);
        };
        let sniffed = &contents[..contents.len().min(BINARY_SNIFF_BYTES as usize)];
        if sniffed.contains(&0) {
            return Ok(None);
        }

        let commit = self.commit_by_hex(commit_id)?;
        let repo_path =
            RepoPathBuf::from_internal_string(path).map_err(|error| Error::Jj(chain(&error)))?;
        let mut annotator = FileAnnotator::with_file_content(commit.id(), &repo_path, contents);
        // jj-lib already intersects the domain with the starting commit's
        // ancestors, so `all()` bounds the walk to exactly those.
        pollster::block_on(annotator.compute(self.repo.as_ref(), &ResolvedRevsetExpression::all()))
            .map_err(|error| Error::Jj(chain(&error)))?;

        let mut origins = Vec::new();
        let mut origin_index_by_commit: HashMap<CommitId, usize> = HashMap::new();
        let mut line_origins = Vec::new();
        // An `Err` origin is a line the walk stopped short of resolving; the
        // commit it names is still the furthest jj could trace it.
        for (origin, _line) in annotator.to_annotation().line_origins() {
            let origin_commit_id = &origin.unwrap_or_else(|unresolved| unresolved).commit_id;
            let index = match origin_index_by_commit.get(origin_commit_id) {
                Some(&index) => index,
                None => {
                    origins.push(self.line_origin(origin_commit_id)?);
                    origin_index_by_commit.insert(origin_commit_id.clone(), origins.len() - 1);
                    origins.len() - 1
                }
            };
            line_origins.push(index);
        }
        Ok(Some(Annotation {
            origins,
            line_origins,
        }))
    }

    fn line_origin(&self, commit_id: &CommitId) -> Result<LineOrigin, Error> {
        let commit = pollster::block_on(self.repo.store().get_commit_async(commit_id))
            .map_err(|error| Error::Jj(chain(&error)))?;
        let author = commit.author();
        Ok(LineOrigin {
            change_id: commit.change_id().reverse_hex(),
            commit_id: commit_id.hex(),
            author: author.name.clone(),
            timestamp_secs: author.timestamp.timestamp.0.div_euclid(1000),
            description_first_line: commit.description().lines().next().unwrap_or("").to_owned(),
        })
    }
}
