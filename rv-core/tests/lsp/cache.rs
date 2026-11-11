use std::path::{Path, PathBuf};

use rv_core::lsp::{Answer, AnswerCache, CacheKey, Location, RequestKind};

fn key(path: &str, kind: RequestKind) -> CacheKey {
    CacheKey {
        path: PathBuf::from(path),
        line: 3,
        column: 4,
        kind,
    }
}

fn locations(line: u32) -> Answer {
    Answer::Locations(vec![Location {
        path: PathBuf::from("src/lib.rs"),
        line,
        column: 0,
    }])
}

#[test]
fn answer_is_returned_while_hash_matches() {
    let mut cache = AnswerCache::default();
    cache.insert(key("a.rs", RequestKind::Definition), "h1", locations(9));
    assert_eq!(
        cache.get(&key("a.rs", RequestKind::Definition), "h1"),
        Some(&locations(9))
    );
    assert_eq!(cache.get(&key("a.rs", RequestKind::References), "h1"), None);
}

#[test]
fn changed_hash_misses_and_new_insert_drops_old_answers() {
    let mut cache = AnswerCache::default();
    cache.insert(key("a.rs", RequestKind::Definition), "h1", locations(9));
    cache.insert(key("a.rs", RequestKind::Hover), "h1", Answer::Hover(None));
    assert_eq!(cache.get(&key("a.rs", RequestKind::Definition), "h2"), None);
    cache.insert(key("a.rs", RequestKind::References), "h2", locations(1));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get(&key("a.rs", RequestKind::Hover), "h1"), None);
}

#[test]
fn invalidating_one_file_keeps_others() {
    let mut cache = AnswerCache::default();
    cache.insert(key("a.rs", RequestKind::Definition), "h", locations(1));
    cache.insert(key("b.rs", RequestKind::Definition), "h", locations(2));
    cache.invalidate(Path::new("a.rs"));
    assert_eq!(cache.get(&key("a.rs", RequestKind::Definition), "h"), None);
    assert_eq!(
        cache.get(&key("b.rs", RequestKind::Definition), "h"),
        Some(&locations(2))
    );
    cache.clear();
    assert!(cache.is_empty());
}
