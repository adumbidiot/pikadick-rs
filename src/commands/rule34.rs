use crate::{
    Database,
    PoiseContext,
    PoiseError,
    database::Rule34Post,
};
use anyhow::Context as _;
use bewu_util::{
    AsyncMutexMap,
    AsyncTimedLruCache,
};
use jiff::SignedDuration;
use nd_util::ArcAnyhowError;
use rand::{
    RngExt,
    prelude::IndexedRandom,
};
use std::{
    sync::Arc,
    time::Duration,
};
use tracing::{
    debug,
    info,
    warn,
};

const FIVE_MINUTES: Duration = Duration::from_secs(60 * 5);
const ONE_DAY: SignedDuration = SignedDuration::from_hours(24);

fn post_list_to_database_model(post_list: &rule34::PostList) -> Vec<Rule34Post> {
    let last_fetched = jiff::Timestamp::now();
    post_list
        .posts
        .iter()
        .map(|post| Rule34Post {
            id: post.id,
            tags: post.tags.to_string(),
            file_url: post.file_url.to_string(),
            last_fetched,
        })
        .collect()
}

/// A caching rule34 client
#[derive(Debug)]
pub struct Rule34Client {
    client: rule34::Client,
    database: Database,
    search_cache: AsyncTimedLruCache<Option<String>, Result<Arc<rule34::PostList>, ArcAnyhowError>>,
    search_map: AsyncMutexMap<Option<String>>,
}

impl Rule34Client {
    /// Make a new [`Rule34Client`].
    pub fn new(user_id: u64, api_key: &str, database: Database) -> Rule34Client {
        let client = rule34::Client::new();
        client.set_auth(user_id, api_key);

        let search_cache = AsyncTimedLruCache::new(100, FIVE_MINUTES);
        let search_map = AsyncMutexMap::new();

        Rule34Client {
            client,
            database,
            search_cache,
            search_map,
        }
    }

    /// Search for a query.
    #[tracing::instrument(skip(self))]
    async fn list(&self, query: Option<String>) -> Result<Arc<rule34::PostList>, ArcAnyhowError> {
        self.search_cache
            .get(query.clone(), || async move {
                self.client
                    .list_posts()
                    .tags(query.as_deref())
                    .limit(Some(rule34::POST_LIST_LIMIT_MAX))
                    .execute()
                    .await
                    .context("Failed to search rule34")
                    .map(Arc::new)
                    .map_err(ArcAnyhowError::new)
            })
            .await
    }

    async fn list_posts_and_get_file_url(&self, query: Option<String>) -> anyhow::Result<String> {
        let now = jiff::Timestamp::now();
        let post_list = self.list(query.clone()).await?;
        let posts = post_list_to_database_model(&post_list);
        self.database
            .upsert_rule34_posts_and_query(posts, query.clone(), now)
            .await
            .context("Failed to upsert post")?;
        post_list
            .posts
            .choose(&mut rand::rng())
            .map(|list_result| list_result.file_url.to_string())
            .with_context(|| format!("No results for {query:?}"))
    }

    async fn get_random_post_file_url_simple(
        &self,
        query: Option<String>,
    ) -> anyhow::Result<String> {
        let random_seed = rand::rng().random::<i64>();
        let limit = 10;
        let (mut file_urls, last_fetched) = self
            .database
            .get_random_rule34_post_file_url_and_last_fetched_time(
                query.clone(),
                random_seed,
                Some(limit),
            )
            .await?;

        if last_fetched.is_none_or(|value| value.duration_until(jiff::Timestamp::now()) > ONE_DAY)
            || self.client.ratelimit_budget_remaining() < rule34::RATELIMIT_BUDGET / 2
            || file_urls.is_empty()
        {
            debug!("Fetching new data");
            return self.list_posts_and_get_file_url(query).await;
        }

        // TODO: Handle deleted posts.
        let file_url = file_urls
            .pop()
            .with_context(|| format!("No results for {query:?}"))?;

        debug!("Using cached data");

        Ok(file_url)
    }

    /// Get a random post file url.
    async fn get_random_post_file_url(&self, query: Option<String>) -> anyhow::Result<String> {
        let _lock = self.search_map.lock(query.clone()).await;

        // TODO: Expand this check to include AND queries.
        // TODO: Consider using query parser.
        if query
            .as_deref()
            .is_none_or(|value| !value.chars().any(|ch| ch == ' '))
        {
            return self.get_random_post_file_url_simple(query).await;
        }

        self.list_posts_and_get_file_url(query).await
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
    ctx.defer().await?;

    let query_format = query
        .as_deref()
        .map(|query| format!("{query:?}"))
        .unwrap_or_else(|| String::from("None"));
    info!("Searching rule34 for {query_format}");
    let result = ctx
        .data()
        .rule34_client
        .get_random_post_file_url(query.clone())
        .await
        .context("Failed to get search results");

    let content = match result {
        Ok(file_url) => file_url,
        Err(error) => {
            warn!("{error:?}");
            format!("{error:?}")
        }
    };
    ctx.reply(content).await?;

    Ok(())
}
