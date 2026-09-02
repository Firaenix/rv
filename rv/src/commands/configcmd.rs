//! `rv config` and `rv keymap`: one command per file.
//!
//! `config` opens `~/.config/rv/Config.toml` (session defaults) and `keymap`
//! opens `~/.config/rv/keybindings.toml` — each seeding a fully-commented
//! template on first run, so the vocabulary is met in an editor rather than
//! in an error, and each validating the moment the editor exits, because a
//! typo found now is a typo not found at the next review. `keymap --show`
//! prints the *effective* map, defaults and patch merged, which is the answer
//! to "why does q do nothing" after a rebind moved it.

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use rv::app::Keymap;
use rv::config;

/// What a fresh `Config.toml` opens on: every field, commented out.
const SETTINGS_TEMPLATE: &str = "\
# rv session defaults — every field optional; delete a line to restore the
# built-in. A flag or environment variable (--no-difft, RV_ASCII) wins over
# this file for its run.

# tree = true            # open the file list as a tree
# sort = \"added\"         # natural | added | removed
# full_context = false   # open on changes-only
# grouped = true         # removals before additions in each hunk
# tint = false           # no proportion tint on sidebar rows
# counts = false         # no +n -n column
# sidebar_hidden = true  # open with the sidebar put away
# split = 40             # sidebar share of the width, in percent
# ascii = true           # ASCII status-bar separators
# engine = \"fallback\"    # auto | fallback
# auto_refresh = false    # stop refreshing when the repo moves
";

/// `rv config`: the session defaults, in `$EDITOR`.
pub fn edit_config() -> Result<()> {
    let path = config::settings_path()
        .context("could not work out the config directory on this system")?;
    edit(&path, SETTINGS_TEMPLATE)?;
    config::load_settings()?;
    println!("settings ok");
    Ok(())
}

/// `rv keymap`: the keybindings, in `$EDITOR`.
pub fn edit_keymap() -> Result<()> {
    let path =
        config::config_path().context("could not work out the config directory on this system")?;
    edit(&path, &Keymap::template())?;
    let mut keymap = Keymap::from_config(&config::load()?)?;
    println!("keybindings ok");
    for warning in keymap.take_warnings() {
        println!("note: {warning}");
    }
    Ok(())
}

/// `rv keymap --show`: the effective keymap, defaults and patch merged.
pub fn print_keymap() -> Result<()> {
    let config = config::load()?;
    let mut keymap = Keymap::from_config(&config)?;
    for warning in keymap.take_warnings() {
        eprintln!("note: {warning}");
    }
    print!("{}", keymap.render_toml(false));
    Ok(())
}

/// Seeds `path` with `template` when it does not exist yet, and hands it to
/// `$EDITOR` — the TUI's own convention (`app/editor.rs`): named rather than
/// guessed, because a default of `vi` would trade one sentence for a modal
/// editor some users cannot leave.
fn edit(path: &Path, template: &str) -> Result<()> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        std::fs::write(path, template)
            .with_context(|| format!("could not write {}", path.display()))?;
        println!("seeded {}", path.display());
    }
    let Some(editor) = std::env::var_os("EDITOR") else {
        bail!(
            "$EDITOR is not set — set it, or edit {} directly",
            path.display()
        );
    };
    let status = std::process::Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("could not run {}", editor.display()))?;
    if !status.success() {
        bail!(
            "{} exited unsuccessfully; the file was not validated",
            editor.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template's commented lines are the real vocabulary: uncommented,
    /// every one of them parses — so the seed cannot drift from the fields.
    #[test]
    fn the_settings_template_uncommented_is_a_valid_config() {
        let uncommented: String = SETTINGS_TEMPLATE
            .lines()
            .filter(|line| line.starts_with("# ") && line.contains('='))
            .map(|line| format!("{}\n", &line[2..]))
            .collect();
        let settings: rv::config::Settings =
            toml::from_str(&uncommented).expect("every commented field parses");
        assert_eq!(settings.tree, Some(true));
        assert_eq!(settings.split, Some(40));
        assert!(matches!(
            settings.engine,
            Some(rv::config::EngineName::Fallback)
        ));
    }
}
