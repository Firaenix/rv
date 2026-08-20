//! The render's properties: conservation, section structure, column-0
//! discipline, header invariance and order stability.
//!
//! Split from [`super`] for the 400-line rule; the generators, fixtures and
//! oracles they run against live there.

use proptest::prelude::*;
use rv_core::markdown::render;

use super::EXPANDED_STATES;
use super::SECTION_ORDER;
use super::count_lines;
use super::details_depths;
use super::entry_numbers;
use super::generator::CommentSpec;
use super::generator::build_comments;
use super::generator::comment_spec;
use super::generator::comment_specs;
use super::generator::hostile_text;
use super::generator::session_strategy;
use super::is_entry_heading;
use super::protocol_lines;
use super::reterminated;
use rv_core::store::Comment;
// ---------------------------------------------------------------------------
// Conservation and structure
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    /// No comment is ever dropped, whatever its state, content, or whether the
    /// session still lists its change: one entry and one anchor marker per
    /// comment, numbered `1..=n` in document order.
    #[test]
    fn every_comment_renders_exactly_once(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);

        let expanded = comments
            .iter()
            .filter(|comment| EXPANDED_STATES.contains(&comment.state))
            .count();
        let collapsed = comments.len() - expanded;

        prop_assert_eq!(
            count_lines(&document, is_entry_heading),
            expanded,
            "one `### <n>.` heading per expanded comment"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("<details><summary>")),
            collapsed,
            "one collapsed entry per resolved/outdated comment"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line == "</details>"),
            collapsed,
            "every <details> must be closed"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("<!-- rv:anchor ")),
            comments.len(),
            "one anchor marker per comment"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("**Comment:**")),
            comments.len(),
            "one comment body per comment"
        );

        // Every id is present, once, in a marker of its own.
        for comment in &comments {
            let marker = format!("<!-- rv:anchor id={} ", comment.id);
            prop_assert_eq!(
                count_lines(&document, |line| line.starts_with(&marker)),
                1,
                "id {} must appear in exactly one anchor marker",
                comment.id
            );
        }

        let numbers = entry_numbers(&document);
        prop_assert_eq!(
            numbers,
            (1..=comments.len()).collect::<Vec<usize>>(),
            "entries must be numbered 1..=n in document order"
        );
    }

    /// The four sections come in fixed order, each heading's count is the
    /// number of comments actually in that state, and every comment's anchor
    /// marker sits under its own section heading — collapsed inside a
    /// `<details>` for Resolved/Outdated, at depth zero otherwise.
    #[test]
    fn sections_are_ordered_counted_and_correctly_collapsed(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);
        let lines: Vec<&str> = document.lines().collect();
        let depths = details_depths(&lines);

        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("## ")),
            SECTION_ORDER.len(),
            "exactly five section headings, whatever the bodies contain"
        );
        prop_assert_eq!(
            depths.last().copied().unwrap_or(0)
                + usize::from(lines.last().is_some_and(|line| line.starts_with("<details"))),
            0,
            "every <details> must be balanced"
        );

        // Heading positions, in the order they appear.
        let headings: Vec<(usize, &str)> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.starts_with("## "))
            .map(|(index, line)| (index, *line))
            .collect();
        for (position, (_, heading)) in headings.iter().enumerate() {
            let (title, state) = SECTION_ORDER[position];
            let in_state = comments
                .iter()
                .filter(|comment| comment.state == state)
                .count();
            let expected = format!("## {title} ({in_state})");
            prop_assert_eq!(
                *heading,
                expected.as_str(),
                "section {} is wrong",
                position
            );
        }

        for comment in &comments {
            let marker = format!("<!-- rv:anchor id={} ", comment.id);
            let at = lines
                .iter()
                .position(|line| line.starts_with(&marker))
                .expect("every comment must render an anchor marker");
            let section = SECTION_ORDER
                .iter()
                .position(|(_, state)| *state == comment.state)
                .expect("every state is a section");
            let start = headings[section].0;
            let end = headings
                .get(section + 1)
                .map_or(lines.len(), |(index, _)| *index);
            prop_assert!(
                start < at && at < end,
                "id {} landed outside its own section",
                comment.id
            );

            let collapsed = !EXPANDED_STATES.contains(&comment.state);
            prop_assert_eq!(
                depths[at],
                usize::from(collapsed),
                "id {} has the wrong <details> nesting",
                comment.id
            );
        }
    }

    /// Structure lives at column 0, and only `render` writes there: every
    /// non-empty line of a rendered document is either indented out of column
    /// 0 or one of the shapes `render` authors — and the number of each kind
    /// of authored line matches the input, so interpolated prose cannot add
    /// one.
    #[test]
    fn only_render_writes_at_column_zero(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);
        let with_reply = comments.iter().filter(|c| c.reply.is_some()).count();

        for line in document.lines() {
            let authored = line == "<!-- rv:v1 -->"
                || line.starts_with("# Review: `")
                || line.starts_with("Base `")
                || line.starts_with("> ")
                || line.starts_with("## ")
                || is_entry_heading(line)
                || line.starts_with("<details><summary>")
                || line == "</details>"
                || line.starts_with("<!-- rv:anchor ")
                || line.starts_with("**Comment:**")
                || line.starts_with("**Reply:**")
                // Names a `trunk()` that resolved to the root — see
                // `markdown::degraded_base`.
                || line.starts_with("**Note:**");
            prop_assert!(
                line.is_empty() || line.starts_with("  ") || authored,
                "line at column 0 that render did not author: {:?}",
                line
            );
        }

        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("**Reply:**")),
            with_reply,
            "exactly one column-0 reply marker per stored reply"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("> ")),
            protocol_lines(),
            "the protocol block must be the only quoted block at column 0"
        );
        prop_assert_eq!(
            count_lines(&document, |line| line == "<!-- rv:v1 -->"),
            1,
            "one version marker"
        );
    }

    /// The header is unconditional: version marker first, then the review
    /// heading with correctly pluralized counts, the base→head line, and one
    /// protocol block — no matter what the comments contain.
    #[test]
    fn the_header_and_protocol_survive_any_content(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let document = render(&session, &comments);

        prop_assert!(
            document.starts_with("<!-- rv:v1 -->\n"),
            "the version marker must be the first line"
        );

        let changes = session.changes.len();
        let plural = |count: usize| if count == 1 { "" } else { "s" };
        let heading = format!(
            "# Review: `{}` — {} change{}, {} comment{}\n",
            session.revset,
            changes,
            plural(changes),
            comments.len(),
            plural(comments.len()),
        );
        prop_assert!(
            document.contains(&heading),
            "missing or miscounted review heading: {:?}",
            heading
        );
        prop_assert!(
            document.contains(&format!(
                "Base `{}` → head `{}`",
                session.base_commit, session.head_commit
            )),
            "missing base→head line"
        );
        prop_assert!(
            document.contains(&session.started_at),
            "missing session start"
        );

        // The protocol block is one contiguous run of `> ` lines.
        let lines: Vec<&str> = document.lines().collect();
        let first_quote = lines
            .iter()
            .position(|line| line.starts_with("> "))
            .expect("the protocol block must be rendered");
        let run = lines[first_quote..]
            .iter()
            .take_while(|line| line.starts_with("> "))
            .count();
        prop_assert_eq!(run, protocol_lines(), "the protocol block must be contiguous");
        prop_assert!(
            lines[first_quote].contains("rendered view"),
            "the note must say the document is a view"
        );
        prop_assert!(
            lines[first_quote..first_quote + run]
                .iter()
                .any(|line| line.contains("rv comments --json")),
            "the note must name the CLI that replaced the round trip"
        );
    }
}

// ---------------------------------------------------------------------------
// Order and stability
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    /// The last clause of the documented entry order, and the only one a random
    /// generator cannot be relied on to reach: comments that tie on *every* sort
    /// key — same section, same change, same file, same line — keep the order
    /// they were stored in. `render` says so in a comment on its `sort_by`,
    /// but with
    /// `comment_spec()`'s original `0u32..4000` line strategy an exact
    /// `(state, change, file, line)` tie arose about once in 5000 pairs, so
    /// nothing in this file exercised the clause.
    ///
    /// Built by construction, and asserted against the *document* rather than
    /// against the sort: `n` comments identical in every key, differing only in
    /// the id `build_comments` assigns from the index and in their hostile
    /// bodies, must lay their anchor markers down in ascending id order.
    #[test]
    fn entries_that_tie_on_every_sort_key_keep_their_stored_order(
        session in session_strategy(),
        template in comment_spec(),
        bodies in prop::collection::vec(
            (hostile_text(), prop::option::weighted(0.7, hostile_text())),
            2..6,
        ),
    ) {
        let specs: Vec<CommentSpec> = bodies
            .into_iter()
            .map(|(body, reply)| CommentSpec {
                body,
                reply,
                ..template.clone()
            })
            .collect();
        let comments = build_comments(&specs);
        let document = render(&session, &comments);

        // `every_comment_renders_exactly_once` pins that a column-0
        // `<!-- rv:anchor ` line is one `render` wrote, so reading the ids off
        // them is reading the document, not re-running the sort.
        let laid_down: Vec<&str> = document
            .lines()
            .filter_map(|line| line.strip_prefix("<!-- rv:anchor id="))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        let stored: Vec<&str> = comments.iter().map(|comment| comment.id.as_str()).collect();
        prop_assert_eq!(
            &laid_down,
            &stored,
            "tied entries were reordered: every comment is on {}:{} of change {}",
            template.file, template.line, template.change_id
        );

        prop_assert_eq!(
            count_lines(&document, |line| line.starts_with("**Reply:**")),
            comments.iter().filter(|comment| comment.reply.is_some()).count(),
            "a document of nothing but tied entries lost or duplicated a reply"
        );
    }

    /// The page does not depend on how a body was terminated.
    ///
    /// `render` writes every body out through `str::lines`, so a trailing
    /// newline is redundant with the terminator the last line already gets,
    /// and a CRLF ending arrives as LF. Re-terminating every stored body must
    /// therefore produce a byte-identical document.
    ///
    /// The loop this defends is real: `rv render` runs again and again over a
    /// store that keeps changing around one unchanged comment, and a body
    /// whose page depended on a terminator nobody can see would churn the file
    /// under whoever is reading it. [`reterminated`] derives the variant from
    /// `str::lines`'s stated behaviour rather than by calling into
    /// `markdown.rs`, so the claim stands on an independent oracle.
    #[test]
    fn the_document_is_a_fixpoint_from_the_second_pass(
        session in session_strategy(),
        specs in comment_specs(),
    ) {
        let comments = build_comments(&specs);
        let first = render(&session, &comments);

        let variant: Vec<Comment> = comments
            .iter()
            .map(|comment| {
                let mut updated = comment.clone();
                updated.body = reterminated(&comment.body);
                updated.reply = comment.reply.as_deref().map(reterminated);
                updated
            })
            .collect();

        prop_assert_eq!(
            render(&session, &variant),
            first.clone(),
            "the page changed with a body's terminator, which never reaches it"
        );
        prop_assert_eq!(
            render(&session, &comments),
            first,
            "the render is not deterministic"
        );
    }
}
