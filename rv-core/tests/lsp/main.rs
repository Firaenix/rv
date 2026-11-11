#![cfg(unix)]

mod cache;
mod client;
mod fake_server;
mod framing;
mod positions;
mod servers;

/// Not a test: the entry point the fake language server runs through.
#[test]
fn fake_lsp_entry() {
    fake_server::run_if_requested();
}
