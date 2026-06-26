use super::{
    JiffTimestampWrapper,
    Rule34Post,
    Rule34QueryStat,
};
use crate::Database;
use nd_async_rusqlite::rusqlite::{
    OptionalExtension,
    named_params,
};
use std::collections::HashSet;

const GET_RANDOM_RULE34_POST_SQL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/sql/get_random_rule34_post.sql"
));

const UPSERT_RULE34_POST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/sql/upsert_rule34_post.sql"
));
const UPSERT_RULE34_TAG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/sql/upsert_rule34_tag.sql"
));
const INSERT_RULE34_POST_TAG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/sql/insert_rule34_post_tag.sql"
));

const CLEAR_RULE34_POST_TAGS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/sql/clear_rule34_post_tags.sql"
));

const GET_RULE34_QUERY_STAT: &str = "
SELECT
    query,
    queries_since_last_fetch,
    last_fetched
FROM
    rule34_query_stat
WHERE
    query = :query;
";

const UPSERT_RULE34_QUERY_STAT: &str = "
INSERT INTO rule34_query_stat (
    query,
    queries_since_last_fetch,
    last_fetched
) VALUES (
    :query,
    :queries_since_last_fetch,
    :last_fetched
) ON CONFLICT (query) DO UPDATE SET
    query = :query,
    last_fetched = :last_fetched;
";

impl Database {
    pub async fn upsert_rule34_posts_and_query(
        &self,
        posts: Vec<Rule34Post>,
        query: Option<String>,
        last_fetched: jiff::Timestamp,
    ) -> anyhow::Result<()> {
        if posts.is_empty() {
            return Ok(());
        }

        self.database
            .write(move |database| {
                let transaction = database.transaction()?;

                let mut post_tag_ids = Vec::new();

                {
                    let mut query = transaction.prepare_cached(UPSERT_RULE34_TAG)?;
                    for post in posts.iter() {
                        let tags: HashSet<_> =
                            post.tags.split(' ').filter(|tag| !tag.is_empty()).collect();
                        let mut tag_ids = HashSet::with_capacity(tags.len());
                        for tag in tags {
                            let id: u64 = query.query_one(
                                named_params! {
                                    ":name": tag,
                                },
                                |row| row.get("id"),
                            )?;
                            tag_ids.insert(id);
                        }
                        post_tag_ids.push(tag_ids);
                    }
                }

                {
                    let mut query = transaction.prepare_cached(UPSERT_RULE34_POST)?;
                    for post in posts.iter() {
                        query.execute(named_params! {
                            ":id": post.id,
                            ":tags": post.tags,
                            ":file_url": post.file_url,
                            ":last_fetched": post.last_fetched.as_millisecond(),
                        })?;
                    }
                }

                {
                    let mut query = transaction.prepare_cached(CLEAR_RULE34_POST_TAGS)?;
                    for post in posts.iter() {
                        query.execute(named_params! {
                            ":id": post.id,
                        })?;
                    }
                }

                {
                    let mut query = transaction.prepare_cached(INSERT_RULE34_POST_TAG)?;
                    for (post, tag_ids) in posts.iter().zip(post_tag_ids.iter()) {
                        for tag_id in tag_ids.iter() {
                            query.execute(named_params! {
                                ":post_id": post.id,
                                ":tag_id": tag_id,
                            })?;
                        }
                    }
                }

                {
                    transaction
                        .prepare_cached(UPSERT_RULE34_QUERY_STAT)?
                        .execute(named_params! {
                            ":query": query.as_deref().unwrap_or(""),
                            ":queries_since_last_fetch": 0,
                            ":last_fetched": JiffTimestampWrapper(last_fetched),
                        })?;
                }

                transaction.commit()?;
                anyhow::Ok(())
            })
            .await?
    }

    /// Get a random rule34 file url by tag, along with the query stats.
    pub async fn get_random_rule34_post_file_url_and_query_stat(
        &self,
        query: Option<String>,
        random_seed: i64,
        limit: Option<usize>,
    ) -> anyhow::Result<(Vec<String>, Option<Rule34QueryStat>)> {
        self.database
            .read(move |database| {
                let transaction = database.transaction()?;
                let results = transaction
                    .prepare_cached(GET_RANDOM_RULE34_POST_SQL)?
                    .query_map(
                        named_params! {
                            ":random_seed": random_seed,
                            ":tag_name": query,
                        },
                        |row| {
                            let file_url: String = row.get("file_url")?;
                            Ok(file_url)
                        },
                    )?
                    .take(limit.unwrap_or(10))
                    .collect::<Result<Vec<String>, _>>()?;

                let query_stat = transaction
                    .prepare_cached(GET_RULE34_QUERY_STAT)?
                    .query_one(
                        named_params! {
                            ":query": query.as_deref().unwrap_or(""),
                        },
                        Rule34QueryStat::from_row,
                    )
                    .optional()?;

                anyhow::Ok((results, query_stat))
            })
            .await?
    }
}
