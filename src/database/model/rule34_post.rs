/*
use nd_async_rusqlite::rusqlite::{
    Result as RusqliteResult,
    Row as RusqliteRow,
    types::FromSqlError,
};
*/
use std::num::NonZeroU64;

///A rule34 post.
#[derive(Debug)]
pub struct Rule34Post {
    pub id: NonZeroU64,
    pub tags: String,
    pub file_url: String,
    pub last_fetched: jiff::Timestamp,
}
/*
impl Rule34Post {
    pub(crate) fn from_row(row: RusqliteRow<'_>) -> RusqliteResult<Self> {
        let id = row.get("id")?;
        let tags = row.get("tags")?;
        let last_fetched: i64 = row.get("last_fetched")?;
        let last_fetched = jiff::Timestamp::from_millisecond(last_fetched)
            .map_err(|error| FromSqlError::Other(error.into()))?;

        Ok(Self {
            id,
            tags,
            last_fetched,
        })
    }
}
*/
