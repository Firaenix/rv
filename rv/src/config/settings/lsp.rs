use std::collections::BTreeMap;

use rv_core::lsp::ServerRegistry;
use serde::Deserialize;

use super::Settings;

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LspMode {
    Off,
    Auto,
}

/// A server as configured: a string is split on whitespace, an array is
/// taken verbatim (for a path with spaces in it).
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum ServerCommand {
    Line(String),
    Argv(Vec<String>),
}

/// Commands by language name; an empty command turns that language off.
#[derive(Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(try_from = "BTreeMap<String, ServerCommand>")]
pub struct LspServers(BTreeMap<String, Vec<String>>);

impl TryFrom<BTreeMap<String, ServerCommand>> for LspServers {
    type Error = String;

    fn try_from(raw: BTreeMap<String, ServerCommand>) -> Result<Self, String> {
        let known = rv_core::lsp::language_names();
        let mut servers = BTreeMap::new();
        for (language, command) in raw {
            if !known.contains(&language.as_str()) {
                let hint = crate::config::closest(&language, &known)
                    .map(|near| format!(" — did you mean `{near}`?"))
                    .unwrap_or_default();
                return Err(format!(
                    "lsp_servers: unknown language `{language}`{hint} (known: {})",
                    known.join(", ")
                ));
            }
            let argv = match command {
                ServerCommand::Line(line) => line.split_whitespace().map(str::to_owned).collect(),
                ServerCommand::Argv(argv) => argv,
            };
            servers.insert(language, argv);
        }
        Ok(Self(servers))
    }
}

impl Settings {
    /// The servers to navigate with, or `None` unless `lsp = "auto"`.
    #[must_use]
    pub fn language_servers(&self) -> Option<ServerRegistry> {
        (self.lsp == Some(LspMode::Auto)).then(|| self.language_servers_ignoring_mode())
    }

    /// The configured servers whether or not `lsp` is on, for `rv lsp`.
    #[must_use]
    pub fn language_servers_ignoring_mode(&self) -> ServerRegistry {
        let overrides = self
            .lsp_servers
            .clone()
            .unwrap_or_default()
            .0
            .into_iter()
            .collect();
        ServerRegistry::with_overrides(overrides)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Settings, toml::de::Error> {
        toml::from_str(text)
    }

    #[test]
    fn lsp_is_off_unless_asked_for() {
        assert!(parse("").unwrap().language_servers().is_none());
        assert!(
            parse("lsp = \"off\"\n")
                .unwrap()
                .language_servers()
                .is_none()
        );
        let auto = parse("lsp = \"auto\"\n")
            .unwrap()
            .language_servers()
            .unwrap();
        assert_eq!(auto, ServerRegistry::default());
    }

    #[test]
    fn a_server_is_a_string_or_an_array() {
        let settings = parse(
            "lsp = \"auto\"\nlsp_servers = { rust = \"ra --log x\", go = [\"/opt/my gopls\"] }\n",
        )
        .unwrap();
        let registry = settings.language_servers().unwrap();
        assert_eq!(
            registry.command_for("rust").unwrap(),
            ["ra".to_owned(), "--log".to_owned(), "x".to_owned()]
        );
        assert_eq!(
            registry.command_for("go").unwrap(),
            ["/opt/my gopls".to_owned()]
        );
    }

    #[test]
    fn an_empty_server_turns_its_language_off() {
        let settings = parse("lsp = \"auto\"\nlsp_servers = { rust = \"\" }\n").unwrap();
        assert!(settings.language_servers().unwrap().is_disabled("rust"));
    }

    #[test]
    fn unknown_modes_and_languages_are_errors() {
        assert!(parse("lsp = \"on\"\n").is_err());
        let error = parse("lsp_servers = { rsut = \"rust-analyzer\" }\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("did you mean `rust`"), "{error}");
    }
}
