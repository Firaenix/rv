//! `rv brief`: the handoff bundle — every unsettled note with the code around
//! it as it stands now, and the commands that answer it.

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use rv::session::Review;
use rv::session::brief::assemble;
use rv_core::markdown::brief;

pub fn brief(review: &Review, json: bool, out: Option<&Path>, context: u32) -> Result<()> {
    let brief = assemble(review, context)?;
    let document = if json {
        let mut serialized =
            serde_json::to_string_pretty(&brief).context("could not serialize the brief")?;
        serialized.push('\n');
        serialized
    } else {
        brief::render(&brief)
    };
    match out {
        None => print!("{document}"),
        Some(path) => {
            std::fs::write(path, document)
                .with_context(|| format!("could not write {}", path.display()))?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}
