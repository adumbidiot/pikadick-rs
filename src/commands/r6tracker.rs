use crate::{
    ClientDataKey,
    client_data::{
        CacheStatsBuilder,
        CacheStatsProvider,
    },
    util::{
        TimedCache,
        TimedCacheEntry,
    },
};
use anyhow::Context as _;
use serenity::builder::{
    CreateEmbed,
    EditInteractionResponse,
};
use std::sync::Arc;
use tracing::{
    error,
    info,
};

/// R6Tracker stats for a user
#[derive(Debug)]
pub struct Stats {
    profile: r6tracker::UserData,
}

impl Stats {
    /// Populate an embed with data.
    pub fn populate_embed(&self, mut embed_builder: CreateEmbed) -> CreateEmbed {
        embed_builder = embed_builder
            .title(self.profile.platform_info.platform_user_handle.as_str())
            .image(self.profile.platform_info.avatar_url.as_str())
            .field(
                "Level",
                self.profile.metadata.clearance_level.to_string(),
                true,
            )
            .field(
                "Suspected Cheater",
                self.profile
                    .user_info
                    .is_suspicious
                    .unwrap_or(false)
                    .to_string(),
                true,
            );

        if let Some(season) = self.profile.get_current_ranked_season() {
            if let Some(name) = season.ranking_rank_name() {
                embed_builder = embed_builder.field("Current Rank", name, true);
            }
            if let Some(value) = season.ranking_value() {
                embed_builder = embed_builder.field("Current MMR", value.to_string(), true);
            }

            embed_builder = embed_builder
                .field(
                    "Seasonal Ranked K/D",
                    format!("{:.2}", season.kd_ratio_value()),
                    true,
                )
                .field(
                    "Seasonal Ranked Win %",
                    format!("{:.2}", season.win_percentage_value()),
                    true,
                )
                .field(
                    "Seasonal # of Ranked Matches",
                    season.matches_played_value().to_string(),
                    true,
                );
        }

        if let Some(season) = self.profile.get_current_ranked_season() {
            if let Some(name) = season.ranking_rank_name() {
                embed_builder = embed_builder.field("Current Casual Rank", name, true);
            }
            if let Some(value) = season.ranking_value() {
                embed_builder = embed_builder.field("Current Casual MMR", value.to_string(), true);
            }

            embed_builder = embed_builder
                .field(
                    "Seasonal Casual K/D",
                    format!("{:.2}", season.kd_ratio_value()),
                    true,
                )
                .field(
                    "Seasonal Casual Win %",
                    format!("{:.2}", season.win_percentage_value()),
                    true,
                )
                .field(
                    "Seasonal # of Casual Matches",
                    season.matches_played_value().to_string(),
                    true,
                );
        }

        let max_ranked_season = self.profile.get_max_ranked_season();
        if let Some(max_ranked_season) = max_ranked_season {
            let max_mmr = max_ranked_season.max_ranking_value();
            if let Some(max_mmr) = max_mmr {
                embed_builder = embed_builder.field("Best MMR", max_mmr.to_string(), true);
            }

            let max_rank = max_ranked_season.max_ranking_rank_name();
            if let Some(max_rank) = max_rank {
                embed_builder = embed_builder.field("Best Rank", max_rank, true);
            }
        }

        let ranked_game_mode = self.profile.get_ranked_game_mode();
        if let Some(game_mode) = ranked_game_mode {
            embed_builder = embed_builder
                .field(
                    "Lifetime Ranked K/D",
                    format!("{:.2}", game_mode.kd_ratio_value()),
                    true,
                )
                .field(
                    "Lifetime Ranked Win %",
                    format!("{:.2}", game_mode.win_percentage_value()),
                    true,
                );
        }

        let overview = self.profile.get_overview();
        if let Some(overview) = overview {
            embed_builder = embed_builder
                .field(
                    "Lifetime K/D",
                    format!("{:.2}", overview.kd_ratio_value()),
                    true,
                )
                .field(
                    "Lifetime Win %",
                    format!("{:.2}", overview.win_percentage_value()),
                    true,
                );
        }

        if let Some(c) = self
            .profile
            .get_current_ranked_season()
            .and_then(|season| season.color_u32())
        {
            embed_builder = embed_builder.color(c);
        }

        if let Some(thumb) = self.profile.current_mmr_image() {
            embed_builder = embed_builder.thumbnail(thumb.as_str());
        }

        embed_builder
    }
}

#[derive(Clone, Default, Debug)]
pub struct R6TrackerClient {
    client: r6tracker::Client,
    /// The value is `None` if the user could not be found
    search_cache: TimedCache<String, Option<Stats>>,
}

impl R6TrackerClient {
    /// Make a new r6 client with caching
    pub fn new() -> Self {
        R6TrackerClient {
            client: Default::default(),
            search_cache: Default::default(),
        }
    }

    /// Get R6Tracker stats
    pub async fn get_stats(
        &self,
        query: &str,
    ) -> anyhow::Result<Arc<TimedCacheEntry<Option<Stats>>>> {
        if let Some(entry) = self.search_cache.get_if_fresh(query) {
            return Ok(entry);
        }

        let profile_response = self
            .client
            .get_profile(query, r6tracker::Platform::Pc)
            .await
            .context("failed to get profile data")?;

        let profile = match profile_response.into_result() {
            Ok(profile) => Some(profile),
            Err(error) if error.is_not_found() => None,
            Err(error) => {
                return Err(r6tracker::Error::from(error).into());
            }
        };

        let entry = profile.map(|profile| Stats { profile });

        self.search_cache.insert(String::from(query), entry);

        self.search_cache
            .get_if_fresh(query)
            .context("cache data expired")
    }
}

impl CacheStatsProvider for R6TrackerClient {
    fn publish_cache_stats(&self, cache_stats_builder: &mut CacheStatsBuilder) {
        cache_stats_builder.publish_stat(
            "r6tracker",
            "search_cache",
            self.search_cache.len() as f32,
        );
    }
}

/// Options for r6tracker
#[derive(Debug, pikadick_slash_framework::FromOptions)]
struct R6TrackerOptions {
    /// The user name
    name: String,
}

/// Create a slash command
pub fn create_slash_command() -> anyhow::Result<pikadick_slash_framework::Command> {
    pikadick_slash_framework::CommandBuilder::new()
        .name("r6tracker")
        .description("Get r6 stats for a user from r6tracker")
        .argument(
            pikadick_slash_framework::ArgumentParamBuilder::new()
                .name("name")
                .description("The name of the user")
                .kind(pikadick_slash_framework::ArgumentKind::String)
                .required(true)
                .build()?,
        )
        .on_process(|ctx, interaction, args: R6TrackerOptions| async move {
            let data_lock = ctx.data.read().await;
            let client_data = data_lock
                .get::<ClientDataKey>()
                .expect("missing client data");
            let client = client_data.r6tracker_client.clone();
            drop(data_lock);

            let name = args.name;

            info!("Getting r6 stats for \"{name}\" using R6Tracker");

            interaction.defer(&ctx.http).await?;

            let result = client
                .get_stats(&name)
                .await
                .with_context(|| format!("failed to get r6tracker stats for \"{name}\""));

            let mut edit_response_builder = EditInteractionResponse::new();
            match result.as_ref().map(|entry| entry.data()) {
                Ok(Some(stats)) => {
                    let embed_builder = stats.populate_embed(CreateEmbed::new());
                    edit_response_builder = edit_response_builder.embed(embed_builder);
                }
                Ok(None) => {
                    edit_response_builder = edit_response_builder.content("No Results");
                }
                Err(error) => {
                    error!("{error:?}");
                    edit_response_builder = edit_response_builder.content(format!("{error:?}"));
                }
            }

            interaction
                .edit_response(&ctx.http, edit_response_builder)
                .await?;

            client.search_cache.trim();

            Ok(())
        })
        .build()
        .context("failed to build r6tracker command")
}
