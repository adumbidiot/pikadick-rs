use crate::{
    types::{
        ApiResponse,
        UserData,
    },
    Error,
};

/// An R6Stats client
#[derive(Debug, Clone)]
pub struct Client {
    client: reqwest::Client,
}

impl Client {
    /// Make a new client
    pub fn new() -> Self {
        Client {
            client: reqwest::Client::new(),
        }
    }

    // TODO: Add non-pc support
    /// Search for a PC user's profile by name.
    pub async fn search(&self, name: &str) -> Result<Vec<UserData>, Error> {
        let url = format!("https://r6stats.com/api/player-search/{}/pc", name);
        let text = self.client.get(&url).send().await?.text().await?;
        let res: ApiResponse<Vec<UserData>> = serde_json::from_str(&text)?;

        Ok(res.data)
    }
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn it_works() {
        let client = Client::new();
        let user_list = client.search("KingGeorge").await.unwrap();
        assert!(!user_list.is_empty());
        dbg!(&user_list);
    }

    #[tokio::test]
    async fn invalid_search() {
        let client = Client::new();
        let user_list = client.search("ygwdauiwgd").await.unwrap();
        assert!(user_list.is_empty());
        dbg!(&user_list);
    }
}
