use super::*;

use super::*;

impl LiteClient {
    pub async fn connect<A: ToSocketAddrs>(
        address: A,
        public_key: impl AsRef<[u8]>,
    ) -> Result<Self> {
        let adnl = AdnlPeer::connect(public_key, address).await?;
        let lite = LitePeer::new(adnl);
        let service =
            ServiceBuilder::new()
                .layer(WrapRawMessagesLayer)
                .service(multiplex::Client::<
                    _,
                    Box<dyn std::error::Error + Send + Sync + 'static>,
                    _,
                >::new(lite));
        Ok(Self {
            inner: service.boxed(),
            wait_seqno: None,
            rate_limiter: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_service<S>(service: S) -> Self
    where
        S: tower::Service<RawWrappedRequest, Response = Vec<u8>, Error = LiteError>
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        Self {
            inner: service.boxed(),
            wait_seqno: None,
            rate_limiter: None,
        }
    }

    #[cfg(feature = "network-config")]
    pub async fn connect_liteserver(liteserver: &ConfigLiteServer) -> Result<Self> {
        Self::connect(liteserver.socket_addr(), liteserver.public_key()).await
    }

    #[cfg(feature = "network-config")]
    pub async fn connect_config(config: &ConfigGlobal, liteserver_index: usize) -> Result<Self> {
        let liteserver = config
            .liteserver(liteserver_index)
            .map_err(|e| LiteError::TlError(crate::tl::TlError::ParseError(e.to_string())))?;
        Self::connect_liteserver(liteserver).await
    }

    #[cfg(feature = "network-config")]
    pub async fn connect_first(config: &ConfigGlobal) -> Result<Self> {
        let liteserver = config
            .first_liteserver()
            .map_err(|e| LiteError::TlError(crate::tl::TlError::ParseError(e.to_string())))?;
        Self::connect_liteserver(liteserver).await
    }

    pub fn wait_masterchain_seqno(mut self, seqno: u32) -> Self {
        self.wait_seqno = Some(seqno);
        self
    }

    pub fn with_rate_limit(mut self, limit: RequestRateLimit) -> Self {
        self.set_rate_limit(limit);
        self
    }

    pub fn set_rate_limit(&mut self, limit: RequestRateLimit) {
        self.rate_limiter = Some(RateLimiter::new(limit));
    }

    pub fn clear_rate_limit(&mut self) {
        self.rate_limiter = None;
    }

    #[cfg(test)]
    pub(crate) fn has_rate_limiter(&self) -> bool {
        self.rate_limiter.is_some()
    }

    pub(super) async fn send_request<T: FromResponse>(&mut self, request: Request) -> Result<T> {
        let response = self.query_raw(tl_proto::serialize(request)).await?;
        let response: Response = tl_proto::deserialize(&response)
            .map_err(|e| LiteError::TlError(crate::tl::TlError::ParseError(e.to_string())))?;
        match response {
            Response::Error(error) => Err(LiteError::from(error)),
            response => T::from_response(response),
        }
    }

    pub async fn query_typed<T: FromResponse>(&mut self, request: Request) -> Result<T> {
        self.send_request(request).await
    }

    pub async fn query_raw(&mut self, request: impl AsRef<[u8]>) -> Result<Vec<u8>> {
        if let Some(limiter) = &self.rate_limiter {
            limiter.acquire().await;
        }

        self.inner
            .ready()
            .await?
            .call(RawWrappedRequest {
                wait_masterchain_seqno: self.wait_seqno.take().map(|seqno| WaitMasterchainSeqno {
                    seqno,
                    timeout_ms: 10000,
                }),
                request: request.as_ref().to_vec(),
            })
            .await
    }

    pub async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo> {
        let response: MasterchainInfo = self.send_request(Request::GetMasterchainInfo).await?;
        Ok(response)
    }

    pub async fn get_masterchain_info_ext(&mut self, mode: u32) -> Result<MasterchainInfoExt> {
        let request = Request::GetMasterchainInfoExt(GetMasterchainInfoExt { mode });
        let response: MasterchainInfoExt = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn get_time(&mut self) -> Result<u32> {
        let response: CurrentTime = self.send_request(Request::GetTime).await?;
        Ok(response.now)
    }

    pub async fn get_version(&mut self) -> Result<Version> {
        let response: Version = self.send_request(Request::GetVersion).await?;
        Ok(response)
    }

    pub async fn get_block(&mut self, id: BlockIdExt) -> Result<Vec<u8>> {
        let request = Request::GetBlock(GetBlock { id });
        let response: BlockData = self.send_request(request).await?;
        Ok(response.data)
    }

    pub async fn raw_get_block(&mut self, id: BlockIdExt) -> Result<crate::tlb::Block> {
        Ok(self.raw_get_block_data(id).await?.data.block)
    }

    pub async fn raw_get_block_data(&mut self, id: BlockIdExt) -> Result<DecodedBlockData> {
        let request = Request::GetBlock(GetBlock { id });
        let raw: BlockData = self.send_request(request).await?;
        let data = decode_block_boc(&raw.data).map_err(decode_error)?;
        Ok(DecodedBlockData { raw, data })
    }

    pub async fn get_state(&mut self, id: BlockIdExt) -> Result<BlockState> {
        let request = Request::GetState(GetState { id });
        let response: BlockState = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn get_block_header(
        &mut self,
        id: BlockIdExt,
        with_state_update: bool,
        with_value_flow: bool,
        with_extra: bool,
        with_shard_hashes: bool,
        with_prev_blk_signatures: bool,
    ) -> Result<Vec<u8>> {
        let request = Request::GetBlockHeader(GetBlockHeader {
            id,
            mode: (),
            with_state_update: if with_state_update { Some(()) } else { None },
            with_value_flow: if with_value_flow { Some(()) } else { None },
            with_extra: if with_extra { Some(()) } else { None },
            with_shard_hashes: if with_shard_hashes { Some(()) } else { None },
            with_prev_blk_signatures: if with_prev_blk_signatures {
                Some(())
            } else {
                None
            },
        });
        let response: BlockHeader = self.send_request(request).await?;
        Ok(response.header_proof)
    }

    pub async fn raw_get_block_header(
        &mut self,
        id: BlockIdExt,
        with_state_update: bool,
        with_value_flow: bool,
        with_extra: bool,
        with_shard_hashes: bool,
        with_prev_blk_signatures: bool,
    ) -> Result<DecodedBlockHeader> {
        let request = Request::GetBlockHeader(GetBlockHeader {
            id,
            mode: (),
            with_state_update: if with_state_update { Some(()) } else { None },
            with_value_flow: if with_value_flow { Some(()) } else { None },
            with_extra: if with_extra { Some(()) } else { None },
            with_shard_hashes: if with_shard_hashes { Some(()) } else { None },
            with_prev_blk_signatures: if with_prev_blk_signatures {
                Some(())
            } else {
                None
            },
        });
        let raw: BlockHeader = self.send_request(request).await?;
        let header_proof =
            crate::liteclient::boc::DecodedBoc::decode(&raw.header_proof).map_err(decode_error)?;
        Ok(DecodedBlockHeader { raw, header_proof })
    }

    pub async fn send_message(&mut self, body: Vec<u8>) -> Result<u32> {
        let request = Request::SendMessage(SendMessage { body });
        let response: SendMsgStatus = self.send_request(request).await?;
        Ok(response.status)
    }

    pub async fn get_account_state(
        &mut self,
        id: BlockIdExt,
        account: AccountId,
    ) -> Result<AccountState> {
        let request = Request::GetAccountState(GetAccountState { id, account });
        let response: AccountState = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn raw_get_account_state(
        &mut self,
        account: Address,
        block: Option<BlockIdExt>,
    ) -> Result<(
        Option<crate::tlb::Account>,
        Option<crate::tlb::ShardAccount>,
    )> {
        let decoded = self.get_account_state_typed(account, block).await?;
        Ok((decoded.account, decoded.shard_account))
    }

    pub async fn get_account_state_typed(
        &mut self,
        account: Address,
        block: Option<BlockIdExt>,
    ) -> Result<DecodedAccountState> {
        let id = match block {
            Some(block) => block,
            None => self.get_masterchain_info().await?.last,
        };
        let raw = self.get_account_state(id, account.to_account_id()).await?;
        let _ = account;
        DecodedAccountState::from_raw(raw).map_err(decode_error)
    }

    pub async fn get_account_state_simple(&mut self, account: Address) -> Result<SimpleAccount> {
        Ok(self.get_account_state_typed(account, None).await?.simple())
    }

    pub async fn run_smc_method(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        params: Vec<u8>,
    ) -> Result<RunMethodResult> {
        let request = Request::RunSmcMethod(RunSmcMethod {
            mode,
            id: block,
            account: account.to_account_id(),
            method_id,
            params,
        });
        let response: RunMethodResult = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn run_get_method(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult> {
        let params = stack
            .to_boc()
            .map_err(|e| LiteError::TlError(crate::tl::TlError::ParseError(e.to_string())))?;
        self.run_smc_method(mode, block, account, method_id, params)
            .await
    }

    pub async fn run_get_method_typed(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>> {
        let result = self
            .run_get_method(mode, block, account, method_id, stack)
            .await?;
        if result.exit_code != 0 {
            return Err(LiteError::TlError(crate::tl::TlError::ParseError(format!(
                "run get method exited with code {}",
                result.exit_code
            ))));
        }
        let stack = result
            .result
            .as_deref()
            .map(TvmStack::from_boc)
            .transpose()
            .map_err(decode_error)?
            .unwrap_or_else(TvmStack::empty);
        Ok(stack.entries().to_vec())
    }

    pub async fn run_get_method_by_name(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method: &str,
        stack: TvmStack,
    ) -> Result<RunMethodResult> {
        self.run_get_method(
            mode,
            block,
            account,
            crate::utils::method_name_to_id(method),
            stack,
        )
        .await
    }

    pub async fn get_shard_info(
        &mut self,
        block: BlockIdExt,
        workchain: i32,
        shard: u64,
        exact: bool,
    ) -> Result<ShardInfo> {
        let request = Request::GetShardInfo(GetShardInfo {
            id: block,
            workchain,
            shard,
            exact,
        });
        let response: ShardInfo = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn raw_get_shard_info(
        &mut self,
        block: BlockIdExt,
        workchain: i32,
        shard: u64,
        exact: bool,
    ) -> Result<DecodedShardInfo> {
        let raw = self.get_shard_info(block, workchain, shard, exact).await?;
        let shard_proof = decode_optional_boc(&raw.shard_proof).map_err(decode_error)?;
        let shard_descr = crate::liteclient::boc::ShardDescr {
            boc: crate::liteclient::boc::DecodedBoc::decode(&raw.shard_descr)
                .map_err(decode_error)?,
        };
        Ok(DecodedShardInfo {
            raw,
            shard_proof,
            shard_descr,
        })
    }

    pub async fn get_all_shards_info(&mut self, block: BlockIdExt) -> Result<AllShardsInfo> {
        let request = Request::GetAllShardsInfo(GetAllShardsInfo { id: block });
        let response: AllShardsInfo = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn raw_get_all_shards_info(
        &mut self,
        block: BlockIdExt,
    ) -> Result<DecodedAllShardsInfo> {
        let raw = self.get_all_shards_info(block).await?;
        let proof = decode_optional_boc(&raw.proof).map_err(decode_error)?;
        let data = crate::liteclient::boc::DecodedBoc::decode(&raw.data).map_err(decode_error)?;
        Ok(DecodedAllShardsInfo { raw, proof, data })
    }

    pub async fn get_all_shards_info_typed(
        &mut self,
        block: BlockIdExt,
    ) -> Result<Vec<BlockIdExt>> {
        let _ = self.raw_get_all_shards_info(block).await?;
        Err(LiteError::TlError(crate::tl::TlError::ParseError(
            "typed all-shards dictionary decode requires full ShardDescr/BinTree TL-B models"
                .to_string(),
        )))
    }

    pub async fn get_one_transaction(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
        lt: u64,
    ) -> Result<TransactionInfo> {
        let request = Request::GetOneTransaction(GetOneTransaction {
            id: block,
            account,
            lt,
        });
        let response: TransactionInfo = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn get_one_transaction_typed(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
        lt: u64,
    ) -> Result<Option<crate::tlb::Transaction>> {
        Ok(self
            .get_one_transaction_decoded(block, account, lt)
            .await?
            .transaction)
    }

    pub async fn get_one_transaction_decoded(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
        lt: u64,
    ) -> Result<DecodedTransactionInfo> {
        let raw = self.get_one_transaction(block, account, lt).await?;
        let proof = decode_optional_boc(&raw.proof).map_err(decode_error)?;
        let transaction = if raw.transaction.is_empty() {
            None
        } else {
            Some(
                crate::liteclient::boc::decode_transaction_boc(&raw.transaction)
                    .map_err(decode_error)?,
            )
        };
        Ok(DecodedTransactionInfo {
            raw,
            proof,
            transaction,
        })
    }

    pub async fn get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<TransactionList> {
        let request = Request::GetTransactions(GetTransactions {
            count,
            account,
            lt,
            hash,
        });
        let response: TransactionList = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn raw_get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<(Vec<crate::tlb::Transaction>, Vec<BlockIdExt>)> {
        let raw = self.get_transactions(count, account, lt, hash).await?;
        let transactions =
            decode_single_transaction_list(&raw.transactions).map_err(decode_error)?;
        Ok((transactions, raw.ids))
    }

    pub async fn lookup_block(
        &mut self,
        mode: (),
        id: BlockId,
        seqno: Option<()>,
        lt: Option<u64>,
        utime: Option<u32>,
        with_state_update: bool,
        with_value_flow: bool,
        with_extra: bool,
        with_shard_hashes: bool,
        with_prev_blk_signatures: bool,
    ) -> Result<BlockHeader> {
        let request = Request::LookupBlock(LookupBlock {
            mode,
            id,
            seqno,
            lt,
            utime,
            with_state_update: if with_state_update { Some(()) } else { None },
            with_value_flow: if with_value_flow { Some(()) } else { None },
            with_extra: if with_extra { Some(()) } else { None },
            with_shard_hashes: if with_shard_hashes { Some(()) } else { None },
            with_prev_blk_signatures: if with_prev_blk_signatures {
                Some(())
            } else {
                None
            },
        });
        let response: BlockHeader = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn lookup_block_with_proof(
        &mut self,
        mode: (),
        id: BlockId,
        mc_block_id: BlockIdExt,
        lt: Option<u64>,
        utime: Option<u32>,
    ) -> Result<LookupBlockResult> {
        let request = Request::LookupBlockWithProof(LookupBlockWithProof {
            mode,
            id,
            mc_block_id,
            lt,
            utime,
        });
        let response: LookupBlockResult = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn list_block_transactions(
        &mut self,
        id: BlockIdExt,
        count: u32,
        after: Option<TransactionId3>,
        reverse_order: bool,
        want_proof: bool,
    ) -> Result<BlockTransactions> {
        let request = Request::ListBlockTransactions(ListBlockTransactions {
            id,
            mode: (),
            count,
            after,
            reverse_order: if reverse_order { Some(()) } else { None },
            want_proof: if want_proof { Some(()) } else { None },
        });
        let response: BlockTransactions = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn list_block_transactions_ext(
        &mut self,
        id: BlockIdExt,
        count: u32,
        after: Option<TransactionId3>,
        reverse_order: bool,
        want_proof: bool,
    ) -> Result<BlockTransactionsExt> {
        let request = Request::ListBlockTransactionsExt(ListBlockTransactions {
            id,
            mode: (),
            count,
            after,
            reverse_order: if reverse_order { Some(()) } else { None },
            want_proof: if want_proof { Some(()) } else { None },
        });
        let response: BlockTransactionsExt = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn raw_get_block_transactions_ext(
        &mut self,
        id: BlockIdExt,
        count: u32,
        after: Option<TransactionId3>,
        reverse_order: bool,
        want_proof: bool,
    ) -> Result<Vec<crate::tlb::Transaction>> {
        Ok(self
            .list_block_transactions_ext_decoded(id, count, after, reverse_order, want_proof)
            .await?
            .transactions)
    }

    pub async fn list_block_transactions_ext_decoded(
        &mut self,
        id: BlockIdExt,
        count: u32,
        after: Option<TransactionId3>,
        reverse_order: bool,
        want_proof: bool,
    ) -> Result<DecodedBlockTransactionsExt> {
        let raw = self
            .list_block_transactions_ext(id, count, after, reverse_order, want_proof)
            .await?;
        let transactions =
            decode_single_transaction_list(&raw.transactions).map_err(decode_error)?;
        let proof = decode_optional_boc(&raw.proof).map_err(decode_error)?;
        Ok(DecodedBlockTransactionsExt {
            raw,
            transactions,
            proof,
        })
    }

    pub async fn get_block_proof(
        &mut self,
        known_block: BlockIdExt,
        target_block: Option<BlockIdExt>,
        allow_weak_target: bool,
        base_block_from_request: bool,
    ) -> Result<PartialBlockProof> {
        let request = Request::GetBlockProof(GetBlockProof {
            mode: (),
            known_block,
            target_block,
            allow_weak_target: if allow_weak_target { Some(()) } else { None },
            base_block_from_request: if base_block_from_request {
                Some(())
            } else {
                None
            },
        });
        let response: PartialBlockProof = self.send_request(request).await?;
        Ok(response)
    }

    pub async fn get_config_all(
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
    ) -> Result<ConfigInfo> {
        let request = Request::GetConfigAll(GetConfigAll {
            mode: (),
            id,
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
}
