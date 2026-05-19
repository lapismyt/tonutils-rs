use super::*;

impl LiteClient {
    #[allow(clippy::too_many_arguments)]
    pub async fn get_config_all_typed(
        &mut self,
        id: BlockIdExt,
        with_state_root: bool,
        with_libraries: bool,
        with_state_extra_root: bool,
        with_shard_hashes: bool,
        with_validator_set: bool,
        with_special_smc: bool,
        with_accounts_root: bool,
        with_prev_blocks: bool,
        with_workchain_info: bool,
        with_capabilities: bool,
        extract_from_key_block: bool,
    ) -> Result<DecodedConfigInfo> {
        let raw = self
            .get_config_all(
                id,
                with_state_root,
                with_libraries,
                with_state_extra_root,
                with_shard_hashes,
                with_validator_set,
                with_special_smc,
                with_accounts_root,
                with_prev_blocks,
                with_workchain_info,
                with_capabilities,
                extract_from_key_block,
            )
            .await?;
        decode_config_info(raw)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_config_params(
        &mut self,
        id: BlockIdExt,
        param_list: Vec<i32>,
        with_state_root: bool,
        with_libraries: bool,
        with_state_extra_root: bool,
        with_shard_hashes: bool,
        with_validator_set: bool,
        with_special_smc: bool,
        with_accounts_root: bool,
        with_prev_blocks: bool,
        with_workchain_info: bool,
        with_capabilities: bool,
        extract_from_key_block: bool,
    ) -> Result<ConfigInfo> {
        let request = Request::GetConfigParams(GetConfigParams {
            mode: (),
            id,
            param_list,
            with_state_root: if with_state_root { Some(()) } else { None },
            with_libraries: if with_libraries { Some(()) } else { None },
            with_state_extra_root: if with_state_extra_root {
                Some(())
            } else {
                None
            },
            with_shard_hashes: if with_shard_hashes { Some(()) } else { None },
            with_validator_set: if with_validator_set { Some(()) } else { None },
            with_special_smc: if with_special_smc { Some(()) } else { None },
            with_accounts_root: if with_accounts_root { Some(()) } else { None },
            with_prev_blocks: if with_prev_blocks { Some(()) } else { None },
            with_workchain_info: if with_workchain_info { Some(()) } else { None },
            with_capabilities: if with_capabilities { Some(()) } else { None },
            extract_from_key_block: if extract_from_key_block {
                Some(())
            } else {
                None
            },
        });
        let response: ConfigInfo = self.send_request(request).await?;
        Ok(response)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_config_params_typed(
        &mut self,
        id: BlockIdExt,
        param_list: Vec<i32>,
        with_state_root: bool,
        with_libraries: bool,
        with_state_extra_root: bool,
        with_shard_hashes: bool,
        with_validator_set: bool,
        with_special_smc: bool,
        with_accounts_root: bool,
        with_prev_blocks: bool,
        with_workchain_info: bool,
        with_capabilities: bool,
        extract_from_key_block: bool,
    ) -> Result<DecodedConfigInfo> {
        let raw = self
            .get_config_params(
                id,
                param_list,
                with_state_root,
                with_libraries,
                with_state_extra_root,
                with_shard_hashes,
                with_validator_set,
                with_special_smc,
                with_accounts_root,
                with_prev_blocks,
                with_workchain_info,
                with_capabilities,
                extract_from_key_block,
            )
            .await?;
        decode_config_info(raw)
    }

    pub async fn get_validator_stats(
        &mut self,
        id: BlockIdExt,
        limit: u32,
        start_after: Option<Int256>,
        modified_after: Option<u32>,
    ) -> Result<ValidatorStats> {
        let request = Request::GetValidatorStats(GetValidatorStats {
            mode: (),
            id,
            limit,
            start_after,
            modified_after,
        });
        let response: ValidatorStats = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn get_libraries(&mut self, library_list: Vec<Int256>) -> Result<Vec<LibraryEntry>> {
        let request = Request::GetLibraries(GetLibraries { library_list });
        let response: LibraryResult = self.send_request(request).await?;
        Ok(response.result)
    }

    pub async fn get_libraries_typed(
        &mut self,
        library_list: Vec<Int256>,
    ) -> Result<HashMap<Int256, Option<Arc<crate::tvm::Cell>>>> {
        let entries = self.get_libraries(library_list).await?;
        let mut libraries = HashMap::with_capacity(entries.len());
        for entry in entries {
            let cell = if entry.data.is_empty() {
                None
            } else {
                Some(deserialize_boc(&entry.data).map_err(decode_error)?)
            };
            libraries.insert(entry.hash, cell);
        }
        Ok(libraries)
    }

    pub async fn get_libraries_with_proof(
        &mut self,
        id: BlockIdExt,
        mode: u32,
        library_list: Vec<Int256>,
    ) -> Result<LibraryResultWithProof> {
        let request = Request::GetLibrariesWithProof(GetLibrariesWithProof {
            id,
            mode: (),
            library_list,
        });
        let _ = mode;
        self.send_request(request).await
    }

    pub async fn get_libraries_with_proof_typed(
        &mut self,
        id: BlockIdExt,
        mode: u32,
        library_list: Vec<Int256>,
    ) -> Result<DecodedLibrariesWithProof> {
        let raw = self
            .get_libraries_with_proof(id, mode, library_list)
            .await?;
        let mut libraries = HashMap::with_capacity(raw.result.len());
        for entry in &raw.result {
            let cell = if entry.data.is_empty() {
                None
            } else {
                Some(deserialize_boc(&entry.data).map_err(decode_error)?)
            };
            libraries.insert(entry.hash.clone(), cell);
        }
        let state_proof = decode_optional_boc(&raw.state_proof).map_err(decode_error)?;
        let data_proof = decode_optional_boc(&raw.data_proof).map_err(decode_error)?;
        Ok(DecodedLibrariesWithProof {
            raw,
            libraries,
            state_proof,
            data_proof,
        })
    }

    pub async fn get_shard_block_proof(&mut self, id: BlockIdExt) -> Result<ShardBlockProof> {
        self.send_request(Request::GetShardBlockProof(GetShardBlockProof { id }))
            .await
    }

    pub async fn get_out_msg_queue_sizes(
        &mut self,
        shard_id: Option<(i32, u64)>,
    ) -> Result<OutMsgQueueSizes> {
        let (wc, shard) = match shard_id {
            Some((wc, shard)) => (Some(wc), Some(shard)),
            None => (None, None),
        };
        self.send_request(Request::GetOutMsgQueueSizes(GetOutMsgQueueSizes {
            mode: (),
            wc,
            shard,
        }))
        .await
    }

    pub async fn get_block_out_msg_queue_size(
        &mut self,
        id: BlockIdExt,
        want_proof: bool,
    ) -> Result<BlockOutMsgQueueSize> {
        self.send_request(Request::GetBlockOutMsgQueueSize(GetBlockOutMsgQueueSize {
            mode: (),
            id,
            want_proof: if want_proof { Some(()) } else { None },
        }))
        .await
    }

    pub async fn get_dispatch_queue_info(
        &mut self,
        id: BlockIdExt,
        after_addr: Option<Int256>,
        max_accounts: u32,
        want_proof: bool,
    ) -> Result<DispatchQueueInfo> {
        self.send_request(Request::GetDispatchQueueInfo(GetDispatchQueueInfo {
            mode: (),
            id,
            want_proof: if want_proof { Some(()) } else { None },
            after_addr,
            max_accounts,
        }))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_dispatch_queue_messages(
        &mut self,
        id: BlockIdExt,
        addr: Int256,
        after_lt: u64,
        max_messages: u32,
        want_proof: bool,
        one_account: bool,
        message_boc: bool,
    ) -> Result<DispatchQueueMessages> {
        self.send_request(Request::GetDispatchQueueMessages(
            GetDispatchQueueMessages {
                mode: (),
                id,
                addr,
                after_lt,
                max_messages,
                want_proof: if want_proof { Some(()) } else { None },
                one_account: if one_account { Some(()) } else { None },
                message_boc: if message_boc { Some(()) } else { None },
            },
        ))
        .await
    }

    pub async fn get_nonfinal_validator_groups(
        &mut self,
        shard_id: Option<(i32, u64)>,
    ) -> Result<NonfinalValidatorGroups> {
        let (wc, shard) = match shard_id {
            Some((wc, shard)) => (Some(wc), Some(shard)),
            None => (None, None),
        };
        self.send_request(Request::NonfinalGetValidatorGroups(
            NonfinalGetValidatorGroups {
                mode: (),
                wc,
                shard,
            },
        ))
        .await
    }

    pub async fn get_nonfinal_candidate(
        &mut self,
        id: NonfinalCandidateId,
    ) -> Result<NonfinalCandidate> {
        self.send_request(Request::NonfinalGetCandidate(NonfinalGetCandidate { id }))
            .await
    }

    pub async fn get_nonfinal_pending_shard_blocks(
        &mut self,
        shard_id: Option<(i32, u64)>,
    ) -> Result<NonfinalPendingShardBlocks> {
        let (wc, shard) = match shard_id {
            Some((wc, shard)) => (Some(wc), Some(shard)),
            None => (None, None),
        };
        self.send_request(Request::NonfinalGetPendingShardBlocks(
            NonfinalGetPendingShardBlocks {
                mode: (),
                wc,
                shard,
            },
        ))
        .await
    }
}
