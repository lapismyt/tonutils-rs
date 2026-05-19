use super::*;

use crate::adnl::AdnlPeer;
use tokio::net::ToSocketAddrs;
use tokio_tower::multiplex;
use tower::{Service as _, ServiceBuilder, ServiceExt as _};

use crate::liteclient::{
    boc::{
        DecodedAccountState, DecodedAllShardsInfo, DecodedBlockData, DecodedBlockHeader,
        DecodedBlockTransactionsExt, DecodedConfigInfo, DecodedLibrariesWithProof,
        DecodedShardInfo, DecodedTransactionInfo, SimpleAccount, decode_block_boc,
        decode_optional_boc, decode_optional_config, decode_single_transaction_list,
    },
    layers::WrapRawMessagesLayer,
    peer::LitePeer,
    rate_limit::{RateLimiter, RequestRateLimit},
    types::LiteError,
};
#[cfg(feature = "network-config")]
use crate::network_config::{ConfigGlobal, ConfigLiteServer};
use crate::tl::{common::*, request::*, response::*, utils::FromResponse};
use crate::tvm::{TvmStack, TvmStackEntry, address::Address, deserialize_boc};
use std::{collections::HashMap, sync::Arc};

pub struct LiteClient {
    pub(super) inner: tower::util::BoxService<RawWrappedRequest, Vec<u8>, LiteError>,
    pub(super) wait_seqno: Option<u32>,
    pub(super) rate_limiter: Option<RateLimiter>,
    pub(super) request_timeout: Option<std::time::Duration>,
}
