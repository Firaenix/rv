//! The nerd-font glyph a file row carries, chosen by what kind of file it is.
//!
//! Nerd-font glyphs live in the Private Use Area, so a font without the patch
//! shows tofu and rv cannot detect one — exactly the powerline arrows'
//! problem, so they ride the same switch: `RV_ASCII` turns them off. The
//! codepoints are the devicon/seti set every nerd-font build carries; a file
//! nothing here recognises gets Font Awesome's plain document.

use ratatui::style::Color;

/// See [`file_icon`].
pub(super) const FILE_ICON: char = '\u{f15b}';

/// The colour a folder icon takes: the muted blue every other file browser
/// draws a folder in, so the tree reads the way a reviewer's editor does.
pub(super) const DIR_COLOUR: Color = Color::Rgb(0x7f, 0xa7, 0xd6);

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

/// The colour the file's icon takes — its language's own, as the nerd-font
/// browsers paint it — or `None` for the plain document, which keeps the
/// terminal's foreground. RGB rather than a theme index for the same reason
/// the gradient is: it is a fact about the file type, not a meaning in rv's
/// chrome, and there are more types than the palette has hues.
pub(super) fn icon_colour(name: &str) -> Option<Color> {
    let base = name.rsplit('/').next().unwrap_or(name);
    let extension = base
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase());
    let rgb = match (base, extension.as_deref()) {
        ("Cargo.toml" | "Cargo.lock", _) | (_, Some("rs")) => (0xde, 0xa5, 0x84),
        ("Dockerfile", _) => (0x38, 0x9a, 0xd5),
        ("Makefile" | "makefile" | "GNUmakefile", _) => (0x6d, 0x80, 0x86),
        (_, Some("nix")) => (0x7e, 0xba, 0xe4),
        (".gitignore" | ".gitattributes" | ".gitmodules", _) => (0xf1, 0x4e, 0x32),
        (_, Some("toml")) => (0x9c, 0x4a, 0x1a),
        (_, Some("md" | "markdown")) => (0x51, 0x9a, 0xba),
        (_, Some("json" | "jsonc")) => (0xcb, 0xcb, 0x41),
        (_, Some("yml" | "yaml")) => (0xe0, 0x6c, 0x75),
        (_, Some("js" | "mjs" | "cjs")) => (0xcb, 0xcb, 0x41),
        (_, Some("ts" | "mts" | "cts")) => (0x51, 0x9a, 0xba),
        (_, Some("jsx" | "tsx")) => (0x20, 0xc2, 0xe6),
        (_, Some("vue")) => (0x8d, 0xc1, 0x49),
        (_, Some("svelte")) => (0xff, 0x3e, 0x00),
        (_, Some("py" | "pyi")) => (0xff, 0xbc, 0x03),
        (_, Some("go")) => (0x51, 0x9a, 0xba),
        (_, Some("sh" | "bash" | "zsh" | "fish")) => (0x4d, 0x5a, 0x5e),
        (_, Some("html" | "htm")) => (0xe4, 0x4d, 0x26),
        (_, Some("css")) => (0x42, 0xa5, 0xf5),
        (_, Some("scss" | "sass")) => (0xf5, 0x5b, 0x5b),
        (_, Some("c" | "h")) => (0x59, 0x9e, 0xff),
        (_, Some("cpp" | "cc" | "cxx" | "hpp" | "hh")) => (0x51, 0x9a, 0xba),
        (_, Some("java")) => (0xcc, 0x37, 0x2c),
        (_, Some("kt" | "kts")) => (0x7f, 0x52, 0xff),
        (_, Some("swift")) => (0xe3, 0x74, 0x33),
        (_, Some("rb" | "erb")) => (0x70, 0x1f, 0x16),
        (_, Some("php")) => (0xa0, 0x74, 0xc4),
        (_, Some("lua")) => (0x51, 0xa0, 0xcf),
        (_, Some("zig")) => (0xf6, 0x9a, 0x1b),
        (_, Some("hs")) => (0xa0, 0x74, 0xc4),
        (_, Some("ex" | "exs")) => (0xa0, 0x74, 0xc4),
        (_, Some("sql")) => (0xda, 0xd8, 0xd8),
        (_, Some("proto")) => (0x51, 0x9a, 0xba),
        (_, Some("graphql" | "gql")) => (0xe5, 0x35, 0xab),
        (_, Some("tf" | "hcl")) => (0x5f, 0x43, 0xe9),
        (_, Some("lock")) => (0x6d, 0x80, 0x86),
        (_, Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp")) => {
            (0xa0, 0x74, 0xc4)
        }
        (_, Some("pdf")) => (0xb3, 0x0b, 0x00),
        (_, Some("zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z")) => (0xec, 0xa5, 0x17),
        _ => return None,
    };
    Some(Color::Rgb(rgb.0, rgb.1, rgb.2))
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
