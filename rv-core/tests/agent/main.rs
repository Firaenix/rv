#![cfg(unix)]

mod ask;
mod contract;
mod prompts;
mod session;
mod stub;

/// Not a test: the entry point the stub agent runs through.
#[test]
fn stub_agent_entry() {
    stub::run_if_requested();
}
