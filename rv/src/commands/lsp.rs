use std::path::Path;

use anyhow::Result;
use rv_core::lsp::{Discovery, SearchPath, ServerRegistry, language_names};
use rv_core::vcs::Repository;

use crate::config;

/// `rv lsp`: which server each language would use, and where it was found.
pub fn list_language_servers(repo_root: &Path) -> Result<()> {
    let settings = config::load_settings()?;
    let enabled = settings.language_servers();
    if enabled.is_none() {
        println!("lsp is off; set `lsp = \"auto\"` in Config.toml to use these");
    }
    let registry = enabled.unwrap_or_else(|| settings.language_servers_ignoring_mode());
    let repo = Repository::open(repo_root).ok();
    let is_tracked = |relative: &Path| match (&repo, relative.to_str()) {
        (Some(repo), Some(path)) => repo.tracks(path).unwrap_or(true),
        _ => false,
    };
    let search = SearchPath::for_repo(repo_root);
    for language in language_names() {
        println!(
            "{language:<11} {}",
            describe(&registry, language, &search, &is_tracked)
        );
    }
    Ok(())
}

fn describe(
    registry: &ServerRegistry,
    language: &str,
    search: &SearchPath,
    is_tracked: &dyn Fn(&Path) -> bool,
) -> String {
    match registry.discover(language, search, is_tracked) {
        Discovery::Configured(command) => match search.find(&command[0], is_tracked) {
            Some(found) => format!("{} (configured, {})", command.join(" "), found.display()),
            None => format!("{} (configured, not found)", command.join(" ")),
        },
        Discovery::Found(command) => command.join(" "),
        Discovery::Disabled => "off (configured empty)".to_owned(),
        Discovery::NotFound { tried } => format!("none found (tried {})", tried.join(", ")),
    }
}
