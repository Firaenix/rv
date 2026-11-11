//! Which directory under `.review/reviews/` a review lives in.

/// The identity a review is stored under — what lets two branches be
/// reviewed on one repo without clobbering each other.
///
/// The asked head's own name where it has one: a bookmark survives amends
/// and rebases, which is exactly the lifetime a review's notes need. The
/// anonymous working copy gets the one ambient `working-copy` review — `@`
/// is wherever you are, and a review of "wherever I am" cannot be pinned to
/// a branch without inventing an identity `jj` does not have. Sanitized to
/// one path segment, so a revset expression asked as the head cannot escape
/// `.review/reviews/`.
#[must_use]
pub fn review_key(asked_head: Option<&str>) -> String {
    let raw = match asked_head {
        Some(name) if name != "@" => name,
        _ => return "working-copy".to_owned(),
    };
    let cleaned: String = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_start_matches(['.', '-']).to_owned();
    if cleaned.is_empty() {
        "review".to_owned()
    } else {
        cleaned
    }
}

/// The key of the tree review at `asked_head`: `@tree` for the working copy,
/// `@tree-<head>` otherwise. [`review_key`] turns every `@` into `-`, so no
/// range review — not even one of a bookmark called `tree` — can share it.
#[must_use]
pub fn tree_review_key(asked_head: Option<&str>) -> String {
    match asked_head {
        Some(name) if name != "@" => format!("@tree-{}", review_key(Some(name))),
        _ => "@tree".to_owned(),
    }
}
