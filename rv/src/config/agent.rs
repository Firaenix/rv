//! `[agent]` in `Config.toml`: the agent CLI rv may run. Absent means off.

use anyhow::Result;
use anyhow::bail;
use rv_core::agent::AgentConfig;
use rv_core::agent::Orientation;
use serde::Deserialize;

#[derive(Deserialize, Debug, Default, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSettings {
    /// The program and its arguments; an empty list turns the agent off.
    pub command: Option<Vec<String>>,
    pub model: Option<String>,
    pub model_flag: Option<String>,
    pub allowed_tools: Option<String>,
    pub allowed_tools_flag: Option<String>,
    pub resume_flag: Option<String>,
    pub timeout_secs: Option<u64>,
    pub orientation: Option<OrientationName>,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum OrientationName {
    Off,
    Files,
    OnDemand,
}

impl From<OrientationName> for Orientation {
    fn from(name: OrientationName) -> Self {
        match name {
            OrientationName::Off => Orientation::Off,
            OrientationName::Files => Orientation::Files,
            OrientationName::OnDemand => Orientation::OnDemand,
        }
    }
}

impl AgentSettings {
    pub(super) fn validate(&self) -> Result<()> {
        if self.command.is_none() {
            bail!(
                "[agent] needs a command, e.g. command = [\"maki\", \"-p\", \"--output-format\", \"json\"]"
            );
        }
        if self.timeout_secs == Some(0) {
            bail!("[agent] timeout_secs must be at least 1");
        }
        for (name, flag) in [
            ("model_flag", &self.model_flag),
            ("allowed_tools_flag", &self.allowed_tools_flag),
            ("resume_flag", &self.resume_flag),
        ] {
            if flag.as_deref().is_some_and(|flag| flag.trim().is_empty()) {
                bail!("[agent] {name} must not be empty");
            }
        }
        Ok(())
    }

    /// The core's config, or `None` when the agent is off. A configured
    /// command with no `orientation` asks only when the reviewer does.
    #[must_use]
    pub fn to_config(&self) -> Option<AgentConfig> {
        let command = self.command.clone().filter(|command| !command.is_empty())?;
        let mut config = AgentConfig::new(command);
        config.model = self.model.clone();
        config.allowed_tools = self.allowed_tools.clone();
        let flags = [
            (&mut config.model_flag, &self.model_flag),
            (&mut config.allowed_tools_flag, &self.allowed_tools_flag),
            (&mut config.resume_flag, &self.resume_flag),
        ];
        for (slot, chosen) in flags {
            if let Some(chosen) = chosen {
                slot.clone_from(chosen);
            }
        }
        if let Some(timeout) = self.timeout_secs {
            config.timeout_secs = timeout;
        }
        if let Some(orientation) = self.orientation {
            config.orientation = orientation.into();
        }
        Some(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    fn parsed(text: &str) -> Settings {
        toml::from_str(text).expect("parses")
    }

    #[test]
    fn no_section_means_no_agent() {
        assert!(parsed("").agent_config().is_none());
        assert!(parsed("[agent]\ncommand = []\n").agent_config().is_none());
    }

    #[test]
    fn a_configured_agent_patches_the_defaults() {
        let config = parsed(
            "[agent]\ncommand = [\"claude\", \"--print\"]\nmodel = \"opus\"\n\
             allowed_tools = \"Read\"\nresume_flag = \"--resume\"\ntimeout_secs = 30\n\
             orientation = \"files\"\n",
        )
        .agent_config()
        .expect("configured");
        assert_eq!(config.command, ["claude", "--print"]);
        assert_eq!(config.model.as_deref(), Some("opus"));
        assert_eq!(config.model_flag, "--model");
        assert_eq!(config.allowed_tools_flag, "--allowedTools");
        assert_eq!(config.resume_flag, "--resume");
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.orientation, Orientation::Files);
    }

    #[test]
    fn a_command_without_orientation_asks_only_on_demand() {
        let config = parsed("[agent]\ncommand = [\"maki\", \"-p\"]\n").agent_config();
        assert_eq!(
            config.map(|config| config.orientation),
            Some(Orientation::OnDemand)
        );
    }

    #[test]
    fn typos_and_missing_commands_are_errors() {
        assert!(toml::from_str::<Settings>("[agent]\ncomand = [\"maki\"]\n").is_err());
        assert!(
            toml::from_str::<Settings>("[agent]\ncommand = [\"m\"]\norientation = \"always\"\n")
                .is_err()
        );
        assert!(parsed("[agent]\nmodel = \"opus\"\n").validate().is_err());
        assert!(
            parsed("[agent]\ncommand = [\"m\"]\ntimeout_secs = 0\n")
                .validate()
                .is_err()
        );
    }
}
