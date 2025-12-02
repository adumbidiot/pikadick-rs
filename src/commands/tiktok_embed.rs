use crate::{
    ClientDataKey,
    LoadingReaction,
    TikTokEmbedFlags,
    util::{
        EncoderTask,
        TimedCache,
        TimedCacheEntry,
    },
};
use anyhow::{
    Context as _,
    ensure,
};
use camino::{
    Utf8Path,
    Utf8PathBuf,
};
use nd_util::{
    ArcAnyhowError,
    DropRemovePath,
};
use pikadick_util::RequestMap;
use serenity::{
    builder::{
        CreateAttachment,
        CreateEmbed,
        CreateInteractionResponse,
        CreateInteractionResponseMessage,
        CreateMessage,
    },
    model::prelude::*,
    prelude::*,
};
use std::sync::Arc;
use tokio_stream::StreamExt;
use tracing::{
    info,
    warn,
};
use url::Url;

const FILE_SIZE_LIMIT_BYTES: u64 = 8 * 1024 * 1024;
const TARGET_FILE_SIZE_BYTES: u64 = 7 * 1024 * 1024;
const ENCODER_PREFERENCE_LIST: &[&str] = &[
    "h264_nvenc",
    "h264_amf",
    "h264_qsv",
    "h264_mf",
    "h264_v4l2m2m",
    "h264_vaapi",
    "h264_omx",
    "libx264",
    "libx264rgb",
];

type VideoDownloadRequestMap = Arc<RequestMap<String, Result<Arc<Utf8Path>, ArcAnyhowError>>>;

/// Calculate the target bitrate.
///
/// target_size is in kilobits.
/// target_duration is in seconds.
/// the bitrate is in kilobits
fn calc_target_bitrate(target_size: u64, duration: u64) -> u64 {
    // https://stackoverflow.com/questions/29082422/ffmpeg-video-compression-specific-file-size

    target_size / duration
}

async fn select_best_encoder(encoder_task: &EncoderTask) -> anyhow::Result<&'static str> {
    let mut encoders = encoder_task
        .get_encoders(true)
        .await
        .context("failed to get encoders")?;

    // Keep only h264 encoders
    encoders.retain(|encoder| encoder.description.ends_with("(codec h264)"));
    info!("found h264 encoders: {encoders:#?}");

    let mut best_encoder_index = None;
    for encoder in encoders {
        let encoder_index = ENCODER_PREFERENCE_LIST
            .iter()
            .position(|name| **name == *encoder.name);
        let encoder_index = match encoder_index {
            Some(encoder_index) => encoder_index,
            None => continue,
        };
        if best_encoder_index.is_none_or(|best_encoder_index| best_encoder_index > encoder_index) {
            best_encoder_index = Some(encoder_index);
        }
    }

    let best_encoder_index = best_encoder_index.context("failed to select an encoder")?;
    let best_encoder = ENCODER_PREFERENCE_LIST[best_encoder_index];

    Ok(best_encoder)
}

#[derive(Debug)]
struct InnerTikTokData {
    /// The inner client
    client: tiktok::Client,

    /// The encoder task
    encoder_task: EncoderTask,

    /// A cache of post urls => post pages
    post_page_cache: TimedCache<String, tiktok::Post>,

    /// The path to tiktok's cache dir
    video_download_cache_path: Utf8PathBuf,

    /// The request map for making requests for video downloads.
    video_download_request_map: VideoDownloadRequestMap,

    /// The best video encoder from ffmpeg.
    video_encoder: &'static str,
}

/// TikTok Data
#[derive(Debug, Clone)]
pub struct TikTokData {
    inner: Arc<InnerTikTokData>,
}

impl TikTokData {
    /// Make a new [`TikTokData`].
    pub async fn new<P>(cache_dir: P, encoder_task: EncoderTask) -> anyhow::Result<Self>
    where
        P: AsRef<Utf8Path>,
    {
        let cache_dir = cache_dir.as_ref();
        let video_download_cache_path = cache_dir.join("tiktok");

        // TODO: Expand into proper filecache manager
        tokio::fs::create_dir_all(&video_download_cache_path)
            .await
            .context("failed to create tiktok cache dir")?;

        let best_encoder = select_best_encoder(&encoder_task).await?;
        info!("selected encoder \"{best_encoder}\"");

        Ok(Self {
            inner: Arc::new(InnerTikTokData {
                client: tiktok::Client::new(),

                encoder_task,

                post_page_cache: TimedCache::new(),

                video_download_cache_path,
                video_download_request_map: Arc::new(RequestMap::new()),
                video_encoder: best_encoder,
            }),
        })
    }

    /// Get a post page, using the cache if needed
    pub async fn get_post_cached(
        &self,
        url: &str,
    ) -> anyhow::Result<Arc<TimedCacheEntry<tiktok::Post>>> {
        if let Some(post_page) = self.inner.post_page_cache.get_if_fresh(url) {
            return Ok(post_page);
        }

        let video_id = Url::parse(url)?
            .path_segments()
            .context("missing path")?
            .next_back()
            .context("missing video id")?
            .parse()
            .context("invalid video id")?;

        let mut feed = self
            .inner
            .client
            .get_feed(Some(video_id))
            .await
            .context("failed to get feed")?;
        ensure!(!feed.aweme_list.is_empty(), "missing post");

        let post = feed.aweme_list.swap_remove(0);
        ensure!(post.aweme_id == video_id);

        Ok(self
            .inner
            .post_page_cache
            .insert_and_get(url.to_string(), post))
    }

    async fn create_reencoded_file(
        &self,
        id: u64,
        url: Url,
        video_duration: u64,
    ) -> anyhow::Result<Utf8PathBuf> {
        ensure!(
            url.path_segments()
                .and_then(|mut path| path.next_back()?.rsplit_once('.'))
                .is_some_and(|(_stem, extension)| extension == "mp4")
        );

        let client = self.inner.client.client.clone();
        let video_encoder = self.inner.video_encoder;

        let reencoded_file_name = format!("{id}-reencoded.mp4");
        let reencoded_file_path = self
            .inner
            .video_download_cache_path
            .join(reencoded_file_name);

        let file_name = format!("{id}.mp4");
        let file_path = self.inner.video_download_cache_path.join(file_name);

        if tokio::fs::try_exists(&reencoded_file_path)
            .await
            .context("failed to get metadata of re-encoded file")?
        {
            // The reencoded file is present. Use it.
            return Ok(reencoded_file_path);
        }

        // Get the metadata of the raw file.
        // Download it if needed.
        let metadata = match crate::util::try_metadata(&file_path).await {
            Ok(Some(metadata)) => metadata,
            Ok(None) => {
                // File not present. Download it.
                info!("downloading tiktok video with with id {id:?} from url {url:?}");

                async {
                    nd_util::download_to_path(&client, url.as_str(), &file_path).await?;
                    tokio::fs::metadata(&file_path)
                        .await
                        .context("failed to get file metadata")
                }
                .await?
            }
            Err(e) => {
                return Err(e).context("failed to get metadata of file");
            }
        };

        // If the file is less than than 8mb, we don't need to re-encode it
        if metadata.len() < FILE_SIZE_LIMIT_BYTES {
            return Ok(file_path);
        }

        // We target half of the maximum size to give ourselves some lee-way.
        // This merely sets the target bit-rate, and we don't take into account audio size.
        let target_bitrate =
            calc_target_bitrate((TARGET_FILE_SIZE_BYTES / 1024) * 8 / 2, video_duration);
        let reencoded_file_path_tmp_1 =
            DropRemovePath::new(reencoded_file_path.with_extension("1.tmp"));

        info!(
            "re-encoding tiktok video {:?} to {:?} @ video bitrate {}",
            file_path,
            reencoded_file_path_tmp_1.display(),
            target_bitrate
        );

        {
            let mut stream = self
                .inner
                .encoder_task
                .encode()
                .input(&file_path)
                .output(&*reencoded_file_path_tmp_1)
                .audio_codec("copy")
                .video_codec(video_encoder)
                .video_bitrate(format!("{target_bitrate}K"))
                .output_format("mp4")
                .try_send()
                .await
                .context("failed to start re-encoding")?;

            let mut maybe_exit_status = None;
            while let Some(msg) = stream.next().await {
                match msg.context("ffmpeg stream error") {
                    Ok(tokio_ffmpeg_cli::Event::ExitStatus(exit_status)) => {
                        maybe_exit_status = Some(exit_status);
                    }
                    Ok(tokio_ffmpeg_cli::Event::Progress(_progress)) => {
                        // For now, we don't care about progress as there is no way to report it to the user on discord.
                    }
                    Ok(tokio_ffmpeg_cli::Event::Unknown(_line)) => {
                        // warn!("unknown ffmpeg line: `{}`", line);
                        // We don't care about unkown lines
                    }
                    Err(error) => {
                        warn!("{error:?}");
                    }
                }
            }

            let exit_status = maybe_exit_status.context("stream did not report an exit status")?;

            // Validate exit status
            ensure!(exit_status.success(), "invalid exit status");
        }

        // The RPI's ffmpeg produces invalid mp4 files.
        // Until we can investigate and fix, transcode the file to try to let ffmpeg fix it.
        let reencoded_file_path_tmp_2 =
            DropRemovePath::new(reencoded_file_path.with_extension("2.tmp"));

        {
            let mut stream = self
                .inner
                .encoder_task
                .encode()
                .input(&*reencoded_file_path_tmp_1)
                .output(&*reencoded_file_path_tmp_2)
                .audio_codec("copy")
                .video_codec("copy")
                .output_format("mp4")
                .try_send()
                .await
                .context("failed to start transcoding")?;

            let mut maybe_exit_status = None;
            while let Some(msg) = stream.next().await {
                match msg.context("ffmpeg stream error") {
                    Ok(tokio_ffmpeg_cli::Event::ExitStatus(exit_status)) => {
                        maybe_exit_status = Some(exit_status);
                    }
                    Ok(tokio_ffmpeg_cli::Event::Progress(_progress)) => {
                        // For now, we don't care about progress as there is no way to report it to the user on discord.
                    }
                    Ok(tokio_ffmpeg_cli::Event::Unknown(_line)) => {
                        // warn!("unknown ffmpeg line: `{}`", line);
                        // We don't care about unkown lines
                    }
                    Err(error) => {
                        warn!("{error:?}");
                    }
                }
            }

            let exit_status = maybe_exit_status.context("stream did not report an exit status")?;

            // Validate exit status
            ensure!(exit_status.success(), "invalid exit status");
        }

        let mut reencoded_file_path_tmp = reencoded_file_path_tmp_2;

        // Validate file size
        let metadata = tokio::fs::metadata(&reencoded_file_path_tmp)
            .await
            .context("failed to get metadata of encoded file")?;
        let metadata_len = metadata.len();
        ensure!(
            metadata_len < FILE_SIZE_LIMIT_BYTES,
            "re-encoded file size ({metadata_len}) is larger than the limit {FILE_SIZE_LIMIT_BYTES}",
        );

        // Rename the tmp file to be the actual name.
        tokio::fs::rename(&*reencoded_file_path_tmp, &reencoded_file_path)
            .await
            .context("failed to rename temp file")?;

        // "Persist" the tmp file, as in don't try to remove it
        reencoded_file_path_tmp.persist();

        Ok(reencoded_file_path)
    }

    /// Get video data, using the cache if needed.
    pub async fn get_video_data_cached(
        &self,
        id: u64,
        url: &Url,
        video_duration: u64,
    ) -> anyhow::Result<Arc<Utf8Path>> {
        self.inner
            .video_download_request_map
            .get_or_fetch(id.to_string(), || {
                let self_clone = self.clone();
                let url = url.clone();

                async move {
                    self_clone
                        .create_reencoded_file(id, url, video_duration)
                        .await
                        .map(Arc::from)
                        .map_err(ArcAnyhowError::new)
                }
            })
            .await
            .map_err(From::from)
    }

    /// Try embedding a url
    pub async fn try_embed_url(
        &self,
        ctx: &Context,
        msg: &Message,
        url: &Url,
        loading_reaction: &mut Option<LoadingReaction>,
        delete_link: bool,
    ) -> anyhow::Result<()> {
        let (video_url, video_id, video_duration) = {
            let post = self.get_post_cached(url.as_str()).await?;
            let post = post.data();

            let video_url = post
                .video
                .download_addr
                .url_list
                .first()
                .context("missing video url")?
                .clone();
            let video_id: u64 = post.aweme_id;
            let video_duration = post.video.duration;

            (video_url, video_id, video_duration)
        };

        let video_path = self
            .get_video_data_cached(video_id, &video_url, video_duration)
            .await
            .context("failed to download tiktok video")?;

        let file = CreateAttachment::path(video_path.as_std_path()).await?;
        let message_builder = CreateMessage::new().add_file(file);
        msg.channel_id
            .send_message(&ctx.http, message_builder)
            .await?;

        if let Some(mut loading_reaction) = loading_reaction.take() {
            loading_reaction.send_ok();

            if delete_link {
                msg.delete(&ctx.http)
                    .await
                    .context("failed to delete original message")?;
            }
        }

        Ok(())
    }
}

/// Convert a bool to a str
fn bool_to_str(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

/// Options for tiktok-embed
#[derive(Debug, pikadick_slash_framework::FromOptions)]
struct TikTokEmbedOptions {
    /// Whether embeds should be enabled for this server
    #[pikadick_slash_framework(description = "Whether embeds should be enabled for this server")]
    enable: Option<bool>,

    /// Whether source messages should be deleted
    #[pikadick_slash_framework(
        rename = "delete-link",
        description = "Whether source messages should be deleted"
    )]
    delete_link: Option<bool>,
}

/// Create a slash command
pub fn create_slash_command() -> anyhow::Result<pikadick_slash_framework::Command> {
    use pikadick_slash_framework::FromOptions;

    pikadick_slash_framework::CommandBuilder::new()
        .name("tiktok-embed")
        .description("Configure tiktok embeds for this server")
        .check(crate::checks::admin::create_slash_check)
        .arguments(TikTokEmbedOptions::get_argument_params()?.into_iter())
        .on_process(|ctx, interaction, args: TikTokEmbedOptions| async move {
            let data_lock = ctx.data.read().await;
            let client_data = data_lock.get::<ClientDataKey>().unwrap();
            let db = client_data.db.clone();
            drop(data_lock);

            let guild_id = match interaction.guild_id {
                Some(id) => id,
                None => {
                    let message_builder = CreateInteractionResponseMessage::new()
                        .content("Missing server id. Are you in a server right now?");
                    let response = CreateInteractionResponse::Message(message_builder);
                    interaction.create_response(&ctx.http, response).await?;
                    return Ok(());
                }
            };

            let mut set_flags = TikTokEmbedFlags::empty();
            let mut unset_flags = TikTokEmbedFlags::empty();

            if let Some(enable) = args.enable {
                if enable {
                    set_flags.insert(TikTokEmbedFlags::ENABLED);
                } else {
                    unset_flags.insert(TikTokEmbedFlags::ENABLED);
                }
            }

            if let Some(enable) = args.delete_link {
                if enable {
                    set_flags.insert(TikTokEmbedFlags::DELETE_LINK);
                } else {
                    unset_flags.insert(TikTokEmbedFlags::DELETE_LINK);
                }
            }

            let (_old_flags, new_flags) = db
                .set_tiktok_embed_flags(guild_id, set_flags, unset_flags)
                .await?;

            let embed_builder = CreateEmbed::new()
                .title("TikTok Embeds")
                .field(
                    "Enabled?",
                    bool_to_str(new_flags.contains(TikTokEmbedFlags::ENABLED)),
                    false,
                )
                .field(
                    "Delete link?",
                    bool_to_str(new_flags.contains(TikTokEmbedFlags::DELETE_LINK)),
                    false,
                );
            let message_builder = CreateInteractionResponseMessage::new().embed(embed_builder);
            let response = CreateInteractionResponse::Message(message_builder);
            interaction.create_response(&ctx.http, response).await?;

            Ok(())
        })
        .build()
        .context("failed to build command")
}
