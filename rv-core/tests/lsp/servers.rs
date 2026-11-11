use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rv_core::lsp::{Discovery, SearchPath, ServerRegistry, find_program, language_for_path};

#[test]
fn extensions_map_to_languages_and_language_ids() {
    let tsx = language_for_path(Path::new("web/App.TSX")).unwrap();
    assert_eq!(
        (tsx.name, tsx.language_id),
        ("typescript", "typescriptreact")
    );
    assert_eq!(
        language_for_path(Path::new("src/lib.rs")).unwrap().name,
        "rust"
    );
    assert_eq!(language_for_path(Path::new("README.md")), None);
    assert_eq!(language_for_path(Path::new("Makefile")), None);
}

#[test]
fn overrides_add_replace_and_disable_servers() {
    let overrides = HashMap::from([
        ("go".to_owned(), vec!["gopls".to_owned()]),
        (
            "python".to_owned(),
            vec!["pyright-langserver".to_owned(), "--stdio".to_owned()],
        ),
        ("rust".to_owned(), vec![]),
    ]);
    let registry = ServerRegistry::with_overrides(overrides);
    assert_eq!(registry.command_for("go"), Some(&["gopls".to_owned()][..]));
    assert_eq!(registry.command_for("python").map(<[String]>::len), Some(2));
    assert_eq!(registry.command_for("rust"), None);
    assert!(registry.is_disabled("rust"));
}

fn install(directory: &Path, program: &str) -> PathBuf {
    let path = directory.join(program);
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(&path, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn untracked(_: &Path) -> bool {
    false
}

#[test]
fn discovery_takes_the_first_installed_candidate_with_its_arguments() {
    let bin = tempfile::tempdir().unwrap();
    install(bin.path(), "rust-analyzer");
    let pylsp = install(bin.path(), "pylsp");
    let search = SearchPath::new(vec![bin.path().to_path_buf()]);
    let registry = ServerRegistry::default().discovered(&search, &untracked);
    assert_eq!(
        registry.command_for("python"),
        Some(&[pylsp.to_string_lossy().into_owned()][..])
    );
    assert!(registry.command_for("rust").is_some());
    assert_eq!(registry.command_for("go"), None);

    let pyright = install(bin.path(), "pyright-langserver");
    let registry = ServerRegistry::default().discovered(&search, &untracked);
    assert_eq!(
        registry.command_for("python").unwrap(),
        [pyright.to_string_lossy().into_owned(), "--stdio".to_owned()]
    );
}

#[test]
fn a_disabled_or_configured_language_is_never_discovered_over() {
    let bin = tempfile::tempdir().unwrap();
    let gopls = install(bin.path(), "gopls");
    install(bin.path(), "rust-analyzer");
    let search = SearchPath::new(vec![bin.path().to_path_buf()]);
    let overrides = HashMap::from([
        ("rust".to_owned(), vec![]),
        (
            "go".to_owned(),
            vec!["gopls".to_owned(), "serve".to_owned()],
        ),
    ]);
    let registry = ServerRegistry::with_overrides(overrides).discovered(&search, &untracked);
    assert_eq!(registry.command_for("rust"), None);
    assert_eq!(
        registry.discover("rust", &search, &untracked),
        Discovery::Disabled
    );
    assert_eq!(
        registry.command_for("go").unwrap(),
        [gopls.to_string_lossy().into_owned(), "serve".to_owned()]
    );
}

#[test]
fn a_language_with_no_installed_server_reports_what_was_tried() {
    let search = SearchPath::new(vec![]);
    let registry = ServerRegistry::default();
    assert_eq!(
        registry.discover("typescript", &search, &untracked),
        Discovery::NotFound {
            tried: vec!["typescript-language-server", "vtsls"]
        }
    );
}

#[test]
fn a_repo_local_server_is_used_only_when_untracked() {
    let repo = tempfile::tempdir().unwrap();
    let local = install(&repo.path().join("node_modules/.bin"), "vtsls");
    let search = SearchPath::for_repo(repo.path());
    assert_eq!(search.find("vtsls", &untracked), Some(local));
    let tracked = |relative: &Path| relative == Path::new("node_modules/.bin/vtsls");
    assert_ne!(
        search.find("vtsls", &tracked).as_deref(),
        Some(repo.path().join("node_modules/.bin/vtsls").as_path())
    );
}

#[test]
fn programs_are_found_on_path_or_by_explicit_path() {
    assert!(find_program("sh").is_some());
    assert_eq!(
        find_program("/bin/sh").as_deref(),
        Some(Path::new("/bin/sh"))
    );
    assert_eq!(find_program("rv-no-such-language-server"), None);
    assert_eq!(find_program("/nonexistent/rust-analyzer"), None);
}
