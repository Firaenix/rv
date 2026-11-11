//! A canned language server, run by re-executing this test binary through a
//! script named `rust-analyzer` (so the status bar names it as the real one
//! would). libtest prints its banner to stdout before any test body runs, so
//! the script moves stdout to stderr and hands the real pipe over as fd 3.
#![cfg(unix)]

use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rv::app::App;
use rv_core::lsp::{read_frame, write_frame};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::support::*;

const MODE_VARIABLE: &str = "RV_FAKE_LSP_MODE";
const PROGRESS_CREATE_ID: i64 = 99;

/// The doc line of the fake's hover, carrying an escape sequence a card must
/// never draw.
pub const HOVER_DOC: &str = "makes a \u{1b}[31mGamma\u{1b}[0m";

/// Not a test: the entry point the fake server runs through.
#[test]
fn fake_lsp_entry() {
    let Ok(mode) = std::env::var(MODE_VARIABLE) else {
        return;
    };
    let output = OpenOptions::new()
        .write(true)
        .open("/dev/fd/3")
        .expect("fd 3 is the client's pipe");
    serve(&mode, std::io::stdin().lock(), output);
    std::process::exit(0);
}

/// A `rust-analyzer` script in its own directory, kept alive by the guard.
pub fn fake_server(mode: &str) -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("server dir");
    let script = directory.path().join("rust-analyzer");
    let test_binary = std::env::current_exe().expect("current test binary");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nexec 3>&1 1>&2\n{MODE_VARIABLE}={mode} exec \"{}\" lsp_fake::fake_lsp_entry --exact --nocapture --test-threads=1\n",
            test_binary.display()
        ),
    )
    .expect("write the script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    (directory, script)
}

pub fn lsp_settings(command: &str) -> rv::config::Settings {
    toml::from_str(&format!(
        "lsp = \"auto\"\nlsp_servers = {{ rust = [{command:?}] }}\n"
    ))
    .expect("lsp settings parse")
}

/// The reviewer over `fixture` with the fake server in `mode`.
pub fn lsp_app(fixture: &Fixture, mode: &str) -> (App, TempDir) {
    let (guard, script) = fake_server(mode);
    let app = app_configured(fixture, "", &lsp_settings(&script.display().to_string()));
    (app, guard)
}

/// Polls until no answer is outstanding — the event loop's job, done here.
pub fn settle_lsp(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.lsp_pending() {
        assert!(Instant::now() < deadline, "the fake server never answered");
        app.collect_lsp(Instant::now());
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn wait_for_label(app: &mut App, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.collect_lsp(Instant::now());
        if app.navigation_label().as_deref() == Some(wanted) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "label stayed {:?}, wanted {wanted:?}",
            app.navigation_label()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn send(output: &mut impl Write, message: Value) {
    write_frame(output, message.to_string().as_bytes()).expect("client pipe open");
}

fn reply(output: &mut impl Write, id: &Value, result: Value) {
    send(
        output,
        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    );
}

fn progress(output: &mut impl Write, value: Value) {
    let params = json!({ "token": "indexing", "value": value });
    send(
        output,
        json!({ "jsonrpc": "2.0", "method": "$/progress", "params": params }),
    );
}

fn at(uri: &str, line: u32, character: u32) -> Value {
    let position = json!({ "line": line, "character": character });
    json!({ "uri": uri, "range": { "start": position, "end": position } })
}

/// Every answer is about the goto fixture: `g d` anywhere lands on
/// `model.rs:1` col 7 (`Gamma`), which the grammar never says for `alpha`.
fn serve(mode: &str, mut input: impl BufRead, mut output: impl Write) {
    while let Ok(Some(body)) = read_frame(&mut input) {
        let message: Value = serde_json::from_slice(&body).expect("client sends JSON");
        let id = message.get("id").cloned();
        let uri = message["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let sibling = |name: &str| {
            let directory = uri.rsplit_once('/').map_or("", |(directory, _)| directory);
            format!("{directory}/{name}")
        };
        match (message["method"].as_str(), id) {
            (Some("initialize"), Some(id)) => {
                reply(&mut output, &id, json!({ "capabilities": {} }));
            }
            (Some("initialized"), None) if mode == "progress" => {
                send(
                    &mut output,
                    json!({ "jsonrpc": "2.0", "id": PROGRESS_CREATE_ID, "method": "window/workDoneProgress/create", "params": { "token": "indexing" } }),
                );
                progress(
                    &mut output,
                    json!({ "kind": "begin", "title": "Indexing", "percentage": 10 }),
                );
                progress(&mut output, json!({ "kind": "report", "percentage": 42 }));
            }
            (Some("textDocument/definition"), Some(_)) if mode == "die_on_definition" => {
                std::process::exit(3)
            }
            (Some("textDocument/definition"), Some(id)) => {
                reply(&mut output, &id, json!([at(&sibling("model.rs"), 0, 7)]));
            }
            (Some("textDocument/references"), Some(id)) => {
                let found = json!([
                    at(&sibling("model.rs"), 0, 7),
                    at(&sibling("user.rs"), 1, 15),
                    at("file:///nonexistent/elsewhere.rs", 4, 0),
                ]);
                reply(&mut output, &id, found);
            }
            (Some("textDocument/hover"), Some(id)) => {
                let value = format!("```rust\nfn alpha() -> Gamma\n```\n\n---\n\n{HOVER_DOC}");
                reply(
                    &mut output,
                    &id,
                    json!({ "contents": { "kind": "markdown", "value": value } }),
                );
            }
            (Some("shutdown"), Some(id)) => reply(&mut output, &id, Value::Null),
            (Some("exit"), _) => std::process::exit(0),
            _ => {}
        }
    }
}
