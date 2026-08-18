//! Consts shared by the case modules.



/// The status line `App::new` starts on. Copied from `app.rs`'s private `HELP`
/// on purpose: if the help text changes, these cases should be re-read rather
/// than silently following it.
pub const HELP: &str = "↓↑ line  [/] file  c comment  enter stack  d delete  s fold  ? help  q quit";

/// What `Enter`, `d` and `s` report on a line carrying no comments. Copied from
/// `app.rs`'s private constant for the same reason [`HELP`] is.
pub const NO_COMMENTS: &str = "no comments on this line";

/// `alpha.rs` at the base of [`Fixture::multi`]. Every line is distinct, so a
/// diff line can be located in the frame unambiguously.
pub const ALPHA_BASE: &str = "\
pub fn alpha() {
    let a01 = 1;
    let a02 = 2;
    let a03 = 3;
    let a04 = 4;
    let a05 = 5;
}
";

/// `alpha.rs` at the head: a header line inserted *above* the rewritten one,
/// so a changed line sits at a different number on each side.
pub const ALPHA_HEAD: &str = "\
// alpha header
pub fn alpha() {
    let a01 = 1;
    let a02 = 22;
    let a03 = 3;
    let a04 = 4;
    let a05 = 5;
}
";

/// `a.rs` at the base of [`Fixture::renamed`].
///
/// Long enough that the head side below still counts as the same file to jj's
/// copy detection: two rewritten lines out of ten leaves the similarity high.
/// A shorter pair is reported as a delete plus an add, which has no left side
/// to anchor anything to.
pub const RENAME_BASE: &str = "\
pub fn a() {
    let x = 1;
    let y = 2;
    let z = 3;
    let w = 4;
    let v = 5;
    let u = 6;
    let t = 7;
    let s = 8;
}
";

/// `b.rs` at the head of [`Fixture::renamed`]: renamed, one line inserted at
/// the top and two rewritten, so several diff lines carry a left number that
/// differs from their right one.
pub const RENAME_HEAD: &str = "\
// header
pub fn a() {
    let x = 42;
    let y = 2;
    let z = 33;
    let w = 4;
    let v = 5;
    let u = 6;
    let t = 7;
    let s = 8;
}
";

/// `same.rs` at the base of [`Fixture::collisions`]: the line that gets
/// rewritten sits at line 2, and nothing is inserted above it.
///
/// Verified with `DFT_UNSTABLE=yes difft --display json` that difftastic pairs
/// the two versions of line 2 as `lhs 1 rhs 1`, so *both* halves of the pair
/// come back with `left == right == Some(2)`: same file, same number, opposite
/// sides. That is the one shape in which a side-blind comment id loses a
/// comment.
pub const SAME_BASE: &str = "\
pub fn same() {
    let b = 1;
}
";

/// `same.rs` at the head of [`Fixture::collisions`]: line 2 rewritten in place.
pub const SAME_HEAD: &str = "\
pub fn same() {
    let X = 1;
}
";

/// `ctx.rs` at the base of [`Fixture::fallback`]: two lines that survive
/// unchanged around one that does not, so the `similar` fallback has context to
/// emit.
pub const CTX_BASE: &str = "\
pub fn ctx() {
    let keep1 = 1;
    let change = 2;
    let keep2 = 3;
}
";

/// `ctx.rs` at the head of [`Fixture::fallback`]: one line rewritten and one
/// appended, so the fallback diff carries `Context`, `Removed` and `Added`
/// lines at once.
pub const CTX_HEAD: &str = "\
pub fn ctx() {
    let keep1 = 1;
    let change = 22;
    let keep2 = 3;
    let added = 4;
}
";

/// `eol.rs` at the base of [`Fixture::terminator`]: three lines, the last of
/// them *not* terminated.
pub const EOL_BASE: &str = "fn eol() {\n    let e = 1;\n}";

/// `eol.rs` at the head: the same three lines, now with a final newline. The
/// files differ, and no line of either differs from its counterpart.
pub const EOL_HEAD: &str = "fn eol() {\n    let e = 1;\n}\n";

/// `crlf.txt` at the base of [`Fixture::terminator`]: CRLF terminators.
pub const CRLF_BASE: &str = "alpha\r\nbeta\r\n";

/// `crlf.txt` at the head: the same two lines, LF-terminated. The other shape
/// of a terminator-only change, and the one that touches every line at once.
pub const CRLF_HEAD: &str = "alpha\nbeta\n";

/// The sentence the diff pane shows for a diff `rv_core::diff` suppressed.
/// Both of the pane's suppressed branches — the note above a suppressed diff's
/// lines, and the whole body of one that has none — start with it, so a test
/// can look for it without pinning either wording.
pub const SUPPRESSED: &str = "no semantic change";

/// How many lines [`Fixture::multi`]'s `long.rs` has: comfortably more than
/// any pane height the rendering properties sweep, so the diff pane has to
/// scroll.
pub const LONG_LINES: usize = 40;

/// The terminal sizes that have historically broken ratatui layout arithmetic:
/// a single cell, a single row, a single column, and the ones where a bar
/// asking for three rows meets a frame that has one or two.
///
/// Spelled out rather than sampled wherever they are swept, because these are
/// *the* cases and a uniform draw over a plausible range visits them almost
/// never.
pub const PATHOLOGICAL: [(u16, u16); 8] = [
    (1, 1),
    (80, 1),
    (1, 40),
    (2, 5),
    (5, 2),
    (3, 3),
    (40, 2),
    (40, 3),
];
