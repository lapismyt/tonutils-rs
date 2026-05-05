use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("network config has no liteservers")]
    EmptyLiteServers,
    #[error("liteserver index {index} is out of bounds for {len} configured liteservers")]
    LiteServerIndexOutOfBounds { index: usize, len: usize },
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

impl FromStr for ConfigGlobal {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s)
    }
}

impl Into<[u8; 32]> for ConfigPublicKey {
    fn into(self) -> [u8; 32] {
        match self {
            ConfigPublicKey::Ed25519 { key } => key,
        }
    }
}

impl ConfigPublicKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        match self {
            ConfigPublicKey::Ed25519 { key } => key,
        }
    }

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
        Self(Ipv4Addr::from(v as u32))
    }
}

impl From<LiteServerAddress> for i32 {
    fn from(v: LiteServerAddress) -> Self {
        u32::from(v.0) as i32
    }
}

impl ConfigLiteServer {
    pub fn socket_addr(&self) -> SocketAddrV4 {
        SocketAddrV4::new(*self.ip, self.port)
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.id.to_bytes()
    }
}

impl ConfigGlobal {
    pub fn liteserver(&self, index: usize) -> Result<&ConfigLiteServer, ConfigError> {
        self.liteservers
            .get(index)
            .ok_or(ConfigError::LiteServerIndexOutOfBounds {
                index,
                len: self.liteservers.len(),
            })
    }

    pub fn first_liteserver(&self) -> Result<&ConfigLiteServer, ConfigError> {
        self.liteservers
            .first()
            .ok_or(ConfigError::EmptyLiteServers)
    }
}

#[cfg(test)]
mod tests;
