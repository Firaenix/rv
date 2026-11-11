//! A canned agent CLI for the reviewer: a shell script named `maki` (so the
//! bar names it as it would the real one) that logs each call and prints the
//! Claude-Code-shaped reply the test wrote — or sleeps, to be stopped.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use rv::app::App;
use tempfile::TempDir;

use crate::support::*;

pub struct Stub {
    dir: TempDir,
}

impl Stub {
    pub fn new() -> Self {
        let stub = Self {
            dir: tempfile::tempdir().expect("stub dir"),
        };
        let dir = stub.dir.path().display();
        std::fs::write(
            stub.script(),
            format!(
                "#!/bin/sh\ncat > '{dir}/prompt.txt'\necho \"$*\" >> '{dir}/calls.log'\n\
                 if [ -f '{dir}/sleep' ]; then exec sleep 30; fi\ncat '{dir}/reply.json'\n"
            ),
        )
        .expect("write the stub");
        std::fs::set_permissions(stub.script(), std::fs::Permissions::from_mode(0o755))
            .expect("make it executable");
        stub.reply("Nothing stands out.", "Nothing stands out.", &[]);
        stub
    }

    pub fn script(&self) -> PathBuf {
        self.dir.path().join("maki")
    }

    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// The next reply: prose, a summary, and focus entries `(path, line, why)`.
    pub fn reply(&self, prose: &str, summary: &str, focus: &[(&str, u32, &str)]) {
        let focus: Vec<serde_json::Value> = focus
            .iter()
            .map(|(path, line, why)| {
                serde_json::json!({"path": path, "line": line, "why": why, "importance": "high"})
            })
            .collect();
        let block = serde_json::json!({"summary": summary, "focus": focus});
        let envelope = serde_json::json!({
            "type": "result",
            "result": format!("{prose}\n\n```json\n{block}\n```"),
            "session_id": "sess-tui",
            "is_error": false,
            "duration_ms": 2500,
            "total_cost_usd": 0.0421,
        });
        std::fs::write(self.dir().join("reply.json"), envelope.to_string()).expect("write reply");
    }

    pub fn hang(&self) {
        std::fs::write(self.dir().join("sleep"), "").expect("mark the stub sleepy");
    }

    pub fn calls(&self) -> usize {
        std::fs::read_to_string(self.dir().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    pub fn prompt(&self) -> String {
        std::fs::read_to_string(self.dir().join("prompt.txt")).unwrap_or_default()
    }
}

pub fn agent_settings(command: &Path, orientation: &str) -> rv::config::Settings {
    toml::from_str(&format!(
        "[agent]\ncommand = [{:?}]\norientation = {orientation:?}\ntimeout_secs = 20\n",
        command.display().to_string()
    ))
    .expect("agent settings parse")
}

/// The reviewer over `fixture` with the stub as its agent.
pub fn agent_app(fixture: &Fixture, stub: &Stub, orientation: &str, keys: &str) -> App {
    let mut app = app_configured(fixture, keys, &agent_settings(&stub.script(), orientation));
    settle_agent(&mut app);
    app
}

/// Polls until no ask is running — the event loop's job, done here.
pub fn settle_agent(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.collect_agent(Instant::now());
        if !app.agent_busy() {
            return;
        }
        assert!(Instant::now() < deadline, "the stub agent never answered");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The agent's flags as the store holds them: `(file, line, reason)`.
pub fn stored_agent_flags(fixture: &Fixture) -> Vec<(String, u32, String)> {
    fixture
        .store()
        .flags()
        .expect("flags")
        .into_iter()
        .filter(|flag| flag.by == rv_core::store::FlaggedBy::Agent)
        .map(|flag| (flag.anchor.file, flag.anchor.line, flag.reason))
        .collect()
}
