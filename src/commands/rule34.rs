use crate::{
    PoiseContext,
    PoiseError,
};
use anyhow::Context as _;
use bewu_util::AsyncTimedLruCache;
use nd_util::ArcAnyhowError;
use rand::prelude::IndexedRandom;
use std::{
    sync::Arc,
    time::Duration,
};
use tracing::{
    error,
    info,
    warn,
};

const FIVE_MINUTES: Duration = Duration::from_secs(60 * 5);

/// A caching rule34 client
#[derive(Debug)]
pub struct Rule34Client {
    client: rule34::Client,
    cache: AsyncTimedLruCache<Option<String>, Result<Arc<rule34::PostList>, ArcAnyhowError>>,
}

impl Rule34Client {
    /// Make a new [`Rule34Client`].
    pub fn new(user_id: u64, api_key: &str) -> Rule34Client {
        let client = rule34::Client::new();
        client.set_auth(user_id, api_key);

        let cache = AsyncTimedLruCache::new(100, FIVE_MINUTES);

        Rule34Client { client, cache }
    }

    /// Search for a query.
    #[tracing::instrument(skip(self))]
    pub async fn list(
        &self,
        query: Option<String>,
    ) -> Result<Arc<rule34::PostList>, ArcAnyhowError> {
        self.cache
            .get(query.clone(), || async move {
                self.client
                    .list_posts()
                    .tags(query.as_deref())
                    .limit(Some(1_000))
                    .execute()
                    .await
                    .context("Failed to search rule34")
                    .map(Arc::new)
                    .map_err(ArcAnyhowError::new)
            })
            .await
    }

    /// Autocomplete a tag.
    pub async fn autocomplete(&self, query: &str) -> anyhow::Result<Vec<String>> {
        let results = self.client.autocomplete(query).await?;
        Ok(results.into_iter().map(|result| result.value).collect())
    }
}

async fn autocomplete_query(ctx: PoiseContext<'_>, partial: &str) -> Vec<String> {
    // TODO: Use a full rule34 tag parser here.
    let (head, partial_tag) = partial
        .rsplit_once(' ')
        .map(|(head, tail)| (Some(head), tail))
        .unwrap_or((None, partial));
    let tags = ctx
        .data()
        .rule34_client
        .autocomplete(partial_tag)
        .await
        .context("Failed to autocomplete");
    let tags = match tags {
        Ok(tags) => tags,
        Err(error) => {
            warn!("{error:?}");
            return Vec::new();
        }
    };

    let mut queries = Vec::with_capacity(std::cmp::min(tags.len(), 25));
    for tag in tags.iter() {
        let mut query = String::new();
        if let Some(head) = head.as_ref() {
            query.push_str(head);
            query.push(' ');
        }
        query.push_str(tag);

        queries.push(query);
    }

    queries
}

#[poise::command(
    slash_command,
    description_localized("en-US", "Look up rule34 images from rule34.xxx"),
    check = "crate::checks::enabled"
)]
pub async fn rule34(
    ctx: PoiseContext<'_>,
    #[description = "The rule34.xxx search query. Supports the same syntax as the website."]
    #[autocomplete = "autocomplete_query"]
    query: Option<String>,
) -> Result<(), PoiseError> {
    let query_format = query
        .as_deref()
        .map(|query| format!("{query:?}"))
        .unwrap_or_else(|| String::from("None"));
    info!("searching rule34 for {query_format}");
    let result = ctx
        .data()
        .rule34_client
        .list(query.clone())
        .await
        .context("Failed to get search results");

    let content = match result {
        Ok(list_results) => {
            let maybe_list_result: Option<String> = list_results
                .posts
                .choose(&mut rand::rng())
                .map(|list_result| list_result.file_url.to_string());

            if let Some(file_url) = maybe_list_result {
                file_url
            } else {
                format!("No results for {query_format}.")
            }
        }
        Err(error) => {
            error!("{error:?}");
            format!("{error:?}")
        }
    };
    ctx.reply(content).await?;

    Ok(())
}
