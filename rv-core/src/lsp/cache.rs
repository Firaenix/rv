use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Answer, RequestKind};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub kind: RequestKind,
}

struct CachedFile {
    content_hash: String,
    answers: HashMap<(u32, u32, RequestKind), Answer>,
}

/// Answers per position, valid only while the queried file's content hash is
/// unchanged. A different hash for a file drops every answer for that file.
/// An edit to *another* file can also move an answer's target; callers that
/// care should [`AnswerCache::clear`] when the working copy changes.
#[derive(Default)]
pub struct AnswerCache {
    files: HashMap<PathBuf, CachedFile>,
}

impl AnswerCache {
    pub fn get(&self, key: &CacheKey, content_hash: &str) -> Option<&Answer> {
        let file = self.files.get(&key.path)?;
        if file.content_hash != content_hash {
            return None;
        }
        file.answers.get(&(key.line, key.column, key.kind))
    }

    pub fn insert(&mut self, key: CacheKey, content_hash: &str, answer: Answer) {
        let file = self.files.entry(key.path).or_insert_with(|| CachedFile {
            content_hash: content_hash.to_owned(),
            answers: HashMap::new(),
        });
        if file.content_hash != content_hash {
            file.content_hash = content_hash.to_owned();
            file.answers.clear();
        }
        file.answers
            .insert((key.line, key.column, key.kind), answer);
    }

    pub fn invalidate(&mut self, path: &Path) {
        self.files.remove(path);
    }

    pub fn clear(&mut self) {
        self.files.clear();
    }

    pub fn len(&self) -> usize {
        self.files.values().map(|file| file.answers.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
