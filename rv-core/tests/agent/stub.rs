//! A canned agent CLI run by re-executing this test binary, as the fake
//! language server is: libtest's banner goes to stderr and the real stdout is
//! handed over as fd 3. The arguments rv appends reach the stub as an
//! environment variable, since the re-exec replaces them with libtest's own.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;

use serde_json::json;

const MODE: &str = "RV_STUB_AGENT_MODE";
const ARGS: &str = "RV_STUB_AGENT_ARGS";
const PID_FILE: &str = "RV_STUB_AGENT_PID_FILE";

pub const CONTRACT_REPLY: &str = "This file parses replies.\n\n```json\n{\"summary\": \"Parses replies.\", \"focus\": [\
    {\"path\": \"src/a.rs\", \"line\": 3, \"why\": \"unchecked index\", \"importance\": \"low\"},\
    {\"path\": \"src/b.rs\", \"line\": 9, \"why\": \"error dropped\", \"importance\": \"high\"}]}\n```\n";

pub fn command(mode: &str, pid_file: Option<&Path>) -> Vec<String> {
    let test_binary = std::env::current_exe().expect("current test binary");
    let pid = pid_file.map_or_else(String::new, |path| {
        format!("{PID_FILE}='{}' ", path.display())
    });
    let script = format!(
        "exec 3>&1 1>&2; {MODE}={mode} {ARGS}=\"$*\" {pid}exec \"$0\" stub_agent_entry --exact --nocapture --test-threads=1"
    );
    vec![
        "/bin/sh".into(),
        "-c".into(),
        script,
        test_binary.display().to_string(),
    ]
}

pub fn run_if_requested() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    if let Ok(path) = std::env::var(PID_FILE) {
        std::fs::write(path, std::process::id().to_string()).expect("write the pid");
    }
    let args = std::env::var(ARGS).unwrap_or_default();
    let mut prompt = String::new();
    let _ = std::io::stdin().read_to_string(&mut prompt);
    let mut out = OpenOptions::new()
        .write(true)
        .open("/dev/fd/3")
        .expect("fd 3 is the caller's pipe");
    let envelope = |result: &str, is_error: bool| {
        json!({
            "type": "result",
            "result": result,
            "session_id": "sess-42",
            "is_error": is_error,
            "duration_ms": 4123,
            "total_cost_usd": 0.0125,
        })
        .to_string()
    };
    let printed = match mode.as_str() {
        "contract" => envelope(CONTRACT_REPLY, false),
        "echo" => envelope(&format!("args=[{args}] prompt={prompt}"), false),
        "raw" => "Just prose, no JSON.\nSecond line.".to_owned(),
        "malformed" => envelope(
            "```json\n{\"summary\": \"s\", \"focus\": [\
                {\"path\": \"ok.rs\", \"line\": 2, \"why\": \"fine\"},\
                {\"path\": \"ok.rs\", \"line\": 0, \"why\": \"zero line\"},\
                {\"path\": \"/etc/passwd\", \"line\": 1, \"why\": \"absolute\"},\
                {\"path\": \"../up.rs\", \"line\": 1, \"why\": \"escapes\"},\
                {\"path\": \"ok.rs\", \"line\": \"7\", \"why\": \"string line\", \"importance\": \"HIGH\"},\
                {\"path\": \"ok.rs\", \"line\": 3},\
                {\"line\": 3, \"why\": \"no path\"},\
                \"not an object\", 17]}\n```",
            false,
        ),
        "broken_block" => envelope("Prose survives.\n```json\n{\"focus\": [\n```", false),
        "ansi" => envelope(
            "\u{1b}[31mred\u{1b}[0m \u{1b}]0;pwned\u{7}text\u{0}\n```json\n{\"summary\": \"\u{1b}[1mbold\u{1b}[0m\", \"focus\": [{\"path\": \"a.rs\", \"line\": 1, \"why\": \"\u{1b}[2Jclear\\nscreen\"}]}\n```",
            false,
        ),
        "error" => envelope("model overloaded", true),
        "fail_on_resume" if args.contains("-r ") => envelope("no such session", true),
        "fail_on_resume" => envelope(&format!("fresh args=[{args}]"), false),
        "exit_nonzero" => {
            eprintln!("boom on stderr");
            std::process::exit(2);
        }
        "sleep" => {
            std::thread::sleep(std::time::Duration::from_secs(60));
            String::new()
        }
        other => panic!("unknown stub mode {other}"),
    };
    out.write_all(printed.as_bytes()).expect("caller pipe open");
    std::process::exit(0);
}
