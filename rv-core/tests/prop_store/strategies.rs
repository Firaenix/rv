//! The generators: hostile text aimed at every serialization layer the store
//! crosses, the prefix-structured id pool, and the `Op` model a property
//! drives an arbitrary interleaving of every store write with.
//!
//! Split from [`super`] for the 400-line rule; it shares that file's fixtures.

use proptest::prelude::*;
use rv_core::model::Anchor;
use rv_core::model::ChangeRef;
use rv_core::model::Side;
use rv_core::store::Comment;
use rv_core::store::CommentState;
use rv_core::store::Session;
use rv_core::store::Store;
// ---------------------------------------------------------------------------
// strategies
// ---------------------------------------------------------------------------

/// Characters aimed at every serialization layer the store crosses: JSON
/// metacharacters, TOML metacharacters, C0 controls, the Unicode line and
/// paragraph separators, a BOM, and multi-byte / astral-plane scalars — salted
/// with fully arbitrary `char`s so the set is not just the ones I thought of.
pub(super) fn hostile_char() -> impl Strategy<Value = char> {
    prop_oneof![
        3 => any::<char>(),
        7 => prop::sample::select(vec![
            '"', '\\', '/', '\n', '\r', '\t', '\0', '\u{1}', '\u{1b}', '\u{7f}',
            '{', '}', '[', ']', ':', ',', '\'', '=', '#', '.', ' ',
            'é', 'ß', '中', '🙂', '\u{2028}', '\u{feff}',
        ]),
    ]
}

pub(super) fn hostile_text(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(hostile_char(), 0..max).prop_map(|chars| chars.into_iter().collect())
}

/// Hostile text with the line terminators removed. Anchor context lines come
/// from splitting file text into lines, so they never contain `\n` by
/// construction (see `anchor::create`); everything else about them is hostile.
pub(super) fn hostile_line(max: usize) -> impl Strategy<Value = String> {
    hostile_text(max).prop_map(|text| text.replace(['\n', '\r'], "~"))
}

/// Comment ids are minted by `rv`, so the id space here is kept small on
/// purpose, to make upsert collisions common.
///
/// The ids also vary in *length* and are deliberately laid out so that some are
/// prefixes of others (`id` of everything, `id0` of `id00`, `id1` of `id10`).
/// An id space of equal-length ids cannot tell `existing.id == comment.id`
/// apart from `existing.id.starts_with(&comment.id)` (or `contains`), which is
/// the same class of defect as keying the upsert on the wrong field: identity
/// matched by something looser than equality. The prefix pairs make the two
/// disagree, so the upsert properties can see the difference.
pub(super) const ID_POOL: &[&str] = &["id", "id0", "id00", "id1", "id10", "id2"];

/// The `index`-th distinct id: [`ID_POOL`] while it lasts, then `id6`, `id7`, …
/// which stay distinct from every pool entry. Used where a strategy needs *n*
/// pairwise-different ids rather than collisions.
pub(super) fn distinct_id(index: usize) -> String {
    ID_POOL
        .get(index)
        .map(|id| (*id).to_owned())
        .unwrap_or_else(|| format!("id{}", index + ID_POOL.len()))
}

pub(super) fn id_pool(count: usize) -> Vec<String> {
    ID_POOL
        .iter()
        .take(count)
        .map(|id| (*id).to_owned())
        .collect()
}

pub(super) fn hex(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!['0', '3', '9', 'a', 'd', 'f']),
        1..max,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

pub(super) fn side() -> impl Strategy<Value = Side> {
    prop_oneof![Just(Side::Left), Just(Side::Right)]
}

pub(super) fn comment_state() -> impl Strategy<Value = CommentState> {
    prop_oneof![
        Just(CommentState::Open),
        Just(CommentState::AwaitingVerification),
        Just(CommentState::Resolved),
        Just(CommentState::Outdated),
    ]
}

pub(super) fn anchor(text_max: usize) -> impl Strategy<Value = Anchor> {
    (
        hostile_text(text_max),
        side(),
        any::<u32>(),
        hex(12),
        prop::collection::vec(hostile_line(text_max), 0..4),
    )
        .prop_map(|(file, side, line, content_hash, context)| Anchor {
            file,
            side,
            line,
            content_hash,
            context,
            context_start: 1,
        })
}

pub(super) fn comment(
    id: impl Strategy<Value = String>,
    text_max: usize,
) -> impl Strategy<Value = Comment> {
    (
        id,
        hex(12),
        hex(12),
        anchor(text_max),
        hostile_text(text_max),
        comment_state(),
        prop::option::of(hostile_text(text_max)),
    )
        .prop_map(
            |(id, change_id, commit_id, anchor, body, state, reply)| Comment {
                id,
                change_id,
                commit_id,
                anchor,
                body,
                state,
                reply,
                settled_by: None,
            },
        )
}

/// A sequence of comments drawn from a small id pool, so runs contain both
/// fresh inserts and same-id updates — and, because [`ID_POOL`] is
/// prefix-structured, both same-id updates and *near*-id non-updates.
pub(super) fn comment_sequence(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Comment>> {
    prop::collection::vec(comment(prop::sample::select(id_pool(5)), 8), len)
}

/// Comments whose ids are distinct by construction, so no upsert collapsing
/// happens and every appended comment must survive. The ids still come from
/// [`ID_POOL`], so "distinct" here means distinct under `==` while remaining
/// entangled under `starts_with`.
pub(super) fn distinct_comments(
    len: std::ops::Range<usize>,
) -> impl Strategy<Value = Vec<Comment>> {
    prop::collection::vec(comment(Just(String::new()), 8), len).prop_map(|mut comments| {
        for (index, comment) in comments.iter_mut().enumerate() {
            comment.id = distinct_id(index);
        }
        comments
    })
}

/// Distinct ids as above, but `change_id` drawn from a two-element pool so
/// collisions on it are the common case rather than a `hex(12)` lottery. Any
/// identity keyed on `change_id` instead of `id` collapses these; identity
/// keyed on `id` leaves every one of them standing.
pub(super) fn distinct_comments_sharing_change_ids(
    len: std::ops::Range<usize>,
) -> impl Strategy<Value = Vec<Comment>> {
    prop::collection::vec(
        (
            comment(Just(String::new()), 8),
            prop::sample::select(vec!["chg0".to_owned(), "chg1".to_owned()]),
        ),
        len,
    )
    .prop_map(|pairs| {
        pairs
            .into_iter()
            .enumerate()
            .map(|(index, (mut comment, change_id))| {
                comment.id = distinct_id(index);
                comment.change_id = change_id;
                comment
            })
            .collect()
    })
}

pub(super) fn change_ref(text_max: usize) -> impl Strategy<Value = ChangeRef> {
    (hex(12), hex(12), hostile_text(text_max)).prop_map(|(change_id, commit_id, description)| {
        ChangeRef {
            change_id,
            commit_id,
            description,
        }
    })
}

/// A whole `session.toml`: hostile scope strings, a change list, and the
/// comments the file now carries. `write_review` replaces the file wholesale,
/// so a round-trip property that generated no comments would leave the array
/// out of the claim entirely.
pub(super) fn session(text_max: usize, max_changes: usize) -> impl Strategy<Value = Session> {
    (
        hostile_text(text_max),
        hex(12),
        hex(12),
        prop::collection::vec(change_ref(text_max), 0..max_changes),
        hostile_text(text_max),
        distinct_comments(0..3),
    )
        .prop_map(
            |(revset, base_commit, head_commit, changes, started_at, comments)| Session {
                revset,
                base_commit,
                head_commit,
                changes,
                started_at,
                comments,
            },
        )
}

/// Plausible `.git/info/exclude` contents: other tools' patterns mixed with
/// near-misses of `/.review/` that must not be mistaken for it.
pub(super) fn exclude_seed() -> impl Strategy<Value = String> {
    let line = prop_oneof![
        Just("/.review/".to_owned()),
        Just("#/.review/".to_owned()),
        Just("# /.review/".to_owned()),
        Just("/.review".to_owned()),
        Just(".review/".to_owned()),
        Just("/.review/ ".to_owned()),
        Just("  /.review/".to_owned()),
        Just("x/.review/y".to_owned()),
        Just("!/.review/".to_owned()),
        Just("/.review/*".to_owned()),
        Just(String::new()),
        Just("target/".to_owned()),
        Just("*.log".to_owned()),
        Just("\t/.review/".to_owned()),
    ];
    (prop::collection::vec(line, 0..5), any::<bool>()).prop_map(|(lines, trailing_newline)| {
        let mut seed = lines.join("\n");
        if trailing_newline && !seed.is_empty() {
            seed.push('\n');
        }
        seed
    })
}

/// One store operation, so a property can drive an arbitrary interleaving of
/// everything the module writes. Every mutating method on `Store` has a
/// variant here — `append_comment`, `remove_comment`, `write_review`,
/// `write_markdown`, `ensure_excluded` — which is what lets the properties
/// below say "every write the module makes" and mean it.
#[derive(Clone, Debug)]
pub(super) enum Op {
    Append(Comment),
    Remove(String),
    WriteReview(Session),
    WriteMarkdown(String),
    EnsureExcluded,
}

/// Ids for [`Op::Remove`] to aim at. Mostly the same pool [`Op::Append`] draws
/// from, so a removal usually finds its target; sometimes an id that pool never
/// mints (`id10` is outside `id_pool(4)`, and `gone` is in no pool at all), so
/// the unknown-id path — a no-op that must neither fail nor disturb anything —
/// gets exercised in the same sequences.
///
/// [`ID_POOL`]'s prefix structure does double duty on the delete path: removing
/// `id0` must not take `id00` with it, and removing `id` must not empty the
/// store, so `existing.id != id` is distinguishable from
/// `!existing.id.starts_with(id)`.
pub(super) fn removable_id() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => prop::sample::select(id_pool(4)),
        1 => Just("id10".to_owned()),
        1 => Just("gone".to_owned()),
    ]
}

pub(super) fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => comment(prop::sample::select(id_pool(4)), 8).prop_map(Op::Append),
        3 => removable_id().prop_map(Op::Remove),
        2 => session(8, 2).prop_map(Op::WriteReview),
        2 => hostile_text(20).prop_map(Op::WriteMarkdown),
        2 => Just(Op::EnsureExcluded),
    ]
}

/// Appends and removals only, in a ratio that makes long append/remove/re-append
/// histories over a handful of ids the common case rather than a rarity.
pub(super) fn append_or_remove() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => comment(prop::sample::select(id_pool(4)), 8).prop_map(Op::Append),
        2 => removable_id().prop_map(Op::Remove),
    ]
}

pub(super) fn apply(store: &Store, op: &Op) -> Result<(), rv_core::store::Error> {
    match op {
        Op::Append(comment) => store.append_comment(comment),
        Op::Remove(id) => store.remove_comment(id).map(|_| ()),
        Op::WriteReview(review) => store.write_review(review),
        Op::WriteMarkdown(document) => store.write_markdown(document),
        Op::EnsureExcluded => store.ensure_excluded().map(|_| ()),
    }
}
