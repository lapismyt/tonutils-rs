use std::collections::HashMap;

use reqwest::Client;
use anyhow::Result;
use serde::Serialize;


#[derive(Debug, Clone)]
pub struct ClientParams {
    pub timeout: Option<u64>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub rps: Option<u64>,
    pub max_retries: Option<u64>,
}

impl Default for ClientParams {
    fn default() -> Self {
        Self {
            timeout: Some(5),
            api_key: None,
            base_url: None,
            rps: Some(10),
            max_retries: Some(5),
        }
    }
}

pub trait BaseClient {
    fn new(params: Option<ClientParams>) -> Self;
    async fn get(&self, endpoint: &str, params: Option<HashMap<&str, &str>>) -> Result<reqwest::Response>;
    async fn post<T: Serialize>(&self, endpoint: &str, body: &T) -> Result<reqwest::Response>;
}
