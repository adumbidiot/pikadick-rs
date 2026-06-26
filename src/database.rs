mod disabled_commands;
mod kv_store;
pub mod model;
mod reddit_embed;
mod rule34;
mod tic_tac_toe;
mod tiktok_embed;

pub use self::{
    model::{
        JiffTimestampWrapper,
        Rule34Post,
        Rule34QueryStat,
    },
    tic_tac_toe::{
        TicTacToeCreateGameError,
        TicTacToeTryMoveError,
        TicTacToeTryMoveResponse,
    },
};
use anyhow::Context;
use camino::Utf8PathBuf;
use nd_async_rusqlite::WalPool;
use tracing::{
    debug,
    error,
};

// Setup
const SETUP_TABLES_SQL: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/sql/setup_tables.sql"));

/// The database
#[derive(Clone, Debug)]
pub struct Database {
    database: nd_async_rusqlite::WalPool,
}

impl Database {
    //// Make a new [`Database`].
    pub async fn new<P>(path: P) -> anyhow::Result<Self>
    where
        P: Into<Utf8PathBuf>,
    {
        let path = path.into();
        let database = WalPool::builder()
            .readers(4)
            .writer_setup(|database| {
                debug!("setting up writer");

                database.execute_batch(SETUP_TABLES_SQL)?;
                Ok(())
            })
            .reader_setup(|_database| {
                debug!("setting up reader");
                Ok(())
            })
            .open(path)
            .await
            .context("failed to open database")?;

        Ok(Self { database })
    }

    /// Read from the database.
    async fn read<F, R>(&self, func: F) -> anyhow::Result<R>
    where
        F: FnOnce(&mut rusqlite::Connection) -> R + Send + 'static,
        R: Send + 'static,
    {
        Ok(self.database.read(move |database| func(database)).await?)
    }

    /// Write to the database.
    async fn write<F, R>(&self, func: F) -> anyhow::Result<R>
    where
        F: FnOnce(&mut rusqlite::Connection) -> R + Send + 'static,
        R: Send + 'static,
    {
        Ok(self.database.write(move |database| func(database)).await?)
    }

    // Compat
    async fn access_db<F, R>(&self, func: F) -> anyhow::Result<R>
    where
        F: FnOnce(&mut rusqlite::Connection) -> R + Send + 'static,
        R: Send + 'static,
    {
        self.write(func).await
    }

    /// Close the database
    pub async fn close(&self) -> anyhow::Result<()> {
        // Failing to run shutdown commands is not critical and should not prevent shutdown.
        if let Err(error) = self
            .database
            .write(|database| {
                database.execute("PRAGMA OPTIMIZE;", [])?;
                database.execute("VACUUM;", [])
            })
            .await
            .context("failed to access database")
            .and_then(|v| v.context("failed to execute shutdown commands"))
        {
            error!("{error:?}");
        }
        self.database
            .close()
            .await
            .context("failed to close database")?;

        Ok(())
    }
}
