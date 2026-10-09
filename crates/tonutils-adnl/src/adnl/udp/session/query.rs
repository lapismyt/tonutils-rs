//! Query helpers of an ADNL session.
//!
//! Kept as a second inherent block: these methods share every private field of
//! [`super::AdnlUdpSession`] but are protocol-level traffic rather than
//! transport, so they would otherwise dominate the transport file.

use super::*;

use tonutils_tl::tl::network::{
    DhtMessage, DhtNodes, DhtNodesBoxed, DhtValueResult, OverlayNodes, OverlayNodesBoxed,
    OverlayQuery,
};

impl AdnlUdpSession {
    #[allow(clippy::unnecessary_join)]
    pub async fn dht_find_node(
        &mut self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::FindNode { key, k: count });
        self.send_contents(PacketContents {
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
        .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "DHT findNode",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
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
                    return Ok(DhtNodesBoxed { nodes: selected });
                }
            }
        }
    }

    pub async fn dht_find_value(
        &mut self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtValueResult, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::FindValue { key, k: count });
        self.send_contents(PacketContents {
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
        .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "DHT findValue",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
                    // Try to deserialize as DhtValueResult first
                    match tl_proto::deserialize::<DhtValueResult>(&answer) {
                        Ok(result) => return Ok(result),
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
                                    let inner_ctor =
                                        u32::from_le_bytes(answer[4..8].try_into().unwrap());
                                    log::debug!(
                                        "dht_find_value: step-by-step: inner constructor=0x{inner_ctor:08x} \
                                         (expected DhtValue=0x90ad27cb)"
                                    );
                                }
                            }

                            // Fallback: try DhtNodesBoxed (some nodes respond with
                            // dht.Nodes constructor 0x7974a0be instead of DhtValueResult)
                            if let Ok(nodes_boxed) = tl_proto::deserialize::<DhtNodesBoxed>(&answer)
                            {
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
                            return Err(AdnlError::MalformedPacket(format!(
                                "DhtValueResult={value_err}, hex_full={hex_full}"
                            )));
                        }
                    }
                }
            }
        }
    }

    pub async fn overlay_get_random_peers(
        &mut self,
        overlay: Int256,
        timeout: Duration,
    ) -> Result<OverlayNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        self.send_overlay_get_random_peers_with_id(overlay, query_id.clone())
            .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "overlay getRandomPeers",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
                    return tl_proto::deserialize(&answer)
                        .map_err(|error| AdnlError::MalformedPacket(error.to_string()));
                }
            }
        }
    }

    /// Sends `overlay.getRandomPeers` as a direct query.
    ///
    /// The first such packet of a session also carries the
    /// `adnl.message.createChannel` handshake, mirroring upstream
    /// `AdnlPeerPairImpl` and pytoniq's `connect_to_peer`: without it a new
    /// session can never decode the channel the peer is already using.
    pub async fn send_overlay_get_random_peers(
        &mut self,
        overlay: Int256,
    ) -> Result<usize, AdnlError> {
        self.send_overlay_get_random_peers_with_id(overlay, Int256::random())
            .await
    }

    async fn send_overlay_get_random_peers_with_id(
        &mut self,
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
        let (message, messages) = match self.create_channel_message() {
            Some(control) => (None, Some(vec![control, outgoing])),
            None => (Some(outgoing), None),
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
    /// The record carries no address: overlay peers resolve the ADNL address
    /// through the DHT `address` value for our node id.  It is what
    /// `overlay.getRandomPeers` answers must contain, see
    /// `adnl.message.query` handling in the overlay session.
    pub fn local_overlay_node(&self, overlay: Int256) -> tonutils_tl::tl::network::OverlayNode {
        let version = now_i32();
        let to_sign = tonutils_tl::tl::network::OverlayNodeToSign {
            id: tonutils_tl::tl::network::AdnlIdShort {
                id: Int256(self.local_id),
            },
            overlay: overlay.clone(),
            version,
        };
        tonutils_tl::tl::network::OverlayNode {
            id: tonutils_tl::tl::network::PublicKey::Ed25519 {
                key: Int256(self.local.public_key.to_bytes()),
            },
            overlay,
            version,
            signature: self.local.sign_raw(&tl_proto::serialize(to_sign)).to_vec(),
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
