//! `rv brief --ask` against a stub agent: a shell script that records its
//! arguments and prints a canned Claude-Code-shaped reply.

use std::path::Path;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;

use super::support::*;

struct Stub {
    home: TempDir,
}

impl Stub {
    /// A config home whose `Config.toml` names `command`.
    fn configured(command: &[&str]) -> Self {
        let stub = Self {
            home: tempfile::tempdir().expect("config home"),
        };
        stub.configure(command);
        stub
    }

    fn configure(&self, command: &[&str]) {
        let quoted: Vec<String> = command.iter().map(|word| format!("{word:?}")).collect();
        let rv_dir = self.dir().join("rv");
        std::fs::create_dir_all(&rv_dir).expect("rv config dir");
        std::fs::write(
            rv_dir.join("Config.toml"),
            format!(
                "[agent]\ncommand = [{}]\nallowed_tools = \"Read\"\ntimeout_secs = 20\n",
                quoted.join(", ")
            ),
        )
        .expect("write Config.toml");
    }

    /// A config home holding the stub script too, configured to run it.
    fn scripted() -> Self {
        let stub = Self {
            home: tempfile::tempdir().expect("stub dir"),
        };
        let script = stub.dir().join("agent.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncat > '{dir}/prompt.txt'\necho \"$*\" >> '{dir}/args.log'\ncat '{dir}/reply.json'\n",
                dir = stub.dir().display()
            ),
        )
        .expect("write the stub");
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("make it executable");
        stub.configure(&[&script.display().to_string()]);
        stub
    }

    fn dir(&self) -> &Path {
        self.home.path()
    }

    fn reply_with(&self, focus: &str) {
        let result = format!(
            "The change looks fine.\n\n```json\n{{\"summary\": \"Looks fine.\", \"focus\": [{focus}]}}\n```"
        );
        let envelope = serde_json::json!({
            "type": "result",
            "result": result,
            "session_id": "sess-xyz",
            "is_error": false,
            "duration_ms": 2500,
            "total_cost_usd": 0.0421,
        });
        std::fs::write(self.dir().join("reply.json"), envelope.to_string()).expect("write reply");
    }

    fn args(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir().join("args.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn rv(&self, workspace: &Fixture, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rv"))
            .args(args)
            .current_dir(workspace.root())
            .env("XDG_CONFIG_HOME", self.dir())
            .output()
            .expect("run rv")
    }
}

fn focus(path: &str, line: u32, why: &str) -> String {
    format!(
        "{{\"path\": \"{path}\", \"line\": {line}, \"why\": \"{why}\", \"importance\": \"high\"}}"
    )
}

fn workspace() -> Fixture {
    let workspace = Fixture::new();
    workspace.write("a.rs", "fn a() {\n    let x = 1;\n}\n");
    workspace.commit("first change");
    workspace
}

fn agent_flags(workspace: &Fixture) -> Vec<(String, u32, String)> {
    workspace
        .store()
        .flags()
        .expect("flags")
        .into_iter()
        .filter(|flag| flag.by == rv_core::store::FlaggedBy::Agent)
        .map(|flag| (flag.anchor.file, flag.anchor.line, flag.reason))
        .collect()
}

#[test]
fn the_answer_is_printed_and_its_focus_becomes_agent_flags() {
    let workspace = workspace();
    let stub = Stub::scripted();
    stub.reply_with(
        &[
            focus("a.rs", 2, "x is never used"),
            focus("elsewhere.rs", 1, "not in the review"),
            focus("a.rs", 99, "past the end"),
        ]
        .join(","),
    );

    let output = stub.rv(&workspace, &["brief", "--ask"]);
    assert!(output.status.success(), "{}", streams(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("The change looks fine."), "{stdout}");
    assert!(!stdout.contains("```json"), "{stdout}");
    assert!(
        stdout.contains("2.5s") && stdout.contains("$0.0421"),
        "{stdout}"
    );
    assert!(
        stdout.contains("flagged") && stdout.contains("a.rs:2"),
        "{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("skipped elsewhere.rs:1"), "{stderr}");
    assert!(stderr.contains("skipped a.rs:99"), "{stderr}");

    assert_eq!(
        agent_flags(&workspace),
        [("a.rs".to_owned(), 2, "x is never used".to_owned())]
    );
    let prompt = std::fs::read_to_string(stub.dir().join("prompt.txt")).expect("prompt");
    assert!(
        prompt.contains("# Review brief") && prompt.contains("\"focus\""),
        "{prompt}"
    );
    assert_eq!(stub.args(), ["--allowedTools Read"]);

    let listed = stub.rv(&workspace, &["flags", "--json"]);
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).expect("json");
    assert_eq!(listed[0]["by"], "agent", "{listed}");
}

#[test]
fn a_second_ask_resumes_the_session_and_files_no_duplicate() {
    let workspace = workspace();
    let stub = Stub::scripted();
    stub.reply_with(&focus("a.rs", 2, "x is never used"));
    assert!(stub.rv(&workspace, &["brief", "--ask"]).status.success());
    assert_eq!(
        workspace.store().agent_session().expect("read"),
        Some("sess-xyz".to_owned())
    );

    let again = stub.rv(&workspace, &["brief", "--ask"]);
    assert!(again.status.success(), "{}", streams(&again));
    assert!(String::from_utf8_lossy(&again.stdout).contains("1 already flagged"));
    assert_eq!(stub.args()[1], "--allowedTools Read -r sess-xyz");
    assert_eq!(agent_flags(&workspace).len(), 1);

    workspace.write("a.rs", "// one\n// two\nfn a() {\n    let x = 1;\n}\n");
    workspace.commit("two lines above");
    stub.reply_with(&focus("a.rs", 4, "x is never used"));
    assert!(stub.rv(&workspace, &["brief", "--ask"]).status.success());
    assert_eq!(
        agent_flags(&workspace).len(),
        1,
        "the moved flag is the same flag"
    );
}

#[test]
fn open_agent_flags_do_not_fail_the_check() {
    let workspace = workspace();
    let stub = Stub::scripted();
    stub.reply_with(&focus("a.rs", 1, "start here"));
    assert!(stub.rv(&workspace, &["brief", "--ask"]).status.success());
    assert_eq!(agent_flags(&workspace).len(), 1);
    let check = stub.rv(&workspace, &["status", "--check"]);
    assert!(check.status.success(), "{}", streams(&check));
}

#[test]
fn without_an_agent_the_ask_is_a_clear_error_and_writes_nothing() {
    let workspace = workspace();
    let unconfigured = tempfile::tempdir().expect("empty config home");
    let output = Command::new(env!("CARGO_BIN_EXE_rv"))
        .args(["brief", "--ask"])
        .current_dir(workspace.root())
        .env("XDG_CONFIG_HOME", unconfigured.path())
        .output()
        .expect("run rv");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no agent is configured"),
        "{}",
        streams(&output)
    );

    let stub = Stub::scripted();
    stub.reply_with("");
    let refused = stub.rv(&workspace, &["--no-agent", "brief", "--ask"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--no-agent"));
    assert!(stub.args().is_empty(), "the agent ran");
}

#[test]
fn an_absent_agent_binary_is_reported_not_a_crash() {
    let workspace = workspace();
    let stub = Stub::configured(&["rv-no-such-agent-binary", "-p"]);
    let output = stub.rv(&workspace, &["brief", "--ask"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`rv-no-such-agent-binary` could not be run"),
        "{stderr}"
    );
}
