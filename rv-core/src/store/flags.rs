//! The flag half of `session.toml`: attention pointers, kept apart from
//! comments (flags spec §2).
//!
//! A flag says *look here, and why*. It has no reply, no settling actor and
//! exactly one transition — unacknowledged to acknowledged — so it never gates
//! `rv status --check`. Same file, same atomic rewrite as the comments.

use serde::Deserialize;
use serde::Serialize;

use super::Error;
use super::Store;
use crate::model::Anchor;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flag {
    pub id: String,
    pub change_id: String,
    pub commit_id: String,
    pub anchor: Anchor,
    pub reason: String,
    #[serde(default)]
    pub acknowledged: bool,
}

impl Store {
    pub fn flags(&self) -> Result<Vec<Flag>, Error> {
        Ok(self.read_review()?.flags)
    }

    /// Upserts `flag` by id, keeping an existing entry's position.
    pub fn append_flag(&self, flag: &Flag) -> Result<(), Error> {
        let mut review = self.read_review()?;
        match review
            .flags
            .iter_mut()
            .find(|existing| existing.id == flag.id)
        {
            Some(existing) => *existing = flag.clone(),
            None => review.flags.push(flag.clone()),
        }
        self.write_review(&review)
    }

    /// Sets whether the flag with `id` has been seen, returning whether one
    /// was there. An unknown id is not an error: acknowledging twice is safe.
    pub fn acknowledge_flag(&self, id: &str, acknowledged: bool) -> Result<bool, Error> {
        let mut review = self.read_review()?;
        let Some(flag) = review.flags.iter_mut().find(|existing| existing.id == id) else {
            return Ok(false);
        };
        flag.acknowledged = acknowledged;
        self.write_review(&review)?;
        Ok(true)
    }

    /// Removes the flag with `id`, returning whether one was there.
    pub fn remove_flag(&self, id: &str) -> Result<bool, Error> {
        let mut review = self.read_review()?;
        let before = review.flags.len();
        review.flags.retain(|existing| existing.id != id);
        if review.flags.len() == before {
            return Ok(false);
        }
        self.write_review(&review)?;
        Ok(true)
    }
}
