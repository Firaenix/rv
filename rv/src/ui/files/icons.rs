//! The nerd-font glyph a file row carries, chosen by what kind of file it is.
//!
//! Nerd-font glyphs live in the Private Use Area, so a font without the patch
//! shows tofu and rv cannot detect one — exactly the powerline arrows'
//! problem, so they ride the same switch: `RV_ASCII` turns them off. The
//! codepoints are the devicon/seti set every nerd-font build carries; a file
//! nothing here recognises gets Font Awesome's plain document.

/// See [`file_icon`].
pub(super) const FILE_ICON: char = '\u{f15b}';
/// Font Awesome's folder-open glyph.
pub(super) const DIR_ICON_OPEN: char = '\u{f07c}';
/// Font Awesome's folder glyph.
pub(super) const DIR_ICON_FOLDED: char = '\u{f07b}';

/// The glyph for the file `name` names — a path or a bare name, either works,
/// since only the last component and its extension are looked at.
pub(super) fn file_icon(name: &str) -> char {
    let base = name.rsplit('/').next().unwrap_or(name);
    if let Some(icon) = by_name(base) {
        return icon;
    }
    let extension = base
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase());
    extension
        .as_deref()
        .and_then(by_extension)
        .unwrap_or(FILE_ICON)
}

fn by_name(base: &str) -> Option<char> {
    Some(match base {
        "Cargo.toml" | "Cargo.lock" => '\u{e7a8}',
        "Dockerfile"
        | "docker-compose.yml"
        | "docker-compose.yaml"
        | "compose.yml"
        | "compose.yaml" => '\u{e7b0}',
        "Makefile" | "makefile" | "GNUmakefile" => '\u{e779}',
        "flake.nix" | "flake.lock" => '\u{f313}',
        ".gitignore" | ".gitattributes" | ".gitmodules" => '\u{e702}',
        "LICENSE" | "LICENSE-MIT" | "LICENSE-APACHE" | "COPYING" => '\u{e60a}',
        _ => return None,
    })
}

fn by_extension(extension: &str) -> Option<char> {
    Some(match extension {
        "rs" => '\u{e7a8}',
        "toml" => '\u{e6b2}',
        "md" | "markdown" => '\u{e73e}',
        "json" | "jsonc" => '\u{e60b}',
        "yml" | "yaml" => '\u{e6a8}',
        "js" | "mjs" | "cjs" => '\u{e74e}',
        "ts" | "mts" | "cts" => '\u{e628}',
        "jsx" | "tsx" => '\u{e7ba}',
        "vue" => '\u{e6a0}',
        "svelte" => '\u{e697}',
        "py" | "pyi" => '\u{e73c}',
        "go" => '\u{e627}',
        "sh" | "bash" | "zsh" | "fish" => '\u{e795}',
        "nix" => '\u{f313}',
        "html" | "htm" => '\u{e736}',
        "css" => '\u{e749}',
        "scss" | "sass" => '\u{e603}',
        "c" | "h" => '\u{e61e}',
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => '\u{e61d}',
        "java" => '\u{e738}',
        "kt" | "kts" => '\u{e634}',
        "swift" => '\u{e755}',
        "rb" | "erb" => '\u{e739}',
        "php" => '\u{e73d}',
        "lua" => '\u{e620}',
        "zig" => '\u{e6a9}',
        "hs" => '\u{e777}',
        "ex" | "exs" => '\u{e62d}',
        "erl" => '\u{e7b1}',
        "scala" => '\u{e737}',
        "clj" | "cljs" => '\u{e768}',
        "dart" => '\u{e798}',
        "sql" => '\u{e706}',
        "proto" => '\u{e60b}',
        "graphql" | "gql" => '\u{e662}',
        "tf" | "hcl" => '\u{e69a}',
        "lock" => '\u{f023}',
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" => '\u{e60d}',
        "txt" => '\u{f15c}',
        "xml" | "plist" => '\u{e619}',
        "csv" | "tsv" => '\u{f1c3}',
        "pdf" => '\u{f1c1}',
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" => '\u{f1c6}',
        _ => return None,
    })
}
