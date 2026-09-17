//! The subcommands, split from `main` for the 400-line rule.

use std::path::PathBuf;

use clap::Subcommand;
use rv_core::store::CommentState;

use crate::commands::ByArg;
use crate::commands::SideArg;

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Add a comment to the review, exactly as the TUI would.
    ///
    /// For reviewer agents: the anchor, the id and the store are all handled,
    /// so nothing writes `.review/` files by hand.
    Comment {
        /// The file, as `rv status` lists it.
        file: String,
        /// The 1-based line the comment is about.
        #[arg(long)]
        line: u32,
        /// Which side of the diff the line is on: `right` is the code as it
        /// will exist (the default), `left` a removed line's base side.
        #[arg(long, default_value = "right")]
        side: SideArg,
        /// The comment itself; `-` reads it from stdin, so a body full of
        /// quotes and backticks never meets the shell.
        #[arg(short, long)]
        message: String,
    },
    /// List the review's comments — the agent's read channel.
    Comments {
        /// Emit JSON instead of text. The JSON is the contract.
        #[arg(long)]
        json: bool,
        /// Only comments in this state, e.g. `--state open` for "what is
        /// waiting on me".
        #[arg(long)]
        state: Option<StateArg>,
    },
    /// Store a reply on a comment — the agent's answer channel.
    ///
    /// A second reply replaces the first. Replying changes no state: resolving
    /// stays its own deliberate act.
    Reply {
        /// The comment's id, from `rv comments`.
        id: String,
        /// The reply; `-` reads it from stdin.
        #[arg(short, long)]
        message: String,
    },
    /// Mark a comment resolved: it was addressed.
    ///
    /// Records who settled it. The default is `agent`, because this command is
    /// the agent's path — a human resolves in the TUI with `R`, which records
    /// `user`. Either state re-applied is the undo: resolving a resolved
    /// comment reopens it.
    Resolve {
        /// The comment's id, from `rv comments`.
        id: String,
        /// Who is settling it.
        #[arg(long, default_value = "agent")]
        by: ByArg,
    },
    /// Mark a comment abandoned: dropped without being addressed.
    ///
    /// A separate state from resolved on purpose — *fixed* and *dropped unfixed*
    /// are different conclusions, and a count that adds them together misreports
    /// what the review decided.
    Abandon {
        /// The comment's id.
        id: String,
        /// Who is settling it.
        #[arg(long, default_value = "agent")]
        by: ByArg,
    },
    /// Print the range's diffs in rv's own side-aware coordinates.
    ///
    /// The numbers printed here are the numbers `rv comment --line` accepts:
    /// `right` is the head file's, `left` the base file's.
    Diff {
        /// One file, as `rv status` lists it [default: every file].
        file: Option<String>,
        /// Emit JSON instead of rows. The JSON is the contract.
        #[arg(long)]
        json: bool,
    },
    /// Print the review as markdown — a view, which nothing reads back.
    Render {
        /// Write to this file instead of stdout.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Open the session defaults (`~/.config/rv/Config.toml`) in $EDITOR.
    ///
    /// A missing file is seeded with the fully-commented defaults first, and
    /// the result is validated the moment the editor exits.
    Config,
    /// Open the keybindings (`~/.config/rv/keybindings.toml`) in $EDITOR —
    /// seeded and validated the same way — or, with --show, print the
    /// effective keymap: the defaults plus the file's patch, as the same TOML
    /// the file speaks.
    Keymap {
        /// Print the effective keymap instead of opening the editor.
        #[arg(long)]
        show: bool,
    },
    /// List every review stored under this repo — one per reviewed head —
    /// with its range and comment counts.
    Reviews,
    /// Report the range, its changes, its files and its comment counts.
    Status {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Exit 1 while any comment is open — the worker's poll and a CI
        /// gate in one flag. Prints nothing unless `--json` asks it to.
        #[arg(long)]
        check: bool,
    },
    /// Point the reviewer at a line with a reason — attention, not feedback.
    ///
    /// A flag never blocks `rv status --check`: it asks for a look, not a
    /// change. The agent's way to say "start here".
    Flag {
        /// The file, as `rv status` lists it.
        file: String,
        /// The 1-based line the flag is about.
        #[arg(long)]
        line: u32,
        /// Which side of the diff the line is on.
        #[arg(long, default_value = "right")]
        side: SideArg,
        /// Why this line deserves a look; `-` reads it from stdin.
        #[arg(short, long)]
        message: String,
    },
    /// List the review's flags.
    Flags {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Only flags nobody has acknowledged yet.
        #[arg(long)]
        open: bool,
    },
    /// Mark a flag acknowledged: it has been looked at.
    Ack {
        /// The flag's id, from `rv flags`.
        id: String,
    },
    /// Remove a flag.
    Unflag {
        /// The flag's id, from `rv flags`.
        id: String,
    },
    /// Tick a file off as reviewed — for the whole range, or for one change's
    /// diff of it with `--change`.
    Review {
        /// The file, as `rv status` lists it.
        file: String,
        /// Tick the file's diff under this change rather than the range.
        #[arg(long, value_name = "CHANGE")]
        change: Option<String>,
    },
    /// Clear a file's reviewed tick.
    Unreview {
        /// The file, as `rv status` lists it.
        file: String,
        /// The change whose tick to clear, if not the range's.
        #[arg(long, value_name = "CHANGE")]
        change: Option<String>,
    },
    /// List the files ticked off as reviewed, and whether each has changed
    /// since.
    Reviewed {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

/// `--state` as clap sees it.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum StateArg {
    Open,
    AwaitingVerification,
    Resolved,
    Abandoned,
    Outdated,
}

impl From<StateArg> for CommentState {
    fn from(state: StateArg) -> Self {
        match state {
            StateArg::Open => CommentState::Open,
            StateArg::AwaitingVerification => CommentState::AwaitingVerification,
            StateArg::Resolved => CommentState::Resolved,
            StateArg::Abandoned => CommentState::Abandoned,
            StateArg::Outdated => CommentState::Outdated,
        }
    }
}
