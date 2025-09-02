use crate::client::base::{self, BaseClient, ClientParams};
use anyhow::{Ok, Result};
use reqwest::Client;
use reqwest::Url;
use serde::Serialize;

struct ToncenterV3Client {
    params: ClientParams,
    client: Client,
}

impl BaseClient for ToncenterV3Client {
    fn new(params: Option<ClientParams>) -> Self {
        Self {
            params: params.unwrap_or_default(),
            client: Client::new(),
        }
    }
    
    async fn get(&self, endpoint: &str, params: Option<std::collections::HashMap<&str, &str>>) -> Result<reqwest::Response> {
        let url = match &self.params.base_url {
            Some(url) => url,
            None => "https://toncenter.com/api/v3",
        };
        let url = match params {
            Some(params) => Url::parse_with_params(&url, params)?,
            None => Url::parse(&url)?,
        };
        let response = self.client.get(url).send().await?;
        Ok(response)
    }
    
    async fn post<T: Serialize>(&self, endpoint: &str, body: &T) -> Result<reqwest::Response> {
        let base_url = match &self.params.base_url {
            Some(url) => url,
            None => "https://toncenter.com/api/v3",
        };
        let url = format!("{}/{}", base_url.trim_end_matches('/'), endpoint.trim_start_matches('/'));
        
        let mut request = self.client
            .post(&url)
            .json(body);
            
        if let Some(api_key) = &self.params.api_key {
            request = request.header("X-API-Key", api_key);
        }
        
        let response = request.send().await?;
        Ok(response)
    }
}
