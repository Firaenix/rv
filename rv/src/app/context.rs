//! Building the lines a file's diff pane actually draws: full-file context
//! where `rv_core::diff::merge_context` can build it honestly, the engine's
//! own lines otherwise — and cached, because the pane is a pure function of
//! `App` and every drawn frame reads it many times over.
//!
//! See `docs/superpowers/specs/2026-08-21-rv-full-file-context-design.md`
//! §4.1/§4.4/§4.5/§5. The `full` flag lets the reviewer toggle the merge
//! off with `f` (§5, walked back: honesty was §5's argument for having no
//! toggle, but the toggle is also an escape hatch for wanting to see less
//! on one file, which is orthogonal to honesty and which the fallback
//! cannot serve).

use std::rc::Rc;

use rv_core::diff::DiffLine;
use rv_core::diff::DiffSource;
use rv_core::diff::FileDiff;
use rv_core::diff::merge_context;

/// What a file's diff pane draws, and whether that is the whole file.
///
/// [`Rc`] because [`crate::app::App::plan`], the hunk-navigation module and
/// the pane's own paint path all read this several times per frame — a paint
/// against a big file called `context::build` 126 times before it was
/// cached. Handing every reader an [`Rc`] over one built value is what
/// collapses that back to one.
pub(super) struct Displayed {
    pub(super) lines: Vec<DiffLine>,
    /// Set only when a merge was attempted **and declined** (§3's
    /// reformatted-region case) — never for a source the merge is not asked
    /// to run over at all (binary, an empty suppressed diff, the `similar`
    /// fallback, which is already full context by construction) and never
    /// when the reviewer turned the merge off (§5 toggle) so the "context
    /// unavailable" suffix reports a failure, not a choice.
    pub(super) bailed: bool,
}

/// Computes what [`Displayed`] should hold for `diff`, from the same
/// `old`/`new` bytes `diff`'s own lines were computed from.
///
/// `full` is the reviewer's toggle (§5): `false` short-circuits the merge
/// and hands back the engine's own changed-only lines with `bailed: false`,
/// because a reviewer who asked for the shorter view is not being told the
/// long one was unavailable.
///
/// Full context is only attempted for a genuine difftastic answer with
/// lines to anchor a walk from — a suppressed empty diff has no anchor
/// (§4.5, deferred rather than solved here), and the fallback already emits
/// full context, so both skip the merge outright rather than report a
/// vacuous "already full."
pub(super) fn build(
    diff: &FileDiff,
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    full: bool,
) -> Rc<Displayed> {
    if !full
        || !matches!(diff.source, DiffSource::Difftastic { .. })
        || diff.lines.is_empty()
    {
        return Rc::new(Displayed {
            lines: diff.lines.clone(),
            bailed: false,
        });
    }
    let old_text = String::from_utf8_lossy(old.unwrap_or(&[])).into_owned();
    let new_text = String::from_utf8_lossy(new.unwrap_or(&[])).into_owned();
    match merge_context(&diff.lines, &old_text, &new_text) {
        Some(lines) => Rc::new(Displayed {
            lines,
            bailed: false,
        }),
        None => Rc::new(Displayed {
            lines: diff.lines.clone(),
            bailed: true,
        }),
    }
}
