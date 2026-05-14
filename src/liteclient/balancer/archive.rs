use super::*;

impl LiteBalancer {
    pub(super) async fn check_archive(client: &mut LiteClient) -> bool {
        // Try to lookup an old block to check if peer is archival
        let block_id = BlockId {
            workchain: -1,
            shard: -9223372036854775808i64,
            seqno: Self::archive_probe_seqno(),
        };

        match client
            .lookup_block(
                (),
                block_id,
                Some(()),
                None,
                None,
                false,
                false,
                false,
                false,
                false,
            )
            .await
        {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    pub(super) fn archive_probe_seqno() -> i32 {
        (rand::random::<u32>() % 1024 + 1) as i32
    }

    pub(super) async fn find_archives(&mut self) {
        let alive_peers: Vec<usize> = self.alive_peers.read().await.iter().copied().collect();
        let mut archival = HashSet::new();

        for i in alive_peers {
            if let Some(client) = self.peers.get_mut(i) {
                if Self::check_archive(client).await {
                    archival.insert(i);
                }
            }
        }

        *self.archival_peers.write().await = archival;
    }
}
