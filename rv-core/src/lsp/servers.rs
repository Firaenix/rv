use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// `(extension, language, LSP languageId)`. The language is the key config
/// names servers by; the languageId is what `textDocument/didOpen` carries.
const LANGUAGES: &[(&str, &str, &str)] = &[
    ("rs", "rust", "rust"),
    ("go", "go", "go"),
    ("ts", "typescript", "typescript"),
    ("tsx", "typescript", "typescriptreact"),
    ("js", "typescript", "javascript"),
    ("jsx", "typescript", "javascriptreact"),
    ("mjs", "typescript", "javascript"),
    ("cjs", "typescript", "javascript"),
    ("py", "python", "python"),
    ("c", "c", "c"),
    ("h", "c", "c"),
    ("cc", "c", "cpp"),
    ("cpp", "c", "cpp"),
    ("hpp", "c", "cpp"),
    ("zig", "zig", "zig"),
    ("lua", "lua", "lua"),
    ("nix", "nix", "nix"),
    ("rb", "ruby", "ruby"),
    ("sh", "bash", "shellscript"),
    ("bash", "bash", "shellscript"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    pub name: &'static str,
    pub language_id: &'static str,
}

pub fn language_for_path(path: &Path) -> Option<Language> {
    let extension = path.extension()?.to_str()?;
    LANGUAGES
        .iter()
        .find(|(known, _, _)| known.eq_ignore_ascii_case(extension))
        .map(|&(_, name, language_id)| Language { name, language_id })
}

/// Every language name a server can be configured for, in table order.
pub fn language_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for &(_, name, _) in LANGUAGES {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Which command serves which language: what was configured, plus whatever
/// [`ServerRegistry::discovered`] found for the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerRegistry {
    commands: HashMap<String, Vec<String>>,
    disabled: HashSet<String>,
}

impl ServerRegistry {
    /// An empty command turns its language off, discovery included.
    pub fn with_overrides(overrides: HashMap<String, Vec<String>>) -> Self {
        let mut registry = Self::default();
        for (language, command) in overrides {
            if command.is_empty() {
                registry.disabled.insert(language);
            } else {
                registry.commands.insert(language, command);
            }
        }
        registry
    }

    pub fn command_for(&self, language: &str) -> Option<&[String]> {
        self.commands.get(language).map(Vec::as_slice)
    }

    pub fn is_disabled(&self, language: &str) -> bool {
        self.disabled.contains(language)
    }

    pub(super) fn set(&mut self, language: &str, command: Vec<String>) {
        self.commands.insert(language.to_owned(), command);
    }

    pub(super) fn configured_languages(&self) -> Vec<String> {
        self.commands.keys().cloned().collect()
    }
}

/// The display name for a server command: its program's file name.
pub fn server_name(command: &[String]) -> String {
    command
        .first()
        .map(|program| {
            Path::new(program).file_name().map_or_else(
                || program.clone(),
                |name| name.to_string_lossy().into_owned(),
            )
        })
        .unwrap_or_default()
}

/// A program containing a path separator is taken as a path; a bare name is
/// looked up on `PATH`, as a shell would.
pub fn find_program(program: &str) -> Option<PathBuf> {
    let as_path = Path::new(program);
    if as_path.components().count() > 1 {
        return is_executable_in(as_path);
    }
    let search_path = std::env::var_os("PATH")?;
    std::env::split_paths(&search_path)
        .flat_map(|directory| executable_candidates(&directory, program))
        .find(|candidate| is_executable(candidate))
}

fn executable_candidates(directory: &Path, program: &str) -> Vec<PathBuf> {
    let mut candidates = vec![directory.join(program)];
    if cfg!(windows) {
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        candidates.extend(
            extensions
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(|extension| directory.join(format!("{program}{extension}"))),
        );
    }
    candidates
}

/// The file itself when it is an executable, else a Windows `PATHEXT` sibling.
pub(super) fn is_executable_in(path: &Path) -> Option<PathBuf> {
    let directory = path.parent()?;
    let program = path.file_name()?.to_str()?;
    executable_candidates(directory, program)
        .into_iter()
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
