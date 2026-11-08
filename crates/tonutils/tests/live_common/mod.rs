#![allow(dead_code)]

use std::{env, time::Duration};

use anyhow::{Context, Result, bail};

pub fn load_global_config(variable: &str) -> Result<String> {
    let json = env::var(variable).with_context(|| format!("set {variable}"))?;
    if json.trim().is_empty() {
        bail!("{variable} is empty");
    }
    Ok(json)
}

pub fn elapsed_millis(started: std::time::Instant) -> u128 {
    started.elapsed().as_millis()
}

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
