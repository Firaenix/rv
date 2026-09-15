//! `rv flag` / `flags` / `ack` / `unflag`, and `rv review` / `unreview` /
//! `reviewed`: the attention channel beside the comment one.

use anyhow::Context as _;
use anyhow::Result;
use rv::session;
use rv::session::Review;
use rv::session::reviewed::Freshness;
use rv_core::model::Side;
use rv_core::store::Flag;
use serde_json::json;

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
    }
}

pub fn flag(review: &Review, file: &str, side: Side, line: u32, reason: &str) -> Result<()> {
    let flag = session::flags::add_flag(review, file, side, line, reason)?;
    println!(
        "flagged {} at {}:{} ({})",
        flag.id,
        flag.anchor.file,
        flag.anchor.line,
        side_name(flag.anchor.side)
    );
    Ok(())
}

pub fn flags(review: &Review, json: bool, open_only: bool) -> Result<()> {
    let flags = session::flags::in_range(
        review,
        review
            .store
            .flags()
            .context("could not read the review's flags")?,
    );
    let flags: Vec<&Flag> = flags
        .iter()
        .filter(|flag| !open_only || !flag.acknowledged)
        .collect();

    if json {
        let listed: Vec<_> = flags
            .iter()
            .map(|flag| {
                json!({
                    "id": flag.id,
                    "change_id": flag.change_id,
                    "commit_id": flag.commit_id,
                    "file": flag.anchor.file,
                    "side": side_name(flag.anchor.side),
                    "line": flag.anchor.line,
                    "reason": flag.reason,
                    "acknowledged": flag.acknowledged,
                })
            })
            .collect();
        let serialized =
            serde_json::to_string_pretty(&listed).context("could not serialize the flags")?;
        println!("{serialized}");
        return Ok(());
    }

    for flag in flags {
        let state = if flag.acknowledged {
            "acknowledged"
        } else {
            "open"
        };
        let first = flag.reason.lines().next().unwrap_or_default();
        println!(
            "{} {state:<12} {}:{} {first}",
            flag.id, flag.anchor.file, flag.anchor.line
        );
    }
    Ok(())
}

pub fn ack(review: &Review, id: &str) -> Result<()> {
    session::flags::acknowledge(review, id)?;
    println!("acknowledged {id}");
    Ok(())
}

pub fn unflag(review: &Review, id: &str) -> Result<()> {
    if !review
        .store
        .remove_flag(id)
        .with_context(|| format!("could not remove flag {id}"))?
    {
        anyhow::bail!("no flag {id} in this review — ids are in `rv flags`");
    }
    println!("removed {id}");
    Ok(())
}

pub fn mark_reviewed(review: &Review, file: &str, change: Option<&str>) -> Result<()> {
    let reviewed = session::reviewed::mark(review, file, change)?;
    println!("reviewed {}{}", reviewed.file, scope_suffix(change));
    Ok(())
}

pub fn unmark_reviewed(review: &Review, file: &str, change: Option<&str>) -> Result<()> {
    if !session::reviewed::unmark(review, file, change)? {
        anyhow::bail!("{file}{} was not marked reviewed", scope_suffix(change));
    }
    println!("unreviewed {file}{}", scope_suffix(change));
    Ok(())
}

fn scope_suffix(change: Option<&str>) -> String {
    change.map_or_else(String::new, |change| format!(" in {change}"))
}

pub fn reviewed(review: &Review, json: bool) -> Result<()> {
    let ticks = review
        .store
        .reviewed()
        .context("could not read the reviewed files")?;
    let fresh: Vec<(_, Freshness)> = ticks
        .iter()
        .map(|tick| (tick, session::reviewed::freshness(review, tick)))
        .collect();

    if json {
        let listed: Vec<_> = fresh
            .iter()
            .map(|(tick, freshness)| {
                json!({
                    "file": tick.file,
                    "change_id": tick.change_id,
                    "changed": *freshness == Freshness::Changed,
                })
            })
            .collect();
        let serialized = serde_json::to_string_pretty(&listed)
            .context("could not serialize the reviewed files")?;
        println!("{serialized}");
        return Ok(());
    }

    for (tick, freshness) in fresh {
        let note = match freshness {
            Freshness::Current => "",
            Freshness::Changed => "  (changed since)",
        };
        println!(
            "{:<8} {}{note}",
            tick.change_id.as_deref().unwrap_or("range"),
            tick.file
        );
    }
    Ok(())
}
