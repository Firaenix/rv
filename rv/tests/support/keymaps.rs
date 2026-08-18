//! The keys the browse-mode keymap answers.


/// Every key this reviewer binds while browsing, spelled the way README's
/// **Browsing** table spells the `Key` column of its rows.
///
/// All but the last are [`rv::app::App::on_key`]'s, read straight out of
/// `on_key_browse`; `Ctrl+C` is [`rv::app::App::on_key_event`]'s, answered
/// before the mode is consulted at all, and is in the same table because a
/// reviewer looking for how to get out does not know or care which function
/// answers them.
///
/// The two tests below hold this list and the table to each other in *both*
/// directions, so neither a binding that ships undocumented nor a row for a key
/// nobody wrote survives. Three waves of this milestone shipped keys the README
/// never mentioned — focus, the tab, the stack, `d`, `s` — which is the drift
/// this exists to stop. What each key actually does is pinned by
/// `rv/tests/app_cases.rs`'s `browse_keybindings` table and by the tests above;
/// this pair is only about whether a user can find out.
/// The arrow leads and the vim key follows in parentheses, here as in
/// [`BINDINGS`] and in the popup: rv is a tool a reviewer may open once a week,
/// and the arrows are the keys someone can find without being told.
pub const BROWSE_KEYS: &[&str] = &[
    "`↓` (`j`)",
    "`↑` (`k`)",
    "`←` (`h`)",
    "`→` (`l`)",
    "`]`",
    "`[`",
    "`Tab`",
    "`Enter`",
    "`Esc`",
    "`c`",
    "`d`",
    "`s`",
    "`<`",
    "`>`",
    "`?`",
    "`q`",
    "`Ctrl+C`",
];
