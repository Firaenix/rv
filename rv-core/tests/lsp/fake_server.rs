//! A canned language server run by re-executing this test binary. libtest
//! prints its banner to stdout before any test body runs, so the wrapper
//! shell moves stdout to stderr and hands the real pipe over as fd 3.

use std::fs::OpenOptions;
use std::io::{BufRead, Write};

use rv_core::lsp::{read_frame, write_frame};
use serde_json::{Value, json};

const MODE_VARIABLE: &str = "RV_FAKE_LSP_MODE";
pub const PROGRESS_CREATE_ID: i64 = 99;

#[derive(Clone, Copy)]
pub enum Mode {
    Echo,
    Progress,
    DieOnDefinition,
    Silent,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Echo => "echo",
            Mode::Progress => "progress",
            Mode::DieOnDefinition => "die_on_definition",
            Mode::Silent => "silent",
        }
    }
}

pub fn command(mode: Mode) -> Vec<String> {
    let test_binary = std::env::current_exe().expect("current test binary");
    let script = format!(
        "exec 3>&1 1>&2; {MODE_VARIABLE}={} exec \"$0\" fake_lsp_entry --exact --nocapture --test-threads=1",
        mode.name()
    );
    vec![
        "/bin/sh".into(),
        "-c".into(),
        script,
        test_binary.display().to_string(),
    ]
}

pub fn run_if_requested() {
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

fn serve(mode: &str, mut input: impl BufRead, mut output: impl Write) {
    let mut create_answered = false;
    while let Ok(Some(body)) = read_frame(&mut input) {
        let message: Value = serde_json::from_slice(&body).expect("client sends JSON");
        let id = message.get("id").cloned();
        let params = &message["params"];
        let uri = params["textDocument"]["uri"].clone();
        let position = params["position"].clone();
        match (message["method"].as_str(), id) {
            (Some("initialize"), _) if mode == "silent" => {}
            (Some("initialize"), Some(id)) => {
                reply(&mut output, &id, json!({ "capabilities": {} }))
            }
            (Some("initialized"), None) if mode == "progress" => {
                let create = json!({ "token": "indexing" });
                send(
                    &mut output,
                    json!({ "jsonrpc": "2.0", "id": PROGRESS_CREATE_ID, "method": "window/workDoneProgress/create", "params": create }),
                );
                progress(
                    &mut output,
                    json!({ "kind": "begin", "title": "Indexing", "percentage": 10 }),
                );
                progress(&mut output, json!({ "kind": "report", "percentage": 40 }));
            }
            (Some("textDocument/definition"), Some(_)) if mode == "die_on_definition" => {
                std::process::exit(3)
            }
            (Some("textDocument/definition"), Some(id)) => {
                let range = json!({ "start": position, "end": position });
                let link = json!({ "targetUri": uri, "targetRange": range, "targetSelectionRange": range });
                reply(&mut output, &id, json!([link]));
            }
            (Some("textDocument/references"), Some(id)) => {
                let outside = json!({ "line": 4, "character": 7 });
                let locations = json!([
                    { "uri": uri, "range": { "start": position, "end": position } },
                    { "uri": "file:///nonexistent/outside%20dir/lib.rs", "range": { "start": outside, "end": outside } },
                ]);
                reply(&mut output, &id, locations);
            }
            (Some("textDocument/hover"), Some(id)) => {
                if mode == "progress" {
                    progress(&mut output, json!({ "kind": "end" }));
                }
                let text = format!(
                    "character={} pid={} create_answered={create_answered}",
                    position["character"],
                    std::process::id()
                );
                reply(
                    &mut output,
                    &id,
                    json!({ "contents": { "kind": "markdown", "value": text } }),
                );
            }
            (Some("shutdown"), Some(id)) => reply(&mut output, &id, Value::Null),
            (Some("exit"), _) if mode == "silent" => {}
            (Some("exit"), _) => std::process::exit(0),
            (None, Some(id)) if id == json!(PROGRESS_CREATE_ID) => create_answered = true,
            _ => {}
        }
    }
    if mode == "silent" {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
