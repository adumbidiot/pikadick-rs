use crate::{
    ClientDataKey,
    PoiseContext,
    PoiseError,
};
use anyhow::Context;
use tracing::error;

#[poise::command(
    slash_command,
    description_localized("en-US", "Get a random comic from https://xkcd.com/"),
    check = "crate::checks::enabled"
)]
pub async fn xkcd(ctx: PoiseContext<'_>) -> Result<(), PoiseError> {
    let data_lock = ctx.serenity_context().data.read().await;
    let client_data = data_lock.get::<ClientDataKey>().unwrap();
    let client = client_data.xkcd_client.clone();
    drop(data_lock);

    let content = match client
        .get_random()
        .await
        .context("failed to get xkcd comic")
    {
        Ok(data) => data.into(),
        Err(error) => {
            error!("{error:?}");
            format!("{error:?}")
        }
    };
    ctx.reply(content).await?;

    Ok(())
}
