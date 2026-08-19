//! The documented keymap.

use std::fs;
use std::path::Path;

use rstest::rstest;

use rv::app::BINDINGS;

use crate::support::*;

/// The README, read from the workspace rather than from the process's working
/// directory — a test binary's cwd is not something to depend on.
fn readme() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../README.md"))
        .expect("read README.md")
}

/// The `Key` column of the markdown table that follows `label`, one entry per
/// row, without the header row or its underline.
///
/// The table is taken as the run of `|` lines after the label, so the tables
/// under the other labels — `Esc` appears in two of them — cannot leak into the
/// answer.
fn table_keys(label: &str) -> Vec<String> {
    let readme = readme();
    let (_, body) = readme
        .split_once(label)
        .unwrap_or_else(|| panic!("the README has no {label} table"));
    body.lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .filter_map(|line| line.split('|').nth(1))
        .map(|cell| cell.trim().to_owned())
        .filter(|cell| cell != "Key" && !cell.starts_with("---"))
        .collect()
}

/// The README under `heading`, up to the next heading of any level.
fn readme_section(heading: &str) -> String {
    let readme = readme();
    let (_, body) = readme
        .split_once(heading)
        .unwrap_or_else(|| panic!("the README has no {heading:?} section"));
    body.lines()
        .take_while(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_readme_documents_every_browse_binding() {
    let documented = table_keys("**Browsing**");
    for key in BROWSE_KEYS {
        assert!(
            documented.iter().any(|row| row == key),
            "the README's Browsing table has no row for {key}, so a reviewer \
             cannot find out that it exists: {documented:?}"
        );
    }
}

/// The direction the pair above could not see.
///
/// [`BROWSE_KEYS`] and the README were held to each other, and both to nothing
/// else — so a key added to [`BINDINGS`] and to neither of them shipped
/// documented nowhere and the suite stayed green. That is not a hypothetical:
/// `r` and `a` were added to the table and the whole suite passed with the page
/// still describing a reviewer that could not resolve anything.
///
/// This closes the loop. [`BINDINGS`] is the dispatcher, so a row of it is a key
/// that *works*; every one of them has to reach the manual.
#[test]
fn every_binding_is_a_key_the_readme_lists() {
    for binding in BINDINGS {
        let spelled = readme_spelling(binding.keys);
        assert!(
            BROWSE_KEYS.contains(&spelled.as_str()),
            "`{}` is dispatched but is not in BROWSE_KEYS, so nothing requires \
             the README to mention it: {BROWSE_KEYS:?}",
            binding.keys
        );
    }
}

/// A binding's `keys` as the README spells it: every key token in backticks,
/// the parentheses around an alias left as they are.
///
/// `↓ (j)` in the table is `` `↓` (`j`) `` on the page — the same two keys with
/// the same one leading, which is the spelling the whole keymap is held to.
fn readme_spelling(keys: &str) -> String {
    keys.split(' ')
        .map(
            |token| match token.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
                Some(alias) => format!("(`{alias}`)"),
                None => format!("`{token}`"),
            },
        )
        .collect::<Vec<_>>()
        .join(" ")
}

/// ...and no row for a key that is not one of them: a table that documents a
/// binding nobody wrote is worse than one that documents none, because a
/// reviewer will press it and read the result as a bug in the key rather than
/// in the page.
#[test]
fn the_readme_documents_no_binding_that_is_not_bound() {
    let documented = table_keys("**Browsing**");
    for row in &documented {
        assert!(
            BROWSE_KEYS.contains(&row.as_str()),
            "the README's Browsing table has a row for {row:?}, which is not one \
             of this reviewer's keys: {BROWSE_KEYS:?}"
        );
    }
}

/// The comment box is the thing a reviewer meets first and has the most
/// questions about, and every one of them below is answered somewhere in the
/// code rather than in the page: that a reply shares its comment's box and is
/// drawn dimmed, that folding is a preference of this session and reaches no
/// file, that a delete is permanent and wants a `y`, and that the markdown is
/// an export written by `rv render` — not a document kept continuously in step,
/// which is what a reader assumes of a file in their working tree until they
/// are told otherwise.
///
/// # Why two of these are phrases rather than words
///
/// A one-word probe passes on a mention, and a mention is not a claim. The
/// export cases are the ones where that difference bites: this section says
/// "an LLM reading the export" in its *folding* paragraph, so a page that had
/// been rewritten to promise `REVIEW-FEEDBACK.md` is kept continuously in step
/// with the store still contained the word `export` and still passed. That
/// mutant — the page asserting the exact opposite of the truth — survived the
/// wave that added these cases, which is the worst shape a documentation test
/// can have: it reports the drift it exists to catch as covered.
///
/// So the two export cases pin the sentence's *claim* — that the file **is** an
/// export, and that it is **not** kept in step — and are deliberately longer and
/// more brittle than the rest. A reworded README should fail them; a reworded
/// README is exactly when someone should be made to reread that paragraph and
/// decide whether the promise it makes is still true. `bordered` is split out of
/// `blue` for the smaller version of the same reason: the case was named for two
/// facts and checked one.
#[rstest]
#[case::under_its_line("beneath the line")]
#[case::the_box_is_blue("blue")]
#[case::the_box_is_bordered("bordered")]
#[case::a_reply_shares_the_box("reply")]
#[case::a_reply_is_dimmed("dimmed")]
#[case::folding_is_a_session_preference("session")]
#[case::deletion_is_permanent("permanent")]
#[case::deletion_is_confirmed("`y`")]
#[case::the_markdown_is_a_view("is a **view**")]
#[case::nothing_reads_it_back("read back by nothing")]
#[case::written_on_request("`rv render --out`")]
fn the_readme_explains_inline_comments(#[case] phrase: &str) {
    let section = readme_section("### Inline comments");
    assert!(
        section.contains(phrase),
        "the README's inline-comments section never mentions {phrase:?}:\n{section}"
    );
}
