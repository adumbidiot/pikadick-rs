use crate::{
    Config,
    commands::{
        nekos::NekosClient,
        r6tracker::R6TrackerClient,
        rule34::Rule34Client,
    },
};
use std::sync::Arc;

#[derive(Debug)]
pub struct PoiseData {
    pub nekos_client: NekosClient,
    pub r6tracker_client: R6TrackerClient,
    pub rule34_client: Rule34Client,
    pub xkcd_client: xkcd::Client,
    pub yodaspeak_client: yodaspeak::Client,
}

impl PoiseData {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            nekos_client: NekosClient::new(),
            r6tracker_client: R6TrackerClient::new(),
            rule34_client: Rule34Client::new(config.rule34.user_id, &config.rule34.api_key),
            xkcd_client: xkcd::Client::new(),
            yodaspeak_client: yodaspeak::Client::new(),
        }
    }
}
