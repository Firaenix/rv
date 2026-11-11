use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rv_core::lsp::{
    Answer, Hover, HoverFormat, Location, LspError, LspHub, RequestKind, ServerRegistry,
    ServerStatus, Ticket,
};
use tempfile::TempDir;

use crate::fake_server::{self, Mode};

const LIB_RS: &str = "fn main() {}\nlet a = \"🦀\"; é_x();\n";
/// The `é` on line 2: character 13, but UTF-16 unit 14 after the emoji.
const E_ACUTE_LINE: u32 = 2;
const E_ACUTE_COLUMN: u32 = 13;
const PATIENCE: Duration = Duration::from_secs(10);

fn repo_with_lib_rs() -> TempDir {
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir(repo.path().join("src")).unwrap();
    std::fs::write(repo.path().join("src/lib.rs"), LIB_RS).unwrap();
    repo
}

fn hub_with_rust_server(repo: &TempDir, command: Vec<String>) -> LspHub {
    let registry = ServerRegistry::with_overrides(HashMap::from([("rust".to_owned(), command)]));
    LspHub::new(repo.path(), registry)
}

fn answer(mut ticket: Ticket) -> Result<Answer, LspError> {
    ticket
        .wait(PATIENCE)
        .expect("an answer within the patience window")
}

fn hover_text(hub: &mut LspHub) -> String {
    let ticket = hub.request(
        RequestKind::Hover,
        Path::new("src/lib.rs"),
        E_ACUTE_LINE,
        E_ACUTE_COLUMN,
    );
    match answer(ticket) {
        Ok(Answer::Hover(Some(Hover {
            format: HoverFormat::Markdown,
            text,
        }))) => text,
        other => panic!("expected markdown hover, got {other:?}"),
    }
}

fn wait_for_status(hub: &LspHub, wanted: impl Fn(&ServerStatus) -> bool) -> ServerStatus {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let status = hub.status_for(Path::new("src/lib.rs")).unwrap();
        if wanted(&status) || Instant::now() > deadline {
            return status;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn definition_round_trip_converts_columns_through_utf16() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, fake_server::command(Mode::Echo));
    let ticket = hub.request(
        RequestKind::Definition,
        Path::new("src/lib.rs"),
        E_ACUTE_LINE,
        E_ACUTE_COLUMN,
    );
    let expected = Location {
        path: PathBuf::from("src/lib.rs"),
        line: E_ACUTE_LINE,
        column: E_ACUTE_COLUMN,
    };
    assert_eq!(answer(ticket), Ok(Answer::Locations(vec![expected])));
    assert!(hover_text(&mut hub).starts_with("character=14 "));
}

#[test]
fn references_outside_the_repo_keep_absolute_paths() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, fake_server::command(Mode::Echo));
    let ticket = hub.request(RequestKind::References, Path::new("src/lib.rs"), 1, 3);
    let Ok(Answer::Locations(locations)) = answer(ticket) else {
        panic!("expected locations");
    };
    assert_eq!(
        locations[0],
        Location {
            path: PathBuf::from("src/lib.rs"),
            line: 1,
            column: 3
        }
    );
    assert_eq!(
        locations[1],
        Location {
            path: PathBuf::from("/nonexistent/outside dir/lib.rs"),
            line: 5,
            column: 7
        }
    );
}

#[test]
fn progress_is_reported_until_the_server_ends_it() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, fake_server::command(Mode::Progress));
    assert_eq!(
        hub.status_for(Path::new("src/lib.rs")),
        Some(ServerStatus::Idle)
    );
    hub.start_for(Path::new("src/lib.rs"));
    let indexing = ServerStatus::Indexing {
        title: "Indexing".into(),
        percentage: Some(40),
    };
    assert_eq!(
        wait_for_status(&hub, |status| *status == indexing),
        indexing
    );
    let text = hover_text(&mut hub);
    assert!(text.ends_with("create_answered=true"), "{text}");
    assert_eq!(
        hub.status_for(Path::new("src/lib.rs")),
        Some(ServerStatus::Ready)
    );
    assert_eq!(hub.servers()[0].server, "sh");
}

#[test]
fn absent_server_binary_answers_unavailable_immediately() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, vec!["rv-no-such-language-server".into()]);
    let mut ticket = hub.request(RequestKind::Definition, Path::new("src/lib.rs"), 1, 0);
    assert!(matches!(
        ticket.try_take(),
        Some(Err(LspError::Unavailable(_)))
    ));
    assert!(ticket.try_take().is_none());
    assert!(matches!(
        hub.status_for(Path::new("src/lib.rs")),
        Some(ServerStatus::Unavailable(_))
    ));
}

#[test]
fn files_without_a_configured_server_are_unavailable() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, fake_server::command(Mode::Echo));
    assert_eq!(hub.status_for(Path::new("README.md")), None);
    assert_eq!(hub.status_for(Path::new("main.go")), None);
    let mut ticket = hub.request(RequestKind::Hover, Path::new("main.go"), 1, 0);
    assert!(matches!(
        ticket.try_take(),
        Some(Err(LspError::Unavailable(_)))
    ));
}

#[test]
fn unreadable_file_is_an_error_not_a_hang() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, fake_server::command(Mode::Echo));
    let ticket = hub.request(RequestKind::Definition, Path::new("src/missing.rs"), 1, 0);
    assert!(matches!(answer(ticket), Err(LspError::Unreadable { .. })));
}

#[test]
fn server_crash_mid_request_fails_pending_and_later_requests() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, fake_server::command(Mode::DieOnDefinition));
    let ticket = hub.request(RequestKind::Definition, Path::new("src/lib.rs"), 1, 0);
    assert!(matches!(answer(ticket), Err(LspError::Unavailable(_))));
    let status = wait_for_status(&hub, |status| {
        matches!(status, ServerStatus::Unavailable(_))
    });
    assert!(matches!(status, ServerStatus::Unavailable(_)), "{status:?}");
    let ticket = hub.request(RequestKind::Hover, Path::new("src/lib.rs"), 1, 0);
    assert!(matches!(answer(ticket), Err(LspError::Unavailable(_))));
}

#[test]
fn killed_server_leaves_requests_failing_quickly() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, fake_server::command(Mode::Echo));
    let text = hover_text(&mut hub);
    let pid = text
        .split("pid=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap();
    let killed = std::process::Command::new("kill")
        .args(["-9", pid])
        .status()
        .unwrap();
    assert!(killed.success());
    let status = wait_for_status(&hub, |status| {
        matches!(status, ServerStatus::Unavailable(_))
    });
    assert!(matches!(status, ServerStatus::Unavailable(_)), "{status:?}");
    let started = Instant::now();
    let ticket = hub.request(RequestKind::Definition, Path::new("src/lib.rs"), 1, 0);
    assert!(matches!(answer(ticket), Err(LspError::Unavailable(_))));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn unresponsive_server_never_blocks_requests_or_drop() {
    let repo = repo_with_lib_rs();
    let mut hub = hub_with_rust_server(&repo, fake_server::command(Mode::Silent));
    let mut ticket = hub.request(RequestKind::Definition, Path::new("src/lib.rs"), 1, 0);
    assert!(ticket.try_take().is_none());
    assert_eq!(
        hub.status_for(Path::new("src/lib.rs")),
        Some(ServerStatus::Starting)
    );
    let started = Instant::now();
    drop(hub);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(matches!(
        ticket.wait(PATIENCE),
        Some(Err(LspError::Unavailable(_)))
    ));
}
