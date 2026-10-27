/// Public key used by ADNL protocol messages.
#[derive(Debug, Copy, Clone, Eq, PartialEq, tl_proto::TlRead, tl_proto::TlWrite)]
#[tl(boxed)]
pub enum AdnlTlPublicKeyRef<'tl> {
    #[tl(id = 0x4813b4c6, size_hint = 32)]
    Ed25519 { key: &'tl [u8; 32] },
    #[tl(id = 0x34ba45cb)]
    Overlay { name: &'tl [u8] },
    #[tl(id = 0x2dbcadd4, size_hint = 32)]
    Aes { key: &'tl [u8; 32] },
    #[tl(id = 0xb61f450a)]
    Unencoded { data: &'tl [u8] },
}

impl AdnlTlPublicKeyRef<'_> {
    #[must_use]
    pub fn as_equivalent_owned(&self) -> AdnlTlPublicKeyOwned {
        match self {
            Self::Ed25519 { key } => AdnlTlPublicKeyOwned::Ed25519 { key: **key },
            Self::Overlay { name } => AdnlTlPublicKeyOwned::Overlay {
                name: name.to_vec(),
            },
            Self::Aes { key } => AdnlTlPublicKeyOwned::Aes { key: **key },
            Self::Unencoded { data } => AdnlTlPublicKeyOwned::Unencoded {
                data: data.to_vec(),
            },
        }
    }
}

/// Owned ADNL protocol public key.
#[derive(Debug, Clone, Eq, PartialEq, tl_proto::TlRead, tl_proto::TlWrite)]
#[tl(boxed)]
pub enum AdnlTlPublicKeyOwned {
    #[tl(id = 0x4813b4c6, size_hint = 32)]
    Ed25519 { key: [u8; 32] },
    #[tl(id = 0x34ba45cb)]
    Overlay { name: Vec<u8> },
    #[tl(id = 0x2dbcadd4, size_hint = 32)]
    Aes { key: [u8; 32] },
    #[tl(id = 0xb61f450a)]
    Unencoded { data: Vec<u8> },
}

impl AdnlTlPublicKeyOwned {
    #[must_use]
    pub fn as_equivalent_ref(&self) -> AdnlTlPublicKeyRef<'_> {
        match self {
            Self::Ed25519 { key } => AdnlTlPublicKeyRef::Ed25519 { key },
            Self::Overlay { name } => AdnlTlPublicKeyRef::Overlay {
                name: name.as_slice(),
            },
            Self::Aes { key } => AdnlTlPublicKeyRef::Aes { key },
            Self::Unencoded { data } => AdnlTlPublicKeyRef::Unencoded {
                data: data.as_slice(),
            },
        }
    }
}
