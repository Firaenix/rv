//! `rv completions <shell>`: a script per shell, generated from the real CLI.

use super::support::*;

/// Every shipped shell generates a script that names the binary and at least
/// one subcommand — the two things a stale or hand-written script gets wrong.
#[test]
fn every_shell_generates_a_script_naming_the_subcommands() {
    let workspace = Fixture::new();

    for shell in ["bash", "zsh", "fish"] {
        let output = workspace.rv(&["completions", shell]);
        assert!(output.status.success(), "{}", streams(&output));
        let script = String::from_utf8_lossy(&output.stdout);
        assert!(
            script.contains("rv") && script.contains("comment"),
            "the {shell} script does not describe rv's commands:\n{script}"
        );
    }
}

/// Completions describe the command line, not a review, so they must not need a
/// workspace to be in — `cargo install` then one shell line, anywhere.
#[test]
fn completions_need_no_jj_workspace() {
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rv"))
        .args(["completions", "zsh"])
        .current_dir(elsewhere.path())
        .output()
        .expect("run rv");

    assert!(output.status.success(), "{}", streams(&output));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("#compdef rv"),
        "not a zsh completion script"
    );
}
