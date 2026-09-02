//! `rv config` and `rv keymap`: the two doors into the keybindings file.
//!
//! `config` opens the file in `$EDITOR` — seeding the fully-commented default
//! template first, so the vocabulary is met in an editor rather than in an
//! error — and validates the result the moment the editor exits, because a
//! typo found now is a typo not found at the next review. `keymap` prints the
//! *effective* map, defaults and patch merged, which is the answer to "why
//! does q do nothing" after a rebind moved it.

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use rv::app::Keymap;
use rv::config;

pub fn edit_config() -> Result<()> {
    let path =
        config::config_path().context("could not work out the config directory on this system")?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        std::fs::write(&path, Keymap::template())
            .with_context(|| format!("could not write {}", path.display()))?;
        println!("seeded {}", path.display());
    }

    // The TUI's own convention (`app/editor.rs`): `$EDITOR`, named rather
    // than guessed — a default of `vi` would trade one sentence for a modal
    // editor some users cannot leave.
    let Some(editor) = std::env::var_os("EDITOR") else {
        bail!(
            "$EDITOR is not set — set it, or edit {} directly",
            path.display()
        );
    };
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("could not run {}", editor.display()))?;
    if !status.success() {
        bail!(
            "{} exited unsuccessfully; the file was not validated",
            editor.display()
        );
    }

    match validate() {
        Ok(warnings) => {
            println!("keybindings ok");
            for warning in warnings {
                println!("note: {warning}");
            }
            Ok(())
        }
        Err(error) => Err(error.context("the keybindings file does not load")),
    }
}

pub fn print_keymap() -> Result<()> {
    let config = config::load()?;
    let mut keymap = Keymap::from_config(&config)?;
    for warning in keymap.take_warnings() {
        eprintln!("note: {warning}");
    }
    print!("{}", keymap.render_toml(false));
    Ok(())
}

fn validate() -> Result<Vec<String>> {
    let config = config::load()?;
    let mut keymap = Keymap::from_config(&config)?;
    Ok(keymap.take_warnings())
}
