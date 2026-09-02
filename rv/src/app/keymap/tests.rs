//! The patch and collision semantics, held to the plan's policy table: a
//! rebind moves, a scoped bind narrows, user-vs-default steals with an alert,
//! and user-vs-user is the one contradiction that refuses to load.

use crossterm::event::KeyCode;

use super::super::Context;
use super::super::bindings::{
    AppCommand, Command, CursorCommand, DiffCommand, FilesCommand, Leader,
};
use super::Keymap;
use crate::config::parse;

fn keymap(toml: &str) -> Keymap {
    Keymap::from_config(&parse(toml).expect("the config parses")).expect("the keymap builds")
}

fn command_of(
    keymap: &Keymap,
    leader: Option<Leader>,
    key: KeyCode,
    context: Context,
) -> Option<Command> {
    keymap.lookup(leader, key, context).map(|row| row.command)
}

#[test]
fn the_default_keymap_answers_the_default_keys() {
    let map = Keymap::default();
    assert_eq!(
        command_of(&map, None, KeyCode::Down, Context::Diff),
        Some(Command::Cursor(CursorCommand::NextRow))
    );
    assert_eq!(
        command_of(&map, None, KeyCode::Char('q'), Context::Files),
        Some(Command::App(AppCommand::Quit))
    );
}

#[test]
fn a_rebind_moves_the_command_off_its_old_key() {
    let map = keymap("[keys]\napp_quit = \"Q\"\n");
    assert_eq!(
        command_of(&map, None, KeyCode::Char('Q'), Context::Files),
        Some(Command::App(AppCommand::Quit))
    );
    assert_eq!(
        command_of(&map, None, KeyCode::Char('q'), Context::Files),
        None,
        "the old key is freed, not left as a second door"
    );
}

#[test]
fn arrays_bind_every_listed_key() {
    let map = keymap("[keys]\ncursor_next_row = [\"j\", \"Down\"]\n");
    for key in [KeyCode::Char('j'), KeyCode::Down] {
        assert_eq!(
            command_of(&map, None, key, Context::Diff),
            Some(Command::Cursor(CursorCommand::NextRow))
        );
    }
}

#[test]
fn a_scoped_bind_lives_only_in_its_pane() {
    let map = keymap("[keys.files]\ndiff_toggle_full_context = \"x\"\n");
    assert_eq!(
        command_of(&map, None, KeyCode::Char('x'), Context::Files),
        Some(Command::Diff(DiffCommand::ToggleFullContext))
    );
    assert_eq!(
        command_of(&map, None, KeyCode::Char('x'), Context::Commits),
        None,
        "a files-pane bind answers nowhere else"
    );
}

#[test]
fn a_scoped_bind_beats_a_global_one_for_the_same_key() {
    // `s` is the global fold key; a files-scoped bind on `s` steals it there
    // and only there.
    let map = keymap("[keys.files]\nfiles_next = \"s\"\n");
    assert_eq!(
        command_of(&map, None, KeyCode::Char('s'), Context::Files),
        Some(Command::Files(FilesCommand::Next)),
    );
}

#[test]
fn a_steal_from_a_default_is_a_warning_not_an_error() {
    let mut map = keymap("[keys.files]\nfiles_next = \"s\"\n");
    let warnings = map.take_warnings();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("comment_toggle_fold")),
        "the fold key moving aside is said out loud: {warnings:?}"
    );
    // And fold still answers `s` where the user said nothing about it.
    assert_eq!(
        command_of(&map, None, KeyCode::Char('s'), Context::Diff).map(super::vocab::config_name),
        Some("comment_toggle_fold")
    );
}

#[test]
fn two_user_binds_on_one_key_in_one_place_refuse_to_load() {
    let config = parse("[keys]\napp_quit = \"x\"\ncursor_next_row = \"x\"\n").unwrap();
    let error = Keymap::from_config(&config).unwrap_err();
    assert!(error.to_string().contains("both"), "{error}");
}

#[test]
fn two_leaders_on_one_key_refuse_to_load() {
    let config = parse("[leaders]\ngoto = \"c\"\n").unwrap();
    let error = Keymap::from_config(&config).unwrap_err();
    assert!(error.to_string().contains("leader"), "{error}");
}

#[test]
fn an_unknown_command_gets_a_suggestion() {
    let config = parse("[keys]\napp_qiut = \"Q\"\n").unwrap();
    let error = Keymap::from_config(&config).unwrap_err();
    assert!(
        error.to_string().contains("did you mean \"app_quit\""),
        "{error}"
    );
}

#[test]
fn an_empty_string_unbinds_the_default() {
    let map = keymap("[keys]\napp_quit = \"\"\n");
    assert_eq!(
        command_of(&map, None, KeyCode::Char('q'), Context::Files),
        None
    );
}

#[test]
fn a_pane_scoped_unbind_leaves_the_other_panes_alone() {
    let map = keymap("[keys.diff]\ncomment_toggle_fold = \"\"\n");
    assert_eq!(
        command_of(&map, None, KeyCode::Char('s'), Context::Diff),
        None,
        "unbound where asked"
    );
    assert_eq!(
        command_of(&map, None, KeyCode::Char('s'), Context::Files).map(super::vocab::config_name),
        Some("comment_toggle_fold"),
        "still bound everywhere else"
    );
}

#[test]
fn a_leader_child_rebinds_within_its_leader() {
    let map = keymap("[goto]\ndiff_next_hunk = \"n\"\n");
    assert_eq!(
        command_of(&map, Some(Leader::Goto), KeyCode::Char('n'), Context::Diff)
            .map(super::vocab::config_name),
        Some("diff_next_hunk")
    );
    assert_eq!(
        command_of(&map, Some(Leader::Goto), KeyCode::Down, Context::Diff),
        None,
        "the old chord is freed"
    );
}

#[test]
fn renamed_leaders_open_on_their_new_key() {
    let map = keymap("[leaders]\ngoto = \"G\"\n");
    assert_eq!(map.leader_key(Leader::Goto), 'G');
    assert_eq!(
        map.leader_key(Leader::Comment),
        'c',
        "the others keep theirs"
    );
}

#[test]
fn the_effective_map_reloads_to_itself() {
    let map = keymap("[keys]\napp_quit = \"Q\"\n[goto.diff]\ncursor_last_row = \"e\"\n");
    let dumped = map.render_toml(false);
    let reloaded = keymap(&dumped);
    assert_eq!(
        command_of(&reloaded, None, KeyCode::Char('Q'), Context::Files)
            .map(super::vocab::config_name),
        Some("app_quit"),
        "a dumped keymap is a valid config meaning the same thing:\n{dumped}"
    );
}

#[test]
fn the_template_is_a_loadable_config() {
    // Commented out, so it loads as the empty patch — but it must *parse*.
    let config = parse(&Keymap::template()).expect("the seeded file parses");
    assert!(config.binds.is_empty(), "a fresh template changes nothing");
}

#[test]
fn every_command_has_a_unique_config_name() {
    let mut names: Vec<&str> = super::vocab::ALL_COMMANDS
        .iter()
        .map(|command| super::vocab::config_name(*command))
        .collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total, "two commands share a config name");
}
