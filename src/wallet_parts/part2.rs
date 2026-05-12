impl WalletMessage {
    /// Creates a simple internal transfer action.
    pub fn internal(destination: Address, value: u64) -> Self {
        Self {
            mode: 3,
            destination,
            value,
            body: None,
            bounce: true,
        }
    }

    /// Sets the send mode.
    pub fn with_mode(mut self, mode: u8) -> Self {
        self.mode = mode;
        self
    }

    /// Sets an inline body cell.
    pub fn with_body(mut self, body: Arc<Cell>) -> Self {
        self.body = Some(body);
        self
    }

    /// Sets the bounce flag.
    pub fn with_bounce(mut self, bounce: bool) -> Self {
        self.bounce = bounce;
        self
    }

    fn into_message_relaxed(self) -> MessageRelaxed {
        let body = self
            .body
            .unwrap_or_else(|| Builder::new().build().expect("empty cell builds"));
        MessageRelaxed {
            info: CommonMsgInfoRelaxed::Internal {
                ihr_disabled: true,
                bounce: self.bounce,
                bounced: false,
                src: MsgAddress::Ext(MsgAddressExt::None),
                dest: MsgAddressInt::std(self.destination),
                value: CurrencyCollection::grams(Grams(BigUint::from(self.value))),
                extra_flags: BigUint::from(0u8),
                fwd_fee: Grams(BigUint::from(0u8)),
                created_lt: 0,
                created_at: 0,
            },
            init: None,
            body: Either::Left(body),
        }
    }

    fn into_action(self) -> OutAction {
        OutAction::SendMsg {
            mode: self.mode,
            out_msg: self.into_message_relaxed(),
        }
    }
}

/// Wallet V5R1 management action from `W5ExtendedAction`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalletV5R1ExtendedAction {
    /// `add_extension#02 addr:MsgAddressInt`.
    AddExtension {
        /// Extension contract address to authorize.
        address: MsgAddressInt,
    },
    /// `delete_extension#03 addr:MsgAddressInt`.
    DeleteExtension {
        /// Extension contract address to remove.
        address: MsgAddressInt,
    },
    /// `set_signature_auth_allowed#04 allowed:Bool`.
    SetSignatureAuthAllowed {
        /// Whether public-key signature authentication remains allowed.
        allowed: bool,
    },
}

impl WalletV5R1ExtendedAction {
    /// Creates an add-extension action from a standard address.
    pub fn add_extension(address: Address) -> Self {
        Self::AddExtension {
            address: MsgAddressInt::std(address),
        }
    }

    /// Creates a delete-extension action from a standard address.
    pub fn delete_extension(address: Address) -> Self {
        Self::DeleteExtension {
            address: MsgAddressInt::std(address),
        }
    }

    /// Creates a signature-auth policy action.
    pub fn set_signature_auth_allowed(allowed: bool) -> Self {
        Self::SetSignatureAuthAllowed { allowed }
    }
}

impl TlbSerialize for WalletV5R1ExtendedAction {
    fn store_tlb(&self, builder: &mut Builder) -> crate::tlb::Result<()> {
        match self {
            Self::AddExtension { address } => {
                builder.store_u8(WALLET_V5R1_ADD_EXTENSION_TAG)?;
                address.store_tlb(builder)?;
            }
            Self::DeleteExtension { address } => {
                builder.store_u8(WALLET_V5R1_DELETE_EXTENSION_TAG)?;
                address.store_tlb(builder)?;
            }
            Self::SetSignatureAuthAllowed { allowed } => {
                builder.store_u8(WALLET_V5R1_SET_SIGNATURE_AUTH_ALLOWED_TAG)?;
                builder.store_bit(*allowed)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for WalletV5R1ExtendedAction {
    fn load_tlb(slice: &mut Slice) -> crate::tlb::Result<Self> {
        match slice.load_u8()? {
            WALLET_V5R1_ADD_EXTENSION_TAG => Ok(Self::AddExtension {
                address: MsgAddressInt::load_tlb(slice)?,
            }),
            WALLET_V5R1_DELETE_EXTENSION_TAG => Ok(Self::DeleteExtension {
                address: MsgAddressInt::load_tlb(slice)?,
            }),
            WALLET_V5R1_SET_SIGNATURE_AUTH_ALLOWED_TAG => Ok(Self::SetSignatureAuthAllowed {
                allowed: slice.load_bit()?,
            }),
            tag => Err(TlbError::CustomSchema {
                schema: "WalletV5R1ExtendedAction",
                message: format!("unknown extended action tag 0x{tag:02x}"),
            }),
        }
    }
}

/// Wallet V5R1 non-empty `W5ExtendedActionList`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV5R1ExtendedActionList {
    /// Extended actions in serialization order.
    pub actions: Vec<WalletV5R1ExtendedAction>,
}

impl WalletV5R1ExtendedActionList {
    /// Creates a non-empty extended action list.
    pub fn new(actions: Vec<WalletV5R1ExtendedAction>) -> crate::tlb::Result<Self> {
        if actions.is_empty() {
            return Err(TlbError::CustomSchema {
                schema: "W5ExtendedActionList",
                message: "extended action list cannot be empty".to_string(),
            });
        }
        if actions.len() > WALLET_V5R1_MAX_ACTIONS {
            return Err(TlbError::CustomSchema {
                schema: "W5ExtendedActionList",
                message: format!(
                    "action count {} exceeds maximum {WALLET_V5R1_MAX_ACTIONS}",
                    actions.len()
                ),
            });
        }
        Ok(Self { actions })
    }

    /// Returns the number of extended actions.
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Returns false because `W5ExtendedActionList` has no empty constructor.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    fn store_actions(
        actions: &[WalletV5R1ExtendedAction],
        builder: &mut Builder,
    ) -> crate::tlb::Result<()> {
        let Some((first, rest)) = actions.split_first() else {
            return Err(TlbError::CustomSchema {
                schema: "W5ExtendedActionList",
                message: "extended action list cannot be empty".to_string(),
            });
        };
        first.store_tlb(builder)?;
        if !rest.is_empty() {
            let mut next = Builder::new();
            Self::store_actions(rest, &mut next)?;
            builder.store_ref(next.build()?)?;
        }
        Ok(())
    }

    fn load_actions(
        slice: &mut Slice,
        depth: usize,
    ) -> crate::tlb::Result<Vec<WalletV5R1ExtendedAction>> {
        if depth >= WALLET_V5R1_MAX_ACTIONS {
            return Err(TlbError::CustomSchema {
                schema: "W5ExtendedActionList",
                message: format!("action count exceeds maximum {WALLET_V5R1_MAX_ACTIONS}"),
            });
        }

        let action = WalletV5R1ExtendedAction::load_tlb(slice)?;
        let mut actions = vec![action];
        match slice.remaining_refs() {
            0 => {}
            1 => {
                let next = slice.load_reference()?;
                let mut next_slice = Slice::new(next);
                actions.extend(Self::load_actions(&mut next_slice, depth + 1).map_err(
                    |source| TlbError::InvalidReferencePayload {
                        schema: "W5ExtendedActionList",
                        source: Box::new(source),
                    },
                )?);
                ensure_empty(&next_slice).map_err(|source| TlbError::InvalidReferencePayload {
                    schema: "W5ExtendedActionList",
                    source: Box::new(source),
                })?;
            }
            count => {
                return Err(TlbError::CustomSchema {
                    schema: "W5ExtendedActionList",
                    message: format!(
                        "list node has {count} continuation references, expected at most 1"
                    ),
                });
            }
        }
        Ok(actions)
    }
}

impl TlbSerialize for WalletV5R1ExtendedActionList {
    fn store_tlb(&self, builder: &mut Builder) -> crate::tlb::Result<()> {
        Self::store_actions(&self.actions, builder)
    }
}

impl TlbDeserialize for WalletV5R1ExtendedActionList {
    fn load_tlb(slice: &mut Slice) -> crate::tlb::Result<Self> {
        Self::new(Self::load_actions(slice, 0)?)
    }
}

/// Wallet V4R2 offline helper bound to code, workchain, wallet id, and public key.
#[derive(Debug, Clone)]
pub struct WalletV4R2 {
    workchain: i8,
    wallet_id: u32,
    public_key: [u8; 32],
    code: Arc<Cell>,
}

impl WalletV4R2 {
    /// Creates a Wallet V4R2 helper from a public key, raw wallet id, code cell,
    /// and workchain.
    pub fn new(public_key: [u8; 32], wallet_id: u32, code: Arc<Cell>, workchain: i8) -> Self {
        Self {
            workchain,
            wallet_id,
            public_key,
            code,
        }
    }

    /// Creates a Wallet V4R2 helper with the common default wallet id.
    pub fn default(public_key: [u8; 32], code: Arc<Cell>, workchain: i8) -> Self {
        Self::new(public_key, WALLET_V4R2_DEFAULT_ID, code, workchain)
    }

    /// Returns the wallet workchain.
    pub fn workchain(&self) -> i8 {
        self.workchain
    }

    /// Returns the raw 32-bit wallet id.
    pub fn wallet_id(&self) -> u32 {
        self.wallet_id
    }

    /// Returns the configured public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// Builds the initial data cell value.
    pub fn data(&self) -> WalletV4R2Data {
        WalletV4R2Data::new(self.wallet_id, self.public_key)
    }

    /// Builds the wallet `StateInit`.
    pub fn state_init(&self) -> Result<StateInit, WalletError> {
        Ok(StateInit {
            code: Some(self.code.clone()),
            data: Some(self.data().to_cell()?),
            ..StateInit::empty()
        })
    }

    /// Derives the wallet address from `StateInit`.
    pub fn address(&self) -> Result<Address, WalletError> {
        let state_init = self.state_init()?;
        Ok(Address::new(self.workchain, state_init.to_cell()?.hash()))
    }

    /// Builds the unsigned signing cell for a Wallet V4R2 simple send.
    pub fn build_external_signing_cell(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
    ) -> Result<Arc<Cell>, WalletError> {
        if messages.len() > WALLET_V4R2_MAX_MESSAGES {
            return Err(WalletError::TooManyActions {
                count: messages.len(),
                max: WALLET_V4R2_MAX_MESSAGES,
            });
        }

        let mut builder = Builder::new();
        builder.store_u32(self.wallet_id)?;
        builder.store_u32(valid_until)?;
        builder.store_u32(seqno)?;
        builder.store_u32(WALLET_V4R2_SIMPLE_SEND_OP)?;
        for message in messages {
            let mode = message.mode;
            let relaxed = message.into_message_relaxed();
            builder.store_u8(mode)?;
            builder.store_ref(relaxed.to_cell()?)?;
        }
        Ok(builder.build()?)
    }

    /// Builds a signed external body cell and returns the body, signed hash,
    /// and Ed25519 signature.
    pub fn build_signed_external_body(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
    ) -> Result<WalletV4R2SignedBody, WalletError> {
        let signing_cell = self.build_external_signing_cell(seqno, valid_until, messages)?;
        let signing_hash = signing_cell.hash();
        let signature = signing_key.sign(&signing_hash).to_bytes();

        let mut builder = Builder::new();
        builder.store_bytes(&signature)?;
        builder.store_cell(&signing_cell)?;
        Ok(WalletV4R2SignedBody {
            body: builder.build()?,
            signing_hash,
            signature,
        })
    }

    /// Builds an external inbound message BoC with the signed body.
    pub fn build_external_message_boc(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<Vec<u8>, WalletError> {
        let signed = self.build_signed_external_body(seqno, valid_until, messages, signing_key)?;
        let state_init = if include_state_init {
            Some(Either::Right(self.state_init()?))
        } else {
            None
        };
        let message = Message {
            info: CommonMsgInfo::ExternalIn {
                src: MsgAddressExt::None,
                dest: MsgAddressInt::std(self.address()?),
                import_fee: Grams(BigUint::from(0u8)),
            },
            init: state_init,
            body: Either::Right(signed.body),
        };
        Ok(serialize_boc(&message.to_cell()?, false)?)
    }

    /// Sends a signed external message BoC through any contract provider.
    #[cfg(feature = "liteclient")]
    pub async fn send_external_message<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<u32, WalletSendError<P::Error>> {
        let boc = self
            .build_external_message_boc(
                seqno,
                valid_until,
                messages,
                signing_key,
                include_state_init,
            )
            .map_err(WalletSendError::Build)?;
        provider
            .send_external_message_boc(boc)
            .await
            .map_err(WalletSendError::Provider)
    }
}

/// Signed Wallet V4R2 external body material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV4R2SignedBody {
    /// Final body cell containing signature followed by signing fields.
    pub body: Arc<Cell>,
    /// Representation hash that was signed.
    pub signing_hash: [u8; 32],
    /// Ed25519 signature bytes.
    pub signature: [u8; 64],
}

/// Wallet V5R1 offline helper bound to code, workchain, wallet id, and public key.
#[derive(Debug, Clone)]
pub struct WalletV5R1 {
    workchain: i8,
    wallet_id: u32,
    public_key: [u8; 32],
    code: Arc<Cell>,
    is_signature_allowed: bool,
}

impl WalletV5R1 {
    /// Creates a Wallet V5R1 helper from a public key, raw wallet id, code cell,
    /// and workchain.
    pub fn new(public_key: [u8; 32], wallet_id: u32, code: Arc<Cell>, workchain: i8) -> Self {
        Self {
            workchain,
            wallet_id,
            public_key,
            code,
            is_signature_allowed: true,
        }
    }

    /// Creates a Wallet V5R1 helper from a packed wallet-id helper.
    pub fn from_wallet_id(
        public_key: [u8; 32],
        wallet_id: WalletV5R1WalletId,
        code: Arc<Cell>,
        workchain: i8,
    ) -> Result<Self, WalletError> {
        Ok(Self::new(public_key, wallet_id.pack()?, code, workchain))
    }

    /// Returns the wallet workchain.
    pub fn workchain(&self) -> i8 {
        self.workchain
    }

    /// Returns the raw 32-bit wallet id.
    pub fn wallet_id(&self) -> u32 {
        self.wallet_id
    }

    /// Returns the configured public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// Builds the initial data cell value.
    pub fn data(&self) -> WalletV5R1Data {
        WalletV5R1Data {
            is_signature_allowed: self.is_signature_allowed,
            seqno: 0,
            wallet_id: self.wallet_id,
            public_key: self.public_key,
            extensions: HashmapE::new(WALLET_V5R1_EXTENSIONS_KEY_BITS),
        }
    }

    /// Builds the wallet `StateInit`.
    pub fn state_init(&self) -> Result<StateInit, WalletError> {
        Ok(StateInit {
            code: Some(self.code.clone()),
            data: Some(self.data().to_cell()?),
            ..StateInit::empty()
        })
    }

    /// Derives the wallet address from `StateInit`.
    pub fn address(&self) -> Result<Address, WalletError> {
        let state_init = self.state_init()?;
        Ok(Address::new(self.workchain, state_init.to_cell()?.hash()))
    }

    /// Builds the unsigned signing cell for an external signed request.
    pub fn build_external_signing_cell(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
    ) -> Result<Arc<Cell>, WalletError> {
        self.build_external_signing_cell_with_extended_actions(
            seqno,
            valid_until,
            messages,
            Vec::new(),
        )
    }

    /// Builds the unsigned signing cell for an external signed request with
    /// optional Wallet V5R1 extended management actions.
    pub fn build_external_signing_cell_with_extended_actions(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        extended_actions: Vec<WalletV5R1ExtendedAction>,
    ) -> Result<Arc<Cell>, WalletError> {
        validate_action_count(messages.len() + extended_actions.len())?;
        let out_list = if messages.is_empty() {
            None
        } else {
            Some(OutList::new(
                messages
                    .into_iter()
                    .map(WalletMessage::into_action)
                    .collect(),
            ))
        };
        let extended_actions = if extended_actions.is_empty() {
            None
        } else {
            Some(WalletV5R1ExtendedActionList::new(extended_actions)?)
        };

        let mut builder = Builder::new();
        builder.store_u32(WALLET_V5R1_EXTERNAL_SIGNED_OP)?;
        builder.store_u32(self.wallet_id)?;
        builder.store_u32(valid_until)?;
        builder.store_u32(seqno)?;
        store_v5_inner_request(&mut builder, out_list.as_ref(), extended_actions.as_ref())?;
        Ok(builder.build()?)
    }

    /// Builds a signed external body cell and returns the body, signed hash, and
    /// Ed25519 signature.
    pub fn build_signed_external_body(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
    ) -> Result<WalletV5R1SignedBody, WalletError> {
        self.build_signed_external_body_with_extended_actions(
            seqno,
            valid_until,
            messages,
            Vec::new(),
            signing_key,
        )
    }

    /// Builds a signed external body cell with optional Wallet V5R1 extended
    /// management actions.
    pub fn build_signed_external_body_with_extended_actions(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        extended_actions: Vec<WalletV5R1ExtendedAction>,
        signing_key: &SigningKey,
    ) -> Result<WalletV5R1SignedBody, WalletError> {
        let signing_cell = self.build_external_signing_cell_with_extended_actions(
            seqno,
            valid_until,
            messages,
            extended_actions,
        )?;
        let signing_hash = signing_cell.hash();
        let signature = signing_key.sign(&signing_hash).to_bytes();

        let mut builder = Builder::new();
        builder.store_cell(&signing_cell)?;
        builder.store_bytes(&signature)?;
        Ok(WalletV5R1SignedBody {
            body: builder.build()?,
            signing_hash,
            signature,
        })
    }

    /// Builds an external inbound message BoC with the signed body.
    pub fn build_external_message_boc(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<Vec<u8>, WalletError> {
        self.build_external_message_boc_with_extended_actions(
            seqno,
            valid_until,
            messages,
            Vec::new(),
            signing_key,
            include_state_init,
        )
    }

    /// Builds an external inbound message BoC with optional Wallet V5R1
    /// extended management actions.
    pub fn build_external_message_boc_with_extended_actions(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        extended_actions: Vec<WalletV5R1ExtendedAction>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<Vec<u8>, WalletError> {
        let signed = self.build_signed_external_body_with_extended_actions(
            seqno,
            valid_until,
            messages,
            extended_actions,
            signing_key,
        )?;
        let state_init = if include_state_init {
            Some(Either::Right(self.state_init()?))
        } else {
            None
        };
        let message = Message {
            info: CommonMsgInfo::ExternalIn {
                src: MsgAddressExt::None,
                dest: MsgAddressInt::std(self.address()?),
                import_fee: Grams(BigUint::from(0u8)),
            },
            init: state_init,
            body: Either::Right(signed.body),
        };
        Ok(serialize_boc(&message.to_cell()?, false)?)
    }

    /// Reads the deployed wallet `seqno` get-method from the latest
    /// masterchain block known by the provider.
    #[cfg(feature = "liteclient")]
    pub async fn seqno<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<u32, WalletGetMethodError<P::Error>> {
        let stack = self.run_v5r1_get_method(provider, "seqno").await?;
        wallet_stack_u32("seqno", &stack, 0)
    }

    /// Reads the deployed wallet id through `get_wallet_id`.
    #[cfg(feature = "liteclient")]
    pub async fn wallet_id_onchain<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<u32, WalletGetMethodError<P::Error>> {
        let stack = self.run_v5r1_get_method(provider, "get_wallet_id").await?;
        wallet_stack_u32("get_wallet_id", &stack, 0)
    }

    /// Reads the deployed Ed25519 public key through `get_public_key`.
    #[cfg(feature = "liteclient")]
    pub async fn public_key_onchain<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<[u8; 32], WalletGetMethodError<P::Error>> {
        let stack = self.run_v5r1_get_method(provider, "get_public_key").await?;
        wallet_stack_public_key("get_public_key", &stack, 0)
    }

    /// Reads whether signature authentication is enabled through
    /// `is_signature_allowed`.
    #[cfg(feature = "liteclient")]
    pub async fn is_signature_allowed_onchain<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<bool, WalletGetMethodError<P::Error>> {
        let stack = self
            .run_v5r1_get_method(provider, "is_signature_allowed")
            .await?;
        wallet_stack_bool_int("is_signature_allowed", &stack, 0)
    }

    /// Reads the raw extension dictionary payload through `get_extensions`.
    ///
    /// The returned cell or slice is preserved as-is; this helper intentionally
    /// does not decode extension addresses or dictionary values.
    #[cfg(feature = "liteclient")]
    pub async fn extensions_raw_onchain<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<Arc<Cell>, WalletGetMethodError<P::Error>> {
        let stack = self.run_v5r1_get_method(provider, "get_extensions").await?;
        wallet_stack_cell("get_extensions", &stack, 0)
    }

    /// Reads and decodes the deployed extension dictionary through
    /// `get_extensions`.
    #[cfg(feature = "liteclient")]
    pub async fn extensions_onchain<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<WalletV5R1Extensions, WalletGetMethodError<P::Error>> {
        let raw = self.extensions_raw_onchain(provider).await?;
        let mut slice = Slice::new(raw);
        let extensions = WalletV5R1Extensions::load_tlb(&mut slice).map_err(|error| {
            WalletGetMethodError::InvalidCell {
                method: "get_extensions",
                error: error.to_string(),
            }
        })?;
        ensure_empty(&slice).map_err(|error| WalletGetMethodError::InvalidCell {
            method: "get_extensions",
            error: error.to_string(),
        })?;
        Ok(extensions)
    }

    #[cfg(feature = "liteclient")]
    async fn run_v5r1_get_method<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
        method: &'static str,
    ) -> Result<crate::tvm::TvmStack, WalletGetMethodError<P::Error>> {
        use crate::contracts::{DecodedRunMethodResult, RunMethodResultExt};
        use crate::tvm::TvmStack;

        let block = provider
            .get_masterchain_info()
            .await
            .map_err(WalletGetMethodError::Provider)?
            .last;
        let result = provider
            .run_get_method(
                0,
                block,
                self.address()?,
                crate::utils::method_name_to_id(method),
                TvmStack::empty(),
            )
            .await
            .map_err(WalletGetMethodError::Provider)?;

        if result.exit_code != 0 {
            return Err(WalletGetMethodError::NonZeroExitCode {
                method,
                exit_code: result.exit_code,
            });
        }

        match result.result_stack_lossless() {
            DecodedRunMethodResult::Decoded(stack) => Ok(stack),
            DecodedRunMethodResult::Missing => Err(WalletGetMethodError::MissingStack { method }),
            DecodedRunMethodResult::Undecodable { error, .. } => {
                Err(WalletGetMethodError::UndecodableStack { method, error })
            }
        }
    }

    /// Sends a signed external message BoC through any contract provider.
    #[cfg(feature = "liteclient")]
    pub async fn send_external_message<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<u32, WalletSendError<P::Error>> {
        let boc = self
            .build_external_message_boc(
                seqno,
                valid_until,
                messages,
                signing_key,
                include_state_init,
            )
            .map_err(WalletSendError::Build)?;
        provider
            .send_external_message_boc(boc)
            .await
            .map_err(WalletSendError::Provider)
    }
}

/// Signed Wallet V5R1 external body material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV5R1SignedBody {
    /// Final body cell containing signing fields and signature.
    pub body: Arc<Cell>,
    /// Representation hash that was signed.
    pub signing_hash: [u8; 32],
    /// Ed25519 signature bytes.
    pub signature: [u8; 64],
}

/// Decoded Wallet V5R1 external signed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV5R1ExternalBody {
    /// Wallet id from the signed request.
    pub wallet_id: u32,
    /// Expiration unix timestamp.
    pub valid_until: u32,
    /// Message seqno.
    pub seqno: u32,
    /// Standard outgoing actions, when present.
    pub out_list: Option<OutList>,
    /// Wallet management actions, when present.
    pub extended_actions: Option<WalletV5R1ExtendedActionList>,
    /// Ed25519 signature bytes.
    pub signature: [u8; 64],
}

