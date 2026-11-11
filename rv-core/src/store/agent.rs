use super::Error;
use super::Store;

impl Store {
    pub fn agent_session(&self) -> Result<Option<String>, Error> {
        Ok(self.read_review()?.agent_session)
    }

    pub fn set_agent_session(&self, id: Option<&str>) -> Result<(), Error> {
        let mut review = self.read_review()?;
        review.agent_session = id.map(str::to_owned);
        self.write_review(&review)
    }
}
