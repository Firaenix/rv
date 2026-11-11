use std::path::{Path, PathBuf};

use super::servers::{ServerRegistry, is_executable_in};

/// Servers tried in order for a language nobody configured; the first one
/// installed wins.
const CANDIDATES: &[(&str, &[&[&str]])] = &[
    ("rust", &[&["rust-analyzer"]]),
    ("go", &[&["gopls"]]),
    (
        "typescript",
        &[
            &["typescript-language-server", "--stdio"],
            &["vtsls", "--stdio"],
        ],
    ),
    (
        "python",
        &[
            &["basedpyright-langserver", "--stdio"],
            &["pyright-langserver", "--stdio"],
            &["pylsp"],
            &["jedi-language-server"],
        ],
    ),
    ("c", &[&["clangd"]]),
    ("zig", &[&["zls"]]),
    ("lua", &[&["lua-language-server"]]),
    ("nix", &[&["nixd"], &["nil"]]),
    ("ruby", &[&["ruby-lsp"], &["solargraph", "stdio"]]),
    ("bash", &[&["bash-language-server", "start"]]),
];

const REPO_LOCAL_DIRECTORIES: &[&str] = &["node_modules/.bin", ".venv/bin", "venv/bin"];

/// Where discovery looks: the repo's own tool directories first, then `PATH`,
/// then the per-user directories installers drop servers into without
/// necessarily putting them on `PATH`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchPath {
    repo_root: Option<PathBuf>,
    directories: Vec<PathBuf>,
}

/// A server discovery settled on, or the reason a language has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Discovery {
    Configured(Vec<String>),
    Found(Vec<String>),
    Disabled,
    NotFound { tried: Vec<&'static str> },
}

impl SearchPath {
    pub fn new(directories: Vec<PathBuf>) -> Self {
        Self {
            repo_root: None,
            directories,
        }
    }

    pub fn for_repo(repo_root: &Path) -> Self {
        let mut directories = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default();
        directories.extend(user_install_directories());
        Self {
            repo_root: Some(repo_root.to_path_buf()),
            directories,
        }
    }

    /// `is_tracked` is asked about every repo-local hit, by repo-relative
    /// path: a server committed to the repo under review is never run.
    pub fn find(&self, program: &str, is_tracked: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
        if Path::new(program).components().count() > 1 {
            return is_executable_in(Path::new(program));
        }
        let repo_local = self.repo_root.iter().flat_map(|root| {
            REPO_LOCAL_DIRECTORIES.iter().filter_map(move |directory| {
                let relative = Path::new(directory).join(program);
                let found = is_executable_in(&root.join(&relative))?;
                (!is_tracked(&relative)).then_some(found)
            })
        });
        repo_local
            .chain(
                self.directories
                    .iter()
                    .filter_map(|directory| is_executable_in(&directory.join(program))),
            )
            .next()
    }
}

fn user_install_directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let go_bin = std::env::var_os("GOPATH")
        .map(|gopath| PathBuf::from(gopath).join("bin"))
        .or_else(|| home.as_ref().map(|home| home.join("go/bin")));
    [
        home.as_ref().map(|home| home.join(".cargo/bin")),
        go_bin,
        data_home.map(|data| data.join("nvim/mason/bin")),
        home.map(|home| home.join(".local/bin")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

impl ServerRegistry {
    /// Fills every language without a configured server from [`CANDIDATES`],
    /// and pins each command's program to the file it was found at.
    #[must_use]
    pub fn discovered(self, search: &SearchPath, is_tracked: &dyn Fn(&Path) -> bool) -> Self {
        let mut registry = self;
        for language in registry.configured_languages() {
            if let Some(command) = registry.command_for(&language) {
                let pinned = pin_program(command, search, is_tracked);
                registry.set(&language, pinned);
            }
        }
        for &(language, _) in CANDIDATES {
            if let Discovery::Found(command) = registry.discover(language, search, is_tracked) {
                registry.set(language, command);
            }
        }
        registry
    }

    pub fn discover(
        &self,
        language: &str,
        search: &SearchPath,
        is_tracked: &dyn Fn(&Path) -> bool,
    ) -> Discovery {
        if self.is_disabled(language) {
            return Discovery::Disabled;
        }
        if let Some(command) = self.command_for(language) {
            return Discovery::Configured(command.to_vec());
        }
        let candidates = candidates_for(language);
        candidates
            .iter()
            .find_map(|candidate| {
                let program = search.find(candidate[0], is_tracked)?;
                let mut command = vec![program.to_string_lossy().into_owned()];
                command.extend(candidate[1..].iter().map(|&argument| argument.to_owned()));
                Some(Discovery::Found(command))
            })
            .unwrap_or_else(|| Discovery::NotFound {
                tried: candidates.iter().map(|candidate| candidate[0]).collect(),
            })
    }
}

fn candidates_for(language: &str) -> &'static [&'static [&'static str]] {
    CANDIDATES
        .iter()
        .find(|(known, _)| *known == language)
        .map_or(&[], |(_, candidates)| candidates)
}

fn pin_program(
    command: &[String],
    search: &SearchPath,
    is_tracked: &dyn Fn(&Path) -> bool,
) -> Vec<String> {
    let mut pinned = command.to_vec();
    if let Some(found) = pinned
        .first()
        .and_then(|program| search.find(program, is_tracked))
    {
        pinned[0] = found.to_string_lossy().into_owned();
    }
    pinned
}
