impl TlbDeserialize for MessageRelaxed {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let info = CommonMsgInfoRelaxed::load_tlb(slice)?;
        let init = load_message_init(slice)?;
        let body = load_message_body(slice)?;
        Ok(Self { info, init, body })
    }
}

/// TL-B `LibRef` used by `action_change_library`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibRef {
    /// `libref_hash$0 lib_hash:bits256`.
    Hash([u8; 32]),
    /// `libref_ref$1 library:^Cell`.
    Ref(Arc<Cell>),
}

impl TlbSerialize for LibRef {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Hash(hash) => {
                builder.store_bit(false)?;
                builder.store_bytes(hash)?;
            }
            Self::Ref(library) => {
                builder.store_bit(true)?;
                builder.store_ref(library.clone())?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for LibRef {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let tag = load_one_bit_tag(slice, "LibRef", "0|1")?;
        if tag == "0" {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&slice.load_bytes(32)?);
            Ok(Self::Hash(hash))
        } else {
            Ok(Self::Ref(slice.load_reference()?))
        }
    }
}

/// Closed upstream TL-B `OutAction` family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutAction {
    /// `action_send_msg#0ec3c86d mode:(## 8) out_msg:^(MessageRelaxed Any)`.
    SendMsg {
        /// Send mode byte.
        mode: u8,
        /// Referenced relaxed outbound message.
        out_msg: MessageRelaxed,
    },
    /// `action_set_code#ad4de08e new_code:^Cell`.
    SetCode {
        /// New contract code cell.
        new_code: Arc<Cell>,
    },
    /// `action_reserve_currency#36e6b809 mode:(## 8) currency:CurrencyCollection`.
    ReserveCurrency {
        /// Reserve mode byte.
        mode: u8,
        /// Currency collection to reserve.
        currency: CurrencyCollection,
    },
    /// `action_change_library#26fa1dd4 mode:(## 7) libref:LibRef`.
    ChangeLibrary {
        /// Seven-bit library action mode, constrained to `0..=127`.
        mode: u8,
        /// Library hash or referenced library cell.
        libref: LibRef,
    },
}

impl TlbSerialize for OutAction {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::SendMsg { mode, out_msg } => {
                builder.store_uint::<u32>(ACTION_SEND_MSG_TAG as u32)?;
                builder.store_uint::<u8>(*mode as u8)?;
                store_ref_tlb(builder, out_msg)?;
            }
            Self::SetCode { new_code } => {
                builder.store_uint::<u32>(ACTION_SET_CODE_TAG as u32)?;
                builder.store_ref(new_code.clone())?;
            }
            Self::ReserveCurrency { mode, currency } => {
                builder.store_uint::<u32>(ACTION_RESERVE_CURRENCY_TAG as u32)?;
                builder.store_uint::<u8>(*mode as u8)?;
                currency.store_tlb(builder)?;
            }
            Self::ChangeLibrary { mode, libref } => {
                if *mode > 0x7F {
                    return Err(TlbError::CustomSchema {
                        schema: "OutAction.action_change_library.mode",
                        message: format!("mode {mode} does not fit in seven bits"),
                    });
                }
                builder.store_uint::<u32>(ACTION_CHANGE_LIBRARY_TAG as u32)?;
                builder.store_uint_custom::<u8>(*mode as u8, 7)?;
                libref.store_tlb(builder)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for OutAction {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        match load_u32_tag(slice, "OutAction", OUT_ACTION_EXPECTED_TAGS)? {
            ACTION_SEND_MSG_TAG => Ok(Self::SendMsg {
                mode: slice.load_uint::<u8>()? as u8,
                out_msg: load_ref_tlb(slice, "MessageRelaxed Any")?,
            }),
            ACTION_SET_CODE_TAG => Ok(Self::SetCode {
                new_code: slice.load_reference()?,
            }),
            ACTION_RESERVE_CURRENCY_TAG => Ok(Self::ReserveCurrency {
                mode: slice.load_uint::<u8>()? as u8,
                currency: CurrencyCollection::load_tlb(slice)?,
            }),
            ACTION_CHANGE_LIBRARY_TAG => Ok(Self::ChangeLibrary {
                mode: slice.load_uint_custom::<u8>(7)? as u8,
                libref: LibRef::load_tlb(slice)?,
            }),
            _ => unreachable!("load_u32_tag only returns known OutAction tags"),
        }
    }
}

const OUT_ACTION_EXPECTED_TAGS: &str = "#0ec3c86d|#ad4de08e|#36e6b809|#26fa1dd4";

/// TL-B `OutList`, represented in execution/schema order.
///
/// The first action is stored deepest next to `out_list_empty$_`, and the last
/// action is stored in the root node.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutList {
    /// Actions in upstream execution/schema order.
    pub actions: Vec<OutAction>,
}

impl OutList {
    /// Creates an action list from actions in execution/schema order.
    pub fn new(actions: Vec<OutAction>) -> Self {
        Self { actions }
    }

    /// Returns the number of actions in the list.
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Returns whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    fn validate_len(&self) -> Result<()> {
        if self.actions.len() > MAX_OUT_LIST_ACTIONS {
            return Err(TlbError::CustomSchema {
                schema: "OutList",
                message: format!(
                    "action count {} exceeds maximum {MAX_OUT_LIST_ACTIONS}",
                    self.actions.len()
                ),
            });
        }
        Ok(())
    }

    fn load_with_depth(slice: &mut Slice, depth: usize) -> Result<Self> {
        if slice.is_empty() {
            return Ok(Self::default());
        }

        if depth >= MAX_OUT_LIST_ACTIONS {
            return Err(TlbError::CustomSchema {
                schema: "OutList",
                message: format!("action count exceeds maximum {MAX_OUT_LIST_ACTIONS}"),
            });
        }

        if slice.remaining_refs() == 0 {
            return Err(TlbError::CustomSchema {
                schema: "OutList",
                message: "non-empty node is missing previous-list reference".to_string(),
            });
        }

        let prev_cell = slice.load_reference()?;
        let mut prev_slice = Slice::new(prev_cell);
        let mut list = Self::load_with_depth(&mut prev_slice, depth + 1).map_err(|source| {
            TlbError::InvalidReferencePayload {
                schema: "OutList",
                source: Box::new(source),
            }
        })?;
        ensure_empty(&prev_slice).map_err(|source| TlbError::InvalidReferencePayload {
            schema: "OutList",
            source: Box::new(source),
        })?;

        list.actions.push(OutAction::load_tlb(slice)?);
        Ok(list)
    }
}

impl TlbSerialize for OutList {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.validate_len()?;

        let mut current = Builder::new().build()?;
        for action in &self.actions {
            let mut node = Builder::new();
            node.store_ref(current)?;
            action.store_tlb(&mut node)?;
            current = node.build()?;
        }

        builder.store_cell(&current)?;
        Ok(())
    }
}

impl TlbDeserialize for OutList {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Self::load_with_depth(slice, 0)
    }
}

/// TL-B `AccStatusChange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccStatusChange {
    /// `acst_unchanged$0`.
    Unchanged,
    /// `acst_frozen$10`.
    Frozen,
    /// `acst_deleted$11`.
    Deleted,
}

impl TlbSerialize for AccStatusChange {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Unchanged => store_tag(builder, "0"),
            Self::Frozen => store_tag(builder, "10"),
            Self::Deleted => store_tag(builder, "11"),
        }
    }
}

impl TlbDeserialize for AccStatusChange {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let first = match slice.load_bit() {
            Ok(bit) => bit,
            Err(_) => {
                return Err(TlbError::TagMismatch {
                    constructor: "AccStatusChange",
                    expected_bits: "0|10|11",
                    actual_bits: String::new(),
                });
            }
        };
        if !first {
            return Ok(Self::Unchanged);
        }

        match slice.load_bit() {
            Ok(false) => Ok(Self::Frozen),
            Ok(true) => Ok(Self::Deleted),
            Err(_) => Err(TlbError::TagMismatch {
                constructor: "AccStatusChange",
                expected_bits: "0|10|11",
                actual_bits: "1".to_string(),
            }),
        }
    }
}

/// TL-B `storage_used$_ cells:(VarUInteger 7) bits:(VarUInteger 7) = StorageUsed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageUsed {
    /// Cell count encoded as canonical `VarUInteger 7`.
    pub cells: BigUint,
    /// Bit count encoded as canonical `VarUInteger 7`.
    pub bits: BigUint,
}

impl StorageUsed {
    /// Creates a storage-size value from cell and bit counters.
    pub fn new(cells: BigUint, bits: BigUint) -> Self {
        Self { cells, bits }
    }
}

impl TlbSerialize for StorageUsed {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_var_uint_7(builder, &self.cells, "StorageUsed.cells")?;
        store_var_uint_7(builder, &self.bits, "StorageUsed.bits")?;
        Ok(())
    }
}

impl TlbDeserialize for StorageUsed {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            cells: load_var_uint_7(slice, "StorageUsed.cells")?,
            bits: load_var_uint_7(slice, "StorageUsed.bits")?,
        })
    }
}

/// TL-B `tr_phase_action$_ ... = TrActionPhase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrActionPhase {
    /// Whether the action phase completed successfully.
    pub success: bool,
    /// Whether the action list was valid.
    pub valid: bool,
    /// Whether execution ran out of funds while processing actions.
    pub no_funds: bool,
    /// Account status transition caused by the action phase.
    pub status_change: AccStatusChange,
    /// Optional total forwarded fees.
    pub total_fwd_fees: Option<Grams>,
    /// Optional total action fees.
    pub total_action_fees: Option<Grams>,
    /// Action phase result code.
    pub result_code: i32,
    /// Optional action phase result argument.
    pub result_arg: Option<i32>,
    /// Total action count.
    pub tot_actions: u16,
    /// Special action count.
    pub spec_actions: u16,
    /// Skipped action count.
    pub skipped_actions: u16,
    /// Created message count.
    pub msgs_created: u16,
    /// Hash of the `OutList` action list; the list itself is not embedded here.
    pub action_list_hash: [u8; 32],
    /// Total size of created messages.
    pub tot_msg_size: StorageUsed,
}

impl TlbSerialize for TrActionPhase {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_bit(self.success)?;
        builder.store_bit(self.valid)?;
        builder.store_bit(self.no_funds)?;
        self.status_change.store_tlb(builder)?;
        store_maybe(builder, &self.total_fwd_fees)?;
        store_maybe(builder, &self.total_action_fees)?;
        builder.store_int(self.result_code as i64, 32)?;
        store_maybe_i32(builder, self.result_arg)?;
        builder.store_uint::<u16>(self.tot_actions as u16)?;
        builder.store_uint::<u16>(self.spec_actions as u16)?;
        builder.store_uint::<u16>(self.skipped_actions as u16)?;
        builder.store_uint::<u16>(self.msgs_created as u16)?;
        builder.store_bytes(&self.action_list_hash)?;
        self.tot_msg_size.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for TrActionPhase {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let success = slice.load_bit()?;
        let valid = slice.load_bit()?;
        let no_funds = slice.load_bit()?;
        let status_change = AccStatusChange::load_tlb(slice)?;
        let total_fwd_fees = load_maybe::<Grams>(slice)?;
        let total_action_fees = load_maybe::<Grams>(slice)?;
        let result_code = slice.load_int(32)? as i32;
        let result_arg = load_maybe_i32(slice)?;
        let tot_actions = slice.load_uint::<u16>()? as u16;
        let spec_actions = slice.load_uint::<u16>()? as u16;
        let skipped_actions = slice.load_uint::<u16>()? as u16;
        let msgs_created = slice.load_uint::<u16>()? as u16;
        let mut action_list_hash = [0u8; 32];
        action_list_hash.copy_from_slice(&slice.load_bytes(32)?);
        let tot_msg_size = StorageUsed::load_tlb(slice)?;
        Ok(Self {
            success,
            valid,
            no_funds,
            status_change,
            total_fwd_fees,
            total_action_fees,
            result_code,
            result_arg,
            tot_actions,
            spec_actions,
            skipped_actions,
            msgs_created,
            action_list_hash,
            tot_msg_size,
        })
    }
}

fn store_state_init_prefix(
    builder: &mut Builder,
    fixed_prefix_length: Option<u8>,
    special: &Option<TickTock>,
    code: &Option<Arc<Cell>>,
    data: &Option<Arc<Cell>>,
    fixed_prefix_schema: &'static str,
) -> Result<()> {
    match fixed_prefix_length {
        Some(value) => {
            if value >= 32 {
                return Err(TlbError::CustomSchema {
                    schema: fixed_prefix_schema,
                    message: format!("value {value} does not fit in five bits"),
                });
            }
            builder.store_bit(true)?;
            builder.store_uint_custom::<u8>(value as u8, 5)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    store_maybe_tick_tock(builder, special)?;
    builder.store_maybe_ref(code.clone())?;
    builder.store_maybe_ref(data.clone())?;
    Ok(())
}

fn load_state_init_prefix(
    slice: &mut Slice,
) -> Result<(
    Option<u8>,
    Option<TickTock>,
    Option<Arc<Cell>>,
    Option<Arc<Cell>>,
)> {
    let fixed_prefix_length = if slice.load_bit()? {
        Some(slice.load_uint_custom::<u8>(5)? as u8)
    } else {
        None
    };
    Ok((
        fixed_prefix_length,
        load_maybe_tick_tock(slice)?,
        load_maybe_ref_cell(slice)?,
        load_maybe_ref_cell(slice)?,
    ))
}

fn store_message_init(
    builder: &mut Builder,
    init: &Option<Either<StateInit, StateInit>>,
) -> Result<()> {
    match init {
        Some(Either::Left(init)) => {
            builder.store_bit(true)?;
            builder.store_bit(false)?;
            init.store_tlb(builder)?;
        }
        Some(Either::Right(init)) => {
            builder.store_bit(true)?;
            builder.store_bit(true)?;
            store_ref_tlb(builder, init)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_message_init(slice: &mut Slice) -> Result<Option<Either<StateInit, StateInit>>> {
    if slice.load_bit()? {
        if slice.load_bit()? {
            Ok(Some(Either::Right(load_ref_tlb(slice, "StateInit")?)))
        } else {
            Ok(Some(Either::Left(StateInit::load_tlb(slice)?)))
        }
    } else {
        Ok(None)
    }
}

fn store_message_body(builder: &mut Builder, body: &Either<Arc<Cell>, Arc<Cell>>) -> Result<()> {
    match body {
        Either::Left(cell) => {
            builder.store_bit(false)?;
            builder.store_cell(cell)?;
        }
        Either::Right(cell) => {
            builder.store_bit(true)?;
            builder.store_ref(cell.clone())?;
        }
    }
    Ok(())
}

fn load_message_body(slice: &mut Slice) -> Result<Either<Arc<Cell>, Arc<Cell>>> {
    if slice.load_bit()? {
        Ok(Either::Right(slice.load_reference()?))
    } else {
        Ok(Either::Left(load_remaining_as_cell(slice)?))
    }
}

fn store_maybe_anycast(builder: &mut Builder, anycast: &Option<Anycast>) -> Result<()> {
    match anycast {
        Some(anycast) => {
            builder.store_bit(true)?;
            anycast.store_tlb(builder)
        }
        None => {
            builder.store_bit(false)?;
            Ok(())
        }
    }
}

fn load_maybe_anycast(slice: &mut Slice) -> Result<Option<Anycast>> {
    if slice.load_bit()? {
        Ok(Some(Anycast::load_tlb(slice)?))
    } else {
        Ok(None)
    }
}

fn store_maybe_tick_tock(builder: &mut Builder, value: &Option<TickTock>) -> Result<()> {
    match value {
        Some(value) => {
            builder.store_bit(true)?;
            value.store_tlb(builder)
        }
        None => {
            builder.store_bit(false)?;
            Ok(())
        }
    }
}

fn load_maybe_tick_tock(slice: &mut Slice) -> Result<Option<TickTock>> {
    if slice.load_bit()? {
        Ok(Some(TickTock::load_tlb(slice)?))
    } else {
        Ok(None)
    }
}

fn load_maybe_ref_cell(slice: &mut Slice) -> Result<Option<Arc<Cell>>> {
    if slice.load_bit()? {
        Ok(Some(slice.load_reference()?))
    } else {
        Ok(None)
    }
}

fn store_maybe_i32(builder: &mut Builder, value: Option<i32>) -> Result<()> {
    match value {
        Some(value) => {
            builder.store_bit(true)?;
            builder.store_int(value as i64, 32)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_maybe_i32(slice: &mut Slice) -> Result<Option<i32>> {
    if slice.load_bit()? {
        Ok(Some(slice.load_int(32)? as i32))
    } else {
        Ok(None)
    }
}

fn store_var_uint_7(builder: &mut Builder, value: &BigUint, schema: &'static str) -> Result<()> {
    let byte_len = value.to_bytes_be().len();
    if byte_len > VAR_UINT_7_MAX_BYTES {
        return Err(TlbError::NonCanonicalValue {
            schema,
            reason: format!("byte length {byte_len} exceeds maximum {VAR_UINT_7_MAX_BYTES}"),
        });
    }
    store_var_uint(builder, value, VAR_UINT_7_LEN_BITS)
}

fn load_var_uint_7(slice: &mut Slice, schema: &'static str) -> Result<BigUint> {
    let value = load_var_uint(slice, VAR_UINT_7_LEN_BITS)?;
    let byte_len = value.to_bytes_be().len();
    if byte_len > VAR_UINT_7_MAX_BYTES {
        return Err(TlbError::NonCanonicalValue {
            schema,
            reason: format!("byte length {byte_len} exceeds maximum {VAR_UINT_7_MAX_BYTES}"),
        });
    }
    Ok(value)
}

fn load_remaining_as_cell(slice: &mut Slice) -> Result<Arc<Cell>> {
    let bits = slice.remaining_bits();
    let refs = slice.remaining_refs();
    let remaining = slice.clone_from_current();
    let mut builder = Builder::new();
    builder.store_slice(&remaining)?;
    let cell = builder.build()?;
    slice.skip_bits(bits)?;
    slice.skip_refs(refs)?;
    Ok(cell)
}

fn validate_bit_string(schema: &'static str, data: &[u8], bit_len: usize) -> Result<()> {
    let required_bytes = bit_len.div_ceil(8);
    if data.len() != required_bytes {
        return Err(TlbError::CustomSchema {
            schema,
            message: format!(
                "data length {} does not match bit length {bit_len}",
                data.len()
            ),
        });
    }
    if bit_len == 0 || data.is_empty() {
        return Ok(());
    }
    let unused_bits = data.len() * 8 - bit_len;
    if unused_bits > 0 {
        let mask = (1u8 << unused_bits) - 1;
        if data[data.len() - 1] & mask != 0 {
            return Err(TlbError::NonCanonicalValue {
                schema,
                reason: "unused final-byte bits must be zero".to_string(),
            });
        }
    }
    Ok(())
}

fn load_two_bit_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<String> {
    let mut actual_bits = String::with_capacity(2);
    for _ in 0..2 {
        match slice.load_bit() {
            Ok(bit) => actual_bits.push(if bit { '1' } else { '0' }),
            Err(_) => {
                return Err(TlbError::TagMismatch {
                    constructor,
                    expected_bits,
                    actual_bits,
                });
            }
        }
    }
    Ok(actual_bits)
}

fn load_one_bit_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<String> {
    match slice.load_bit() {
        Ok(bit) => Ok(if bit { "1" } else { "0" }.to_string()),
        Err(_) => Err(TlbError::TagMismatch {
            constructor,
            expected_bits,
            actual_bits: String::new(),
        }),
    }
}

fn load_u32_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<u32> {
    let mut actual_bits = String::with_capacity(32);
    let mut tag = 0u32;
    for _ in 0..32 {
        match slice.load_bit() {
            Ok(bit) => {
                actual_bits.push(if bit { '1' } else { '0' });
                tag = (tag << 1) | u32::from(bit);
            }
            Err(_) => {
                return Err(TlbError::TagMismatch {
                    constructor,
                    expected_bits,
                    actual_bits,
                });
            }
        }
    }

    match tag {
        ACTION_SEND_MSG_TAG
        | ACTION_SET_CODE_TAG
        | ACTION_RESERVE_CURRENCY_TAG
        | ACTION_CHANGE_LIBRARY_TAG => Ok(tag),
        _ => Err(TlbError::TagMismatch {
            constructor,
            expected_bits,
            actual_bits,
        }),
    }
}
