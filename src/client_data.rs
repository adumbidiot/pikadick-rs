use crate::{
    checks::EnabledCheckData,
    commands::{
        deviantart::DeviantartClient,
        iqdb::IqdbClient,
        nekos::NekosClient,
        quizizz::QuizizzClient,
        r6tracker::R6TrackerClient,
        reddit_embed::RedditEmbedData,
        rule34::Rule34Client,
        sauce_nao::SauceNaoClient,
        shift::ShiftClient,
        tic_tac_toe::TicTacToeData,
        tiktok_embed::TikTokData,
        urban::UrbanClient,
    },
    config::Config,
    database::Database,
    util::EncoderTask,
};
use anyhow::Context;
use serenity::gateway::ShardManager;
use std::{
    fmt::Debug,
    sync::Arc,
};
use tracing::error;

/// The [`ClientData`].
#[derive(Debug)]
pub struct ClientData {
    /// The discord shard_manager
    pub shard_manager: Arc<ShardManager>,

    /// The client for nekos
    pub nekos_client: NekosClient,
    /// The r6tracker client
    pub r6tracker_client: R6TrackerClient,
    /// The rule34 client
    pub rule34_client: Rule34Client,
    /// The quizizz client
    pub quizizz_client: QuizizzClient,
    /// The shift client
    pub shift_client: ShiftClient,
    /// The reddit embed data
    pub reddit_embed_data: RedditEmbedData,
    /// The enabled check data
    pub enabled_check_data: EnabledCheckData,
    /// The insta client data
    pub insta_client: insta::Client,
    /// The deviantart client
    pub deviantart_client: DeviantartClient,
    /// The urban dictionary client
    pub urban_client: UrbanClient,
    /// The xkcd client
    pub xkcd_client: xkcd::Client,
    /// The tic tac toe data
    pub tic_tac_toe_data: TicTacToeData,
    /// The iqdb client
    pub iqdb_client: IqdbClient,
    /// The sauce nao client
    pub sauce_nao_client: SauceNaoClient,
    /// The yodaspeak client
    pub yodaspeak: yodaspeak::Client,
    /// TikTokData
    pub tiktok_data: TikTokData,
    /// Encoder Task
    pub encoder_task: EncoderTask,

    /// The database
    pub db: Database,

    /// The config
    pub config: Arc<Config>,
}

impl ClientData {
    /// Init this client data
    pub async fn init(
        shard_manager: Arc<ShardManager>,
        config: Arc<Config>,
        db: Database,
    ) -> anyhow::Result<Self> {
        // TODO: Standardize an async init system with allocated data per command somehow. Maybe boxes?

        let cache_dir = config.cache_dir();
        let encoder_task = EncoderTask::new();

        let deviantart_client = DeviantartClient::new(&db)
            .await
            .context("failed to init deviantart client")?;
        let tiktok_data = TikTokData::new(&cache_dir, encoder_task.clone())
            .await
            .context("failed to init tiktok data")?;
        let rule34_client = Rule34Client::new(config.rule34.user_id, &config.rule34.api_key);

        Ok(ClientData {
            shard_manager,

            nekos_client: Default::default(),
            r6tracker_client: Default::default(),
            rule34_client,

            quizizz_client: Default::default(),
            shift_client: ShiftClient::new(),
            reddit_embed_data: Default::default(),
            enabled_check_data: Default::default(),
            insta_client: insta::Client::new(),
            deviantart_client,
            urban_client: Default::default(),
            xkcd_client: Default::default(),
            tic_tac_toe_data: Default::default(),
            iqdb_client: Default::default(),
            sauce_nao_client: SauceNaoClient::new(config.sauce_nao.api_key.as_str()),
            yodaspeak: yodaspeak::Client::new(),
            tiktok_data,
            encoder_task,

            db,

            config,
        })
    }

    /// Shutdown anything that needs to be shut down.
    ///
    /// Errors are logged to the console,
    /// but not returned to the user as it is assumed that they don't matter in the middle of a shutdown.
    pub async fn shutdown(&self) {
        if let Err(error) = self.encoder_task.shutdown().await {
            error!("{error:?}");
        }
    }
}
