//! Query helpers of an ADNL session.
//!
//! Kept as a second inherent block: these methods share every private
//! field of [`super::SessionInner`] but are protocol-level traffic
//! rather than transport, so they would otherwise dominate the
//! transport file.
//!
//! Every helper awaits its answer by query id through the session's
//! pending map ([`SessionInner::exchange`]), so several callers can
//! share one session - which is the whole point of the shared
//! transport - without stealing each other's packets.

use super::*;

use tonutils_tl::tl::network::{
    DhtMessage, DhtNodes, DhtNodesBoxed, DhtStored, DhtValue, DhtValueResult, OverlayNodes,
    OverlayNodesBoxed, OverlayQuery,
};

impl AdnlUdpSession {
    #[allow(clippy::unnecessary_join)]
    pub async fn dht_find_node(
        &self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::FindNode { key, k: count });
        let answer = self
            .inner
            .exchange(query_id, query, timeout, "DHT findNode")
            .await?;
        let nodes: DhtNodesBoxed = tl_proto::deserialize(&answer)
            .map_err(|error| AdnlError::MalformedPacket(error.to_string()))?;
        let now = now_i32();
        let total = nodes.nodes.len();
        let selected: Vec<_> = nodes
            .nodes
            .into_iter()
            .filter(|node| node.is_valid(now))
            .collect();
        log::debug!(
            "dht_find_node: received {total} raw nodes, {} passed is_valid",
            selected.len(),
        );
        Ok(DhtNodesBoxed { nodes: selected })
    }

    pub async fn dht_find_value(
        &self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtValueResult, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::FindValue { key, k: count });
        let answer = self
            .inner
            .exchange(query_id, query, timeout, "DHT findValue")
            .await?;
        // Try to deserialize as DhtValueResult first
        match tl_proto::deserialize::<DhtValueResult>(&answer) {
            Ok(result) => Ok(result),
            Err(value_err) => {
                // Log raw bytes for debugging
                let hex_full = hex::encode(&answer);
                log::debug!(
                    "dht_find_value: DhtValueResult deserialize failed ({}), \
                     answer len={}, hex_full={hex_full}, \
                     trying DhtNodesBoxed fallback",
                    value_err,
                    answer.len()
                );

                // Step-by-step: try to read constructor ID
                if answer.len() >= 4 {
                    let ctor = u32::from_le_bytes(answer[..4].try_into().unwrap());
                    log::debug!(
                        "dht_find_value: step-by-step: constructor=0x{ctor:08x} \
                         (expected Found=0xe40cf774, NotFound=0xa2620568)"
                    );
                    if ctor == 0xe40cf774 && answer.len() >= 8 {
                        let inner_ctor = u32::from_le_bytes(answer[4..8].try_into().unwrap());
                        log::debug!(
                            "dht_find_value: step-by-step: inner constructor=0x{inner_ctor:08x} \
                             (expected DhtValue=0x90ad27cb)"
                        );
                    }
                }

                // Fallback: try DhtNodesBoxed (some nodes respond with
                // dht.Nodes constructor 0x7974a0be instead of DhtValueResult)
                if let Ok(nodes_boxed) = tl_proto::deserialize::<DhtNodesBoxed>(&answer) {
                    log::debug!(
                        "dht_find_value: got DhtNodesBoxed fallback with {} nodes",
                        nodes_boxed.nodes.len()
                    );
                    return Ok(DhtValueResult::NotFound {
                        nodes: DhtNodes {
                            nodes: nodes_boxed.nodes,
                        },
                    });
                }

                // Both failed, return the original error with hex context
                Err(AdnlError::MalformedPacket(format!(
                    "DhtValueResult={value_err}, hex_full={hex_full}"
                )))
            }
        }
    }

    /// Sends `dht.store` over this session.
    ///
    /// The value must be signed by the key owner (see
    /// `DhtValue::unsigned_bytes`); the receiving node verifies
    /// the signature and stores the value when it is responsible
    /// for the key, answering `dht.stored`.  Nodes that are not
    /// responsible drop the value silently, so a store round
    /// targets the nodes closest to the key.
    pub async fn dht_store(&self, value: DhtValue, timeout: Duration) -> Result<(), AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::Store { value });
        let answer = self
            .inner
            .exchange(query_id, query, timeout, "DHT store")
            .await?;
        match tl_proto::deserialize::<DhtStored>(&answer) {
            Ok(DhtStored) => Ok(()),
            Err(error) => Err(AdnlError::MalformedPacket(error.to_string())),
        }
    }

    pub async fn overlay_get_random_peers(
        &self,
        overlay: Int256,
        timeout: Duration,
    ) -> Result<OverlayNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        let rx = self.inner.register_pending(query_id.clone());
        let sent = self
            .send_overlay_get_random_peers_with_id(overlay, query_id.clone())
            .await;
        if let Err(error) = sent {
            self.inner.unregister_pending(&query_id.0);
            return Err(error);
        }
        let answer = self
            .inner
            .await_pending_answer(query_id, timeout, "overlay getRandomPeers", rx)
            .await?;
        tl_proto::deserialize(&answer)
            .map_err(|error| AdnlError::MalformedPacket(error.to_string()))
    }

    /// Sends `overlay.getRandomPeers` as a direct query.
    ///
    /// The first such packet of a session also carries the
    /// `adnl.message.createChannel` handshake, mirroring upstream
    /// `AdnlPeerPairImpl` and pytoniq's `connect_to_peer`: without it a
    /// new session can never decode the channel the peer is already
    /// using.
    ///
    /// The answer to the query is routed by query id to whichever call
    /// registered it (see [`AdnlUdpSession::overlay_get_random_peers`]);
    /// a fire-and-forget send leaves the answer to be consumed by the
    /// session's regular receive path.
    pub async fn send_overlay_get_random_peers(&self, overlay: Int256) -> Result<usize, AdnlError> {
        self.send_overlay_get_random_peers_with_id(overlay, Int256::random())
            .await
    }

    async fn send_overlay_get_random_peers_with_id(
        &self,
        overlay: Int256,
        query_id: Int256,
    ) -> Result<usize, AdnlError> {
        let mut query = tl_proto::serialize(OverlayQuery::Query {
            overlay: overlay.clone(),
        });
        query.extend(tl_proto::serialize(OverlayQuery::GetRandomPeers {
            peers: OverlayNodes {
                nodes: vec![self.local_overlay_node(overlay)],
            },
        }));
        let outgoing = AdnlMessage::Query { query_id, query };
        let (message, messages) = {
            let mut state = self.inner.state.lock().await;
            match self.inner.create_channel_message(&mut state) {
                Some(control) => (None, Some(vec![control, outgoing])),
                None => (Some(outgoing), None),
            }
        };
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message,
            messages,
            address: None,
            priority_address: None,
            seqno: None,
            confirm_seqno: None,
            recv_addr_list_version: None,
            recv_priority_addr_list_version: None,
            reinit_date: None,
            dst_reinit_date: None,
            signature: None,
            rand2: vec![0; 7],
        })
        .await
    }

    /// Returns this session's signed `overlay.node` record for `overlay`.
    ///
    /// The record carries no address: overlay peers resolve the ADNL
    /// address through the DHT `address` value for our node id.  It is
    /// what `overlay.getRandomPeers` answers must contain, see
    /// `adnl.message.query` handling in the overlay session.
    pub fn local_overlay_node(&self, overlay: Int256) -> tonutils_tl::tl::network::OverlayNode {
        let version = now_i32();
        let to_sign = tonutils_tl::tl::network::OverlayNodeToSign {
            id: tonutils_tl::tl::network::AdnlIdShort {
                id: Int256(self.inner.local_id),
            },
            overlay: overlay.clone(),
            version,
        };
        tonutils_tl::tl::network::OverlayNode {
            id: tonutils_tl::tl::network::PublicKey::Ed25519 {
                key: Int256(self.inner.local.public_key.to_bytes()),
            },
            overlay,
            version,
            signature: self
                .inner
                .local
                .sign_raw(&tl_proto::serialize(to_sign))
                .to_vec(),
        }
    }

    /// The `overlay.nodeV2` form of [`Self::local_overlay_node`].
    ///
    /// `flags` is zero, so upstream signs the record with the same
    /// `overlay.node.toSign` definition (`OverlayNode::to_sign` in
    /// `overlay/overlay-id.hpp` picks `overlay_node_toSign` for
    /// `flags_ == 0`), and a peer validating the answer accepts it
    /// exactly like the V1 record.
    pub fn local_overlay_node_v2(
        &self,
        overlay: Int256,
    ) -> tonutils_tl::tl::network::OverlayNodeV2 {
        let ours = self.local_overlay_node(overlay);
        tonutils_tl::tl::network::OverlayNodeV2 {
            id: ours.id,
            overlay: ours.overlay,
            flags: 0,
            version: ours.version,
            signature: ours.signature,
            certificate: tonutils_tl::tl::network::OverlayMemberCertificate::Empty,
        }
    }
}

impl SessionInner {
    /// Sends a plain `adnl.message.query` and awaits its answer.
    ///
    /// The answer is routed by query id to this call, so concurrent
    /// callers of a shared session each get their own answer.
    async fn exchange(
        &self,
        query_id: Int256,
        query: Vec<u8>,
        timeout: Duration,
        operation: &'static str,
    ) -> Result<Vec<u8>, AdnlError> {
        let rx = self.register_pending(query_id.clone());
        let sent = self
            .send_contents(PacketContents {
                rand1: vec![0; 7],
                flags: (),
                from: None,
                from_short: None,
                message: Some(AdnlMessage::Query {
                    query_id: query_id.clone(),
                    query,
                }),
                messages: None,
                address: None,
                priority_address: None,
                seqno: None,
                confirm_seqno: None,
                recv_addr_list_version: None,
                recv_priority_addr_list_version: None,
                reinit_date: None,
                dst_reinit_date: None,
                signature: None,
                rand2: vec![0; 7],
            })
            .await;
        if let Err(error) = sent {
            self.unregister_pending(&query_id.0);
            return Err(error);
        }
        self.await_pending_answer(query_id, timeout, operation, rx)
            .await
    }
}
