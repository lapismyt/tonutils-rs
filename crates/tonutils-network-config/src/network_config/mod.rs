use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("network config has no liteservers")]
    EmptyLiteServers,
    #[error("liteserver index {index} is out of bounds for {len} configured liteservers")]
    LiteServerIndexOutOfBounds { index: usize, len: usize },
    #[error("global config JSON does not contain a supported bootstrap address")]
    NoBootstrapAddresses,
    #[error("bootstrap address is not usable: {0}")]
    InvalidBootstrapAddress(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LiteServerBlacklistParseError {
    #[error("invalid liteserver index `{value}`")]
    InvalidIndex { value: String },
    #[error("invalid liteserver id encoding `{value}`")]
    InvalidIdEncoding { value: String },
    #[error("invalid liteserver id length for `{value}`: expected 32 bytes, got {len}")]
    InvalidIdLength { value: String, len: usize },
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "@type")]
pub enum ConfigPublicKey {
    #[serde(rename = "pub.ed25519")]
    Ed25519 {
        #[serde_as(as = "serde_with::base64::Base64")]
        key: [u8; 32],
    },
}

#[derive(Debug, Clone)]
pub struct LiteServerAddress(Ipv4Addr);

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigLiteServer {
    #[serde_as(as = "serde_with::FromInto<i32>")]
    pub ip: LiteServerAddress,
    pub port: u16,
    pub id: ConfigPublicKey,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigGlobal {
    pub liteservers: Vec<ConfigLiteServer>,
}

/// A validated network endpoint extracted from a global configuration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigBootstrapAddress {
    pub address: SocketAddr,
    pub public_key: Option<[u8; 32]>,
}

impl ConfigBootstrapAddress {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.address.port() != 0
            && !self.address.ip().is_unspecified()
            && self.public_key.is_some_and(|key| key != [0; 32])
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiteServerBlacklist {
    indexes: BTreeSet<usize>,
    ids: BTreeSet<[u8; 32]>,
}

impl FromStr for ConfigGlobal {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

impl From<ConfigPublicKey> for [u8; 32] {
    fn from(value: ConfigPublicKey) -> Self {
        match value {
            ConfigPublicKey::Ed25519 { key } => key,
        }
    }
}

impl ConfigPublicKey {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        match self {
            ConfigPublicKey::Ed25519 { key } => key,
        }
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        *self.as_bytes()
    }
}

impl Deref for LiteServerAddress {
    type Target = Ipv4Addr;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for LiteServerAddress {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<i32> for LiteServerAddress {
    fn from(v: i32) -> Self {
        Self(Ipv4Addr::from(v.cast_unsigned()))
    }
}

impl From<LiteServerAddress> for i32 {
    fn from(v: LiteServerAddress) -> Self {
        u32::from(v.0).cast_signed()
    }
}

impl ConfigLiteServer {
    #[must_use]
    pub fn socket_addr(&self) -> SocketAddrV4 {
        SocketAddrV4::new(*self.ip, self.port)
    }

    #[must_use]
    pub fn public_key(&self) -> [u8; 32] {
        self.id.to_bytes()
    }
}

impl ConfigGlobal {
    /// Returns the configured liteserver at `index`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::LiteServerIndexOutOfBounds`] when `index` is not configured.
    pub fn liteserver(&self, index: usize) -> Result<&ConfigLiteServer, ConfigError> {
        self.liteservers
            .get(index)
            .ok_or(ConfigError::LiteServerIndexOutOfBounds {
                index,
                len: self.liteservers.len(),
            })
    }

    /// Returns the first configured liteserver.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::EmptyLiteServers`] when no liteservers are configured.
    pub fn first_liteserver(&self) -> Result<&ConfigLiteServer, ConfigError> {
        self.liteservers
            .first()
            .ok_or(ConfigError::EmptyLiteServers)
    }

    #[must_use]
    pub fn select_liteservers(
        &self,
        limit: usize,
        blacklist: &LiteServerBlacklist,
    ) -> Vec<(usize, &ConfigLiteServer)> {
        self.liteservers
            .iter()
            .enumerate()
            .filter(|(index, liteserver)| !blacklist.contains(*index, liteserver))
            .take(limit)
            .collect()
    }

    /// Returns liteserver endpoints as generic bootstrap addresses.
    ///
    /// This is intentionally limited to data represented by [`ConfigGlobal`].
    /// Applications that retain the original JSON can additionally call
    /// [`extract_bootstrap_addresses`] to inspect DHT static nodes.
    #[must_use]
    pub fn bootstrap_addresses(&self) -> Vec<ConfigBootstrapAddress> {
        self.liteservers
            .iter()
            .map(|server| ConfigBootstrapAddress {
                address: SocketAddr::V4(server.socket_addr()),
                public_key: Some(server.public_key()),
            })
            .filter(ConfigBootstrapAddress::is_valid)
            .collect()
    }
}

/// Extracts supported DHT and liteserver endpoints from raw global-config JSON.
///
/// The parser is deliberately tolerant of unrelated fields and of the several
/// address-list shapes used by historical TON configs. It performs no network
/// access and returns only syntactically valid socket addresses.
pub fn extract_bootstrap_addresses(json: &str) -> Result<Vec<ConfigBootstrapAddress>, ConfigError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| ConfigError::NoBootstrapAddresses)?;
    let mut addresses = BTreeSet::new();

    if let Some(servers) = value
        .get("liteservers")
        .and_then(serde_json::Value::as_array)
    {
        for server in servers {
            let Some(ip) = server
                .get("ip")
                .and_then(serde_json::Value::as_i64)
                .and_then(|ip| i32::try_from(ip).ok())
            else {
                continue;
            };
            let Some(port) = server
                .get("port")
                .and_then(serde_json::Value::as_u64)
                .and_then(|port| u16::try_from(port).ok())
            else {
                continue;
            };
            let key = server.get("id").and_then(parse_public_key_value);
            let candidate = ConfigBootstrapAddress {
                address: SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::from(ip.cast_unsigned()),
                    port,
                )),
                public_key: key,
            };
            if candidate.is_valid() {
                addresses.insert(candidate);
            }
        }
    }

    if let Some(nodes) = value
        .pointer("/dht/static_nodes/nodes")
        .and_then(serde_json::Value::as_array)
    {
        for node in nodes {
            let key = node.get("id").and_then(parse_public_key_value);
            let Some(addrs) = node.get("addr_list").and_then(|list| {
                list.get("addrs")
                    .or_else(|| list.get("addresses"))
                    .and_then(serde_json::Value::as_array)
            }) else {
                continue;
            };
            for addr in addrs {
                let Some(ip) = addr
                    .get("ip")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|ip| i32::try_from(ip).ok())
                else {
                    continue;
                };
                let Some(port) = addr
                    .get("port")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|port| u16::try_from(port).ok())
                else {
                    continue;
                };
                let candidate = ConfigBootstrapAddress {
                    address: SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip.cast_unsigned())), port),
                    public_key: key,
                };
                if candidate.is_valid() {
                    addresses.insert(candidate);
                }
            }
        }
    }

    let result = addresses.into_iter().collect::<Vec<_>>();
    if result.is_empty() {
        Err(ConfigError::NoBootstrapAddresses)
    } else {
        Ok(result)
    }
}

fn parse_public_key_value(value: &serde_json::Value) -> Option<[u8; 32]> {
    let key = value
        .get("key")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.as_str())?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(key)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(key))
        .ok()?;
    decoded.try_into().ok()
}

impl LiteServerBlacklist {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses liteserver indexes and public keys from `tokens`.
    ///
    /// # Errors
    ///
    /// Returns an error when a token is not a valid index or 32-byte public key.
    pub fn parse_tokens<'a>(
        tokens: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, LiteServerBlacklistParseError> {
        let mut blacklist = Self::new();
        for token in tokens {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            blacklist.insert_token(token)?;
        }
        Ok(blacklist)
    }

    #[must_use]
    pub fn contains(&self, index: usize, liteserver: &ConfigLiteServer) -> bool {
        self.indexes.contains(&index) || self.ids.contains(&liteserver.public_key())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indexes.is_empty() && self.ids.is_empty()
    }

    fn insert_token(&mut self, token: &str) -> Result<(), LiteServerBlacklistParseError> {
        if let Some(index) = token.strip_prefix("index:") {
            self.insert_index(index)
        } else if let Some(id) = token.strip_prefix("id:") {
            self.insert_id(id)
        } else if token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            self.insert_id(token)
        } else if token.bytes().all(|byte| byte.is_ascii_digit()) {
            self.insert_index(token)
        } else {
            self.insert_id(token)
        }
    }

    fn insert_index(&mut self, value: &str) -> Result<(), LiteServerBlacklistParseError> {
        let index =
            value
                .parse::<usize>()
                .map_err(|_| LiteServerBlacklistParseError::InvalidIndex {
                    value: value.to_owned(),
                })?;
        self.indexes.insert(index);
        Ok(())
    }

    fn insert_id(&mut self, value: &str) -> Result<(), LiteServerBlacklistParseError> {
        let bytes = decode_liteserver_id(value)?;
        if bytes.len() != 32 {
            return Err(LiteServerBlacklistParseError::InvalidIdLength {
                value: value.to_owned(),
                len: bytes.len(),
            });
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        self.ids.insert(id);
        Ok(())
    }
}

impl FromStr for LiteServerBlacklist {
    type Err = LiteServerBlacklistParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_tokens(s.split(','))
    }
}

fn decode_liteserver_id(value: &str) -> Result<Vec<u8>, LiteServerBlacklistParseError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return hex::decode(value).map_err(|_| LiteServerBlacklistParseError::InvalidIdEncoding {
            value: value.to_owned(),
        });
    }

    for engine in [
        &base64::engine::general_purpose::STANDARD,
        &base64::engine::general_purpose::STANDARD_NO_PAD,
        &base64::engine::general_purpose::URL_SAFE,
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(bytes) = engine.decode(value) {
            return Ok(bytes);
        }
    }

    Err(LiteServerBlacklistParseError::InvalidIdEncoding {
        value: value.to_owned(),
    })
}

#[cfg(test)]
mod tests;
