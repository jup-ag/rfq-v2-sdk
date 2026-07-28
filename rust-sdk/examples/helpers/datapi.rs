use serde::Deserialize;
use std::collections::HashMap;
use std::error::Error;

#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenPriceData {
    pub usd_price: f64,
    pub block_id: u64,
    pub decimals: u8,
    pub price_change24h: f64,
}

pub type DatapiResponse = HashMap<String, TokenPriceData>;

pub struct DatapiClient {
    client: reqwest::Client,
    host: String,
}

impl DatapiClient {
    pub fn new(url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            host: url.to_string(),
        }
    }

    pub async fn fetch_prices(
        &self,
        token_ids: &[String],
    ) -> Result<DatapiResponse, Box<dyn Error + Send + Sync>> {
        let ids = token_ids.join(",");
        let url = format!("{}/v1/prices?ids={}", self.host, ids);

        tracing::debug!("fetching prices for tokens {:?}", token_ids);

        let response = self
            .client
            .get(&url)
            .header("accept", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()).into());
        }

        let price_response: DatapiResponse = response.json().await?;
        Ok(price_response)
    }
}
