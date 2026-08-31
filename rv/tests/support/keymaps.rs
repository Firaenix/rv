//! The keys the browse-mode keymap answers.

/// Every key this reviewer binds while browsing, spelled the way README's
/// **Browsing** table spells the `Key` column of its rows.
///
/// Movement is arrows (and the mouse) only; the rest of the keymap is layered
/// under three leaders — `g` goes somewhere, `c` acts on the comment under the
/// cursor, `v` changes the view — so most rows are a chord: the leader's letter
/// then the key. `Ctrl+C` is [`rv::app::App::on_key_event`]'s, answered before
/// the mode is consulted, and is in the same table because a reviewer looking
/// for how to get out does not care which function answers it.
///
/// The two tests hold this list and the README table to each other in *both*
/// directions, so neither a binding that ships undocumented nor a row for a key
/// nobody wrote survives.
pub const BROWSE_KEYS: &[&str] = &[
    // Direct
    "`↓`",
    "`↑`",
    "`←`",
    "`→`",
    "`PgDn`",
    "`PgUp`",
    "`Home`",
    "`End`",
    "`Shift`+`←`",
    "`Shift`+`→`",
    "`]`",
    "`[`",
    "`Enter`",
    "`Tab`",
    "`s`",
    "`E`",
    "`Esc`",
    "`?`",
    "`q`",
    "`Ctrl+C`",
    // Space — mode
    "`Space` `f`",
    "`Space` `c`",
    "`Space` `m`",
    "`Space` `d`",
    // g — goto
    "`g` `↓`",
    "`g` `↑`",
    "`g` `n`",
    "`g` `N`",
    "`g` `/`",
    // c — comment
    "`c` `c`",
    "`c` `d`",
    "`c` `r`",
    "`c` `a`",
    // v — view
    "`v` `f`",
    "`v` `g`",
    "`v` `b`",
    "`v` `t`",
    "`v` `o`",
    "`v` `c`",
    "`v` `#`",
    "`v` `z`",
    "`v` `<`",
    "`v` `>`",
    "`v` `i`",
    "`v` `r`",
];
