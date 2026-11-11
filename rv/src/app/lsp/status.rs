//! What the bar and the jump list say about the language server.

use std::path::Path;

use rv_core::lsp::ServerStatus;

use super::State;
use crate::app::App;
use crate::app::JumpSource;

impl App {
    /// Whether the loop should keep waking: an answer is outstanding, or the
    /// selected file's server is still starting or indexing.
    pub(in crate::app) fn lsp_busy(&self) -> bool {
        self.lsp_pending()
            || matches!(
                self.selected_server_status(),
                Some(ServerStatus::Starting | ServerStatus::Indexing { .. })
            )
    }

    fn selected_server_status(&self) -> Option<ServerStatus> {
        let path = &self.selected_file()?.path;
        self.navigation.hub()?.status_for(Path::new(path))
    }

    pub(in crate::app) fn selected_server_name(&self) -> String {
        let language = self
            .selected_file()
            .and_then(|file| rv_core::lsp::language_for_path(Path::new(&file.path)));
        self.navigation
            .hub()
            .and_then(|hub| {
                hub.servers()
                    .into_iter()
                    .find(|summary| Some(summary.language) == language.map(|l| l.name))
            })
            .map_or_else(|| "language server".to_owned(), |summary| summary.server)
    }

    /// The status bar's `nav:` segment; `None` while LSP is off.
    #[must_use]
    pub fn navigation_label(&self) -> Option<String> {
        if matches!(self.navigation.state, State::Off) {
            return None;
        }
        let name = || self.selected_server_name();
        Some(match self.selected_server_status() {
            None | Some(ServerStatus::Idle) => "nav: grammar".to_owned(),
            Some(ServerStatus::Starting) => format!("nav: {} starting", name()),
            Some(ServerStatus::Indexing {
                percentage: Some(percentage),
                ..
            }) => format!("nav: {} indexing {percentage}%", name()),
            Some(ServerStatus::Indexing { .. }) => format!("nav: {} indexing", name()),
            Some(ServerStatus::Ready) => format!("nav: {}", name()),
            Some(ServerStatus::Unavailable(_)) => "nav: unavailable".to_owned(),
        })
    }

    /// What the open jump list's title adds: the server that answered it, or
    /// why it is the grammar's answer only.
    #[must_use]
    pub fn jump_list_note(&self) -> Option<String> {
        if self.jump_list == crate::app::JumpList::Matches {
            return None;
        }
        match self.navigation.list_source {
            JumpSource::LanguageServer => Some(self.selected_server_name()),
            JumpSource::Grammar | JumpSource::Annotation => self.navigation.note.map(str::to_owned),
        }
    }
}
