use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;

use super::support::*;

fn install(directory: &Path, program: &str) {
    std::fs::create_dir_all(directory).expect("bin dir");
    let path = directory.join(program);
    std::fs::write(&path, "#!/bin/sh\n").expect("write the server");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// `rv lsp` with only `bin` on PATH and an empty home, so nothing the
/// developer installed is discovered.
fn rv_lsp(workspace: &Fixture, bin: &Path, home: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rv"))
        .arg("lsp")
        .current_dir(workspace.root())
        .env("PATH", bin)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_DATA_HOME", home)
        .env_remove("GOPATH")
        .output()
        .expect("run rv lsp");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).expect("utf-8")
}

fn line_for<'a>(listing: &'a str, language: &str) -> &'a str {
    listing
        .lines()
        .find(|line| line.split_whitespace().next() == Some(language))
        .unwrap_or_else(|| panic!("no {language} line in {listing}"))
}

#[test]
fn rv_lsp_names_the_discovered_server_and_what_was_tried_elsewhere() {
    let workspace = Fixture::new();
    let tools = tempfile::tempdir().expect("tools");
    install(tools.path(), "gopls");

    let listing = rv_lsp(&workspace, tools.path(), tools.path());

    assert!(listing.contains("lsp is off"), "{listing}");
    assert!(
        line_for(&listing, "go").contains(&tools.path().join("gopls").display().to_string()),
        "{listing}"
    );
    assert!(
        line_for(&listing, "typescript").contains("tried typescript-language-server, vtsls"),
        "{listing}"
    );
}

#[test]
fn a_repo_local_server_is_discovered_only_when_jj_does_not_track_it() {
    let workspace = Fixture::new();
    workspace.write(".gitignore", "node_modules/\n");
    install(&workspace.root().join(".venv/bin"), "pylsp");
    workspace.commit("commit a python server");
    install(&workspace.root().join("node_modules/.bin"), "vtsls");
    let empty = tempfile::tempdir().expect("empty");

    let listing = rv_lsp(&workspace, empty.path(), empty.path());

    assert!(
        line_for(&listing, "typescript").contains("node_modules/.bin/vtsls --stdio"),
        "{listing}"
    );
    assert!(
        line_for(&listing, "python").contains("none found"),
        "{listing}"
    );
}
