use crate::{
    ClientDataKey,
    PoiseContext,
    PoiseError,
};
use anyhow::Context as _;
use tracing::{
    error,
    info,
};

#[poise::command(
    slash_command,
    description_localized("en-US", "Translate into what yoda would say"),
    check = "crate::checks::enabled"
)]
pub async fn yodaspeak(
    ctx: PoiseContext<'_>,
    #[description = "The message to translate"] message: String,
) -> Result<(), PoiseError> {
    let data_lock = ctx.serenity_context().data.read().await;
    let client_data = data_lock.get::<ClientDataKey>().unwrap();
    let client = client_data.yodaspeak.clone();
    drop(data_lock);

    info!("Translating {message:?} to yodaspeak");
    ctx.defer().await?;

    let result = client
        .translate(message.as_str())
        .await
        .context("failed to translate");

    let content = match result {
        Ok(translated) => translated,
        Err(error) => {
            error!("{error:?}");
            format!("{error:?}")
        }
    };

    ctx.reply(content).await?;

    Ok(())
}
