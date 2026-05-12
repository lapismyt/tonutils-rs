impl TlbDeserialize for TrComputePhase {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        if !load_tag_bit(slice, "TrComputePhase", "0|1", "")? {
            return Ok(Self::Skipped {
                reason: ComputeSkipReason::load_tlb(slice)?,
            });
        }

        let success = slice.load_bit()?;
        let msg_state_used = slice.load_bit()?;
        let account_activated = slice.load_bit()?;
        let gas_fees = Grams::load_tlb(slice)?;
        let child = slice.load_reference()?;
        let mut child_slice = Slice::new(child);
        let vm = load_compute_vm_tail(
            &mut child_slice,
            success,
            msg_state_used,
            account_activated,
            gas_fees,
        )
        .map_err(|source| TlbError::InvalidReferencePayload {
            schema: "TrComputePhase.vm",
            source: Box::new(source),
        })?;
        ensure_empty(&child_slice).map_err(|source| TlbError::InvalidReferencePayload {
            schema: "TrComputePhase.vm",
            source: Box::new(source),
        })?;
        Ok(vm)
    }
}

/// TL-B `TrBouncePhase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrBouncePhase {
    /// `tr_phase_bounce_negfunds$00`.
    NegativeFunds,
    /// `tr_phase_bounce_nofunds$01`.
    NoFunds {
        /// Size of the bounced message.
        msg_size: StorageUsed,
        /// Required forwarding fees.
        req_fwd_fees: Grams,
    },
    /// `tr_phase_bounce_ok$1`.
    Ok {
        /// Size of the bounced message.
        msg_size: StorageUsed,
        /// Message fees.
        msg_fees: Grams,
        /// Forwarding fees.
        fwd_fees: Grams,
    },
}

impl TlbSerialize for TrBouncePhase {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::NegativeFunds => store_tag(builder, "00")?,
            Self::NoFunds {
                msg_size,
                req_fwd_fees,
            } => {
                store_tag(builder, "01")?;
                msg_size.store_tlb(builder)?;
                req_fwd_fees.store_tlb(builder)?;
            }
            Self::Ok {
                msg_size,
                msg_fees,
                fwd_fees,
            } => {
                store_tag(builder, "1")?;
                msg_size.store_tlb(builder)?;
                msg_fees.store_tlb(builder)?;
                fwd_fees.store_tlb(builder)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for TrBouncePhase {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let first = load_tag_bit(slice, "TrBouncePhase", "00|01|1", "")?;
        if first {
            return Ok(Self::Ok {
                msg_size: StorageUsed::load_tlb(slice)?,
                msg_fees: Grams::load_tlb(slice)?,
                fwd_fees: Grams::load_tlb(slice)?,
            });
        }

        let second = load_tag_bit(slice, "TrBouncePhase", "00|01|1", "0")?;
        if second {
            Ok(Self::NoFunds {
                msg_size: StorageUsed::load_tlb(slice)?,
                req_fwd_fees: Grams::load_tlb(slice)?,
            })
        } else {
            Ok(Self::NegativeFunds)
        }
    }
}

/// TL-B `split_merge_info$_ ... = SplitMergeInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMergeInfo {
    /// Current shard prefix length, encoded in six bits.
    pub cur_shard_pfx_len: u8,
    /// Account split depth, encoded in six bits.
    pub acc_split_depth: u8,
    /// Current account address bits.
    pub this_addr: [u8; 32],
    /// Sibling account address bits.
    pub sibling_addr: [u8; 32],
}

impl TlbSerialize for SplitMergeInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        validate_u6("SplitMergeInfo.cur_shard_pfx_len", self.cur_shard_pfx_len)?;
        validate_u6("SplitMergeInfo.acc_split_depth", self.acc_split_depth)?;
        builder.store_uint_custom::<u8>(self.cur_shard_pfx_len as u8, 6)?;
        builder.store_uint_custom::<u8>(self.acc_split_depth as u8, 6)?;
        builder.store_bytes(&self.this_addr)?;
        builder.store_bytes(&self.sibling_addr)?;
        Ok(())
    }
}

impl TlbDeserialize for SplitMergeInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let mut this_addr = [0; 32];
        let mut sibling_addr = [0; 32];
        let cur_shard_pfx_len = slice.load_uint_custom::<u8>(6)? as u8;
        let acc_split_depth = slice.load_uint_custom::<u8>(6)? as u8;
        this_addr.copy_from_slice(&slice.load_bytes(32)?);
        sibling_addr.copy_from_slice(&slice.load_bytes(32)?);
        Ok(Self {
            cur_shard_pfx_len,
            acc_split_depth,
            this_addr,
            sibling_addr,
        })
    }
}

/// TL-B `TransactionDescr` constructors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionDescr {
    /// `trans_ord$0000`.
    Ordinary {
        /// Whether credit is processed before storage.
        credit_first: bool,
        /// Optional storage phase.
        storage_ph: Option<TrStoragePhase>,
        /// Optional credit phase.
        credit_ph: Option<TrCreditPhase>,
        /// Compute phase.
        compute_ph: TrComputePhase,
        /// Optional referenced action phase.
        action: Option<TrActionPhase>,
        /// Whether the transaction aborted.
        aborted: bool,
        /// Optional bounce phase.
        bounce: Option<TrBouncePhase>,
        /// Whether the account was destroyed.
        destroyed: bool,
    },
    /// `trans_storage$0001`.
    Storage {
        /// Storage-only phase.
        storage_ph: TrStoragePhase,
    },
    /// `trans_tick_tock$001`.
    TickTock {
        /// `true` for tock, `false` for tick.
        is_tock: bool,
        /// Storage phase.
        storage_ph: TrStoragePhase,
        /// Compute phase.
        compute_ph: TrComputePhase,
        /// Optional referenced action phase.
        action: Option<TrActionPhase>,
        /// Whether the transaction aborted.
        aborted: bool,
        /// Whether the account was destroyed.
        destroyed: bool,
    },
    /// `trans_split_prepare$0100`.
    SplitPrepare {
        /// Split metadata.
        split_info: SplitMergeInfo,
        /// Optional storage phase.
        storage_ph: Option<TrStoragePhase>,
        /// Compute phase.
        compute_ph: TrComputePhase,
        /// Optional referenced action phase.
        action: Option<TrActionPhase>,
        /// Whether the transaction aborted.
        aborted: bool,
        /// Whether the account was destroyed.
        destroyed: bool,
    },
    /// `trans_split_install$0101`.
    SplitInstall {
        /// Split metadata.
        split_info: SplitMergeInfo,
        /// Referenced prepared transaction.
        prepare_transaction: Box<Transaction>,
        /// Whether the prepared transaction was installed.
        installed: bool,
    },
    /// `trans_merge_prepare$0110`.
    MergePrepare {
        /// Merge metadata.
        split_info: SplitMergeInfo,
        /// Storage phase.
        storage_ph: TrStoragePhase,
        /// Whether the transaction aborted.
        aborted: bool,
    },
    /// `trans_merge_install$0111`.
    MergeInstall {
        /// Merge metadata.
        split_info: SplitMergeInfo,
        /// Referenced prepared transaction.
        prepare_transaction: Box<Transaction>,
        /// Optional storage phase.
        storage_ph: Option<TrStoragePhase>,
        /// Optional credit phase.
        credit_ph: Option<TrCreditPhase>,
        /// Compute phase.
        compute_ph: TrComputePhase,
        /// Optional referenced action phase.
        action: Option<TrActionPhase>,
        /// Whether the transaction aborted.
        aborted: bool,
        /// Whether the account was destroyed.
        destroyed: bool,
    },
}

impl TlbSerialize for TransactionDescr {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Ordinary {
                credit_first,
                storage_ph,
                credit_ph,
                compute_ph,
                action,
                aborted,
                bounce,
                destroyed,
            } => {
                store_tag(builder, "0000")?;
                builder.store_bit(*credit_first)?;
                store_maybe(builder, storage_ph)?;
                store_maybe(builder, credit_ph)?;
                compute_ph.store_tlb(builder)?;
                store_maybe_ref_action_phase(builder, action)?;
                builder.store_bit(*aborted)?;
                store_maybe(builder, bounce)?;
                builder.store_bit(*destroyed)?;
            }
            Self::Storage { storage_ph } => {
                store_tag(builder, "0001")?;
                storage_ph.store_tlb(builder)?;
            }
            Self::TickTock {
                is_tock,
                storage_ph,
                compute_ph,
                action,
                aborted,
                destroyed,
            } => {
                store_tag(builder, "001")?;
                builder.store_bit(*is_tock)?;
                storage_ph.store_tlb(builder)?;
                compute_ph.store_tlb(builder)?;
                store_maybe_ref_action_phase(builder, action)?;
                builder.store_bit(*aborted)?;
                builder.store_bit(*destroyed)?;
            }
            Self::SplitPrepare {
                split_info,
                storage_ph,
                compute_ph,
                action,
                aborted,
                destroyed,
            } => {
                store_tag(builder, "0100")?;
                split_info.store_tlb(builder)?;
                store_maybe(builder, storage_ph)?;
                compute_ph.store_tlb(builder)?;
                store_maybe_ref_action_phase(builder, action)?;
                builder.store_bit(*aborted)?;
                builder.store_bit(*destroyed)?;
            }
            Self::SplitInstall {
                split_info,
                prepare_transaction,
                installed,
            } => {
                store_tag(builder, "0101")?;
                split_info.store_tlb(builder)?;
                store_ref_tlb(builder, prepare_transaction.as_ref())?;
                builder.store_bit(*installed)?;
            }
            Self::MergePrepare {
                split_info,
                storage_ph,
                aborted,
            } => {
                store_tag(builder, "0110")?;
                split_info.store_tlb(builder)?;
                storage_ph.store_tlb(builder)?;
                builder.store_bit(*aborted)?;
            }
            Self::MergeInstall {
                split_info,
                prepare_transaction,
                storage_ph,
                credit_ph,
                compute_ph,
                action,
                aborted,
                destroyed,
            } => {
                store_tag(builder, "0111")?;
                split_info.store_tlb(builder)?;
                store_ref_tlb(builder, prepare_transaction.as_ref())?;
                store_maybe(builder, storage_ph)?;
                store_maybe(builder, credit_ph)?;
                compute_ph.store_tlb(builder)?;
                store_maybe_ref_action_phase(builder, action)?;
                builder.store_bit(*aborted)?;
                builder.store_bit(*destroyed)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for TransactionDescr {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        match load_transaction_descr_tag(slice)? {
            TransactionDescrTag::Ordinary => Ok(Self::Ordinary {
                credit_first: slice.load_bit()?,
                storage_ph: load_maybe(slice)?,
                credit_ph: load_maybe(slice)?,
                compute_ph: TrComputePhase::load_tlb(slice)?,
                action: load_maybe_ref_action_phase(slice)?,
                aborted: slice.load_bit()?,
                bounce: load_maybe(slice)?,
                destroyed: slice.load_bit()?,
            }),
            TransactionDescrTag::Storage => Ok(Self::Storage {
                storage_ph: TrStoragePhase::load_tlb(slice)?,
            }),
            TransactionDescrTag::TickTock => Ok(Self::TickTock {
                is_tock: slice.load_bit()?,
                storage_ph: TrStoragePhase::load_tlb(slice)?,
                compute_ph: TrComputePhase::load_tlb(slice)?,
                action: load_maybe_ref_action_phase(slice)?,
                aborted: slice.load_bit()?,
                destroyed: slice.load_bit()?,
            }),
            TransactionDescrTag::SplitPrepare => Ok(Self::SplitPrepare {
                split_info: SplitMergeInfo::load_tlb(slice)?,
                storage_ph: load_maybe(slice)?,
                compute_ph: TrComputePhase::load_tlb(slice)?,
                action: load_maybe_ref_action_phase(slice)?,
                aborted: slice.load_bit()?,
                destroyed: slice.load_bit()?,
            }),
            TransactionDescrTag::SplitInstall => Ok(Self::SplitInstall {
                split_info: SplitMergeInfo::load_tlb(slice)?,
                prepare_transaction: Box::new(load_ref_tlb(slice, "Transaction")?),
                installed: slice.load_bit()?,
            }),
            TransactionDescrTag::MergePrepare => Ok(Self::MergePrepare {
                split_info: SplitMergeInfo::load_tlb(slice)?,
                storage_ph: TrStoragePhase::load_tlb(slice)?,
                aborted: slice.load_bit()?,
            }),
            TransactionDescrTag::MergeInstall => Ok(Self::MergeInstall {
                split_info: SplitMergeInfo::load_tlb(slice)?,
                prepare_transaction: Box::new(load_ref_tlb(slice, "Transaction")?),
                storage_ph: load_maybe(slice)?,
                credit_ph: load_maybe(slice)?,
                compute_ph: TrComputePhase::load_tlb(slice)?,
                action: load_maybe_ref_action_phase(slice)?,
                aborted: slice.load_bit()?,
                destroyed: slice.load_bit()?,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionDescrTag {
    Ordinary,
    Storage,
    TickTock,
    SplitPrepare,
    SplitInstall,
    MergePrepare,
    MergeInstall,
}

fn load_compute_vm_tail(
    slice: &mut Slice,
    success: bool,
    msg_state_used: bool,
    account_activated: bool,
    gas_fees: Grams,
) -> Result<TrComputePhase> {
    let gas_used = load_var_uint_7(slice, "TrComputePhase.gas_used")?;
    let gas_limit = load_var_uint_7(slice, "TrComputePhase.gas_limit")?;
    let gas_credit = load_maybe_var_uint_3(slice)?;
    let mode = slice.load_int(8)? as i8;
    let exit_code = slice.load_int(32)? as i32;
    let exit_arg = load_maybe_i32(slice)?;
    let vm_steps = slice.load_u32()?;
    let mut vm_init_state_hash = [0; 32];
    vm_init_state_hash.copy_from_slice(&slice.load_bytes(32)?);
    let mut vm_final_state_hash = [0; 32];
    vm_final_state_hash.copy_from_slice(&slice.load_bytes(32)?);
    Ok(TrComputePhase::Vm {
        success,
        msg_state_used,
        account_activated,
        gas_fees,
        gas_used,
        gas_limit,
        gas_credit,
        mode,
        exit_code,
        exit_arg,
        vm_steps,
        vm_init_state_hash,
        vm_final_state_hash,
    })
}

fn store_maybe_ref_action_phase(
    builder: &mut Builder,
    action: &Option<TrActionPhase>,
) -> Result<()> {
    match action {
        Some(action) => {
            builder.store_bit(true)?;
            store_ref_tlb(builder, action)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_maybe_ref_action_phase(slice: &mut Slice) -> Result<Option<TrActionPhase>> {
    if slice.load_bit()? {
        Ok(Some(load_ref_tlb(slice, "TrActionPhase")?))
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

fn store_maybe_var_uint_3(builder: &mut Builder, value: &Option<BigUint>) -> Result<()> {
    match value {
        Some(value) => {
            builder.store_bit(true)?;
            store_var_uint_3(builder, value, "TrComputePhase.gas_credit")?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_maybe_var_uint_3(slice: &mut Slice) -> Result<Option<BigUint>> {
    if slice.load_bit()? {
        Ok(Some(load_var_uint_3(slice, "TrComputePhase.gas_credit")?))
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

fn store_var_uint_3(builder: &mut Builder, value: &BigUint, schema: &'static str) -> Result<()> {
    let byte_len = value.to_bytes_be().len();
    if byte_len > VAR_UINT_3_MAX_BYTES {
        return Err(TlbError::NonCanonicalValue {
            schema,
            reason: format!("byte length {byte_len} exceeds maximum {VAR_UINT_3_MAX_BYTES}"),
        });
    }
    store_var_uint(builder, value, VAR_UINT_3_LEN_BITS)
}

fn load_var_uint_3(slice: &mut Slice, schema: &'static str) -> Result<BigUint> {
    let value = load_var_uint(slice, VAR_UINT_3_LEN_BITS)?;
    let byte_len = value.to_bytes_be().len();
    if byte_len > VAR_UINT_3_MAX_BYTES {
        return Err(TlbError::NonCanonicalValue {
            schema,
            reason: format!("byte length {byte_len} exceeds maximum {VAR_UINT_3_MAX_BYTES}"),
        });
    }
    Ok(value)
}

fn validate_u6(schema: &'static str, value: u8) -> Result<()> {
    if value > 63 {
        Err(TlbError::CustomSchema {
            schema,
            message: format!("value {value} does not fit in six bits"),
        })
    } else {
        Ok(())
    }
}

fn store_maybe_ref_message(builder: &mut Builder, message: &Option<Message>) -> Result<()> {
    match message {
        Some(message) => {
            builder.store_bit(true)?;
            store_ref_tlb(builder, message)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_maybe_ref_message(slice: &mut Slice) -> Result<Option<Message>> {
    if slice.load_bit()? {
        Ok(Some(load_ref_tlb(slice, "Message Any")?))
    } else {
        Ok(None)
    }
}

fn expect_tag_bits(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<()> {
    let mut actual_bits = String::with_capacity(expected_bits.len());
    for expected in expected_bits.bytes() {
        let bit = slice.load_bit().map_err(|_| TlbError::TagMismatch {
            constructor,
            expected_bits,
            actual_bits: actual_bits.clone(),
        })?;
        actual_bits.push(if bit { '1' } else { '0' });
        if bit != (expected == b'1') {
            return Err(TlbError::TagMismatch {
                constructor,
                expected_bits,
                actual_bits,
            });
        }
    }
    Ok(())
}

fn expect_u8_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
    expected_tag: u8,
) -> Result<()> {
    let mut actual_bits = String::with_capacity(8);
    let mut tag = 0u8;
    for _ in 0..8 {
        let bit = slice.load_bit().map_err(|_| TlbError::TagMismatch {
            constructor,
            expected_bits,
            actual_bits: actual_bits.clone(),
        })?;
        actual_bits.push(if bit { '1' } else { '0' });
        tag = (tag << 1) | u8::from(bit);
    }
    if tag == expected_tag {
        Ok(())
    } else {
        Err(TlbError::TagMismatch {
            constructor,
            expected_bits,
            actual_bits,
        })
    }
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

fn load_three_bit_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<String> {
    let mut actual_bits = String::with_capacity(3);
    for _ in 0..3 {
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

fn anyhow_to_tlb_error(error: anyhow::Error) -> TlbError {
    match error.downcast::<TlbError>() {
        Ok(error) => error,
        Err(error) => TlbError::Tvm(error),
    }
}

fn load_tag_bit(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
    actual_prefix: &'static str,
) -> Result<bool> {
    slice.load_bit().map_err(|_| TlbError::TagMismatch {
        constructor,
        expected_bits,
        actual_bits: actual_prefix.to_string(),
    })
}

fn load_transaction_descr_tag(slice: &mut Slice) -> Result<TransactionDescrTag> {
    let b0 = load_descr_tag_bit(slice, "")?;
    if b0 {
        return Err(TlbError::TagMismatch {
            constructor: "TransactionDescr",
            expected_bits: "0000|0001|001|0100|0101|0110|0111",
            actual_bits: "1".to_string(),
        });
    }

    let b1 = load_descr_tag_bit(slice, "0")?;
    let b2 = load_descr_tag_bit(slice, if b1 { "01" } else { "00" })?;
    match (b1, b2) {
        (false, false) => match load_descr_tag_bit(slice, "000")? {
            false => Ok(TransactionDescrTag::Ordinary),
            true => Ok(TransactionDescrTag::Storage),
        },
        (false, true) => Ok(TransactionDescrTag::TickTock),
        (true, false) => match load_descr_tag_bit(slice, "010")? {
            false => Ok(TransactionDescrTag::SplitPrepare),
            true => Ok(TransactionDescrTag::SplitInstall),
        },
        (true, true) => match load_descr_tag_bit(slice, "011")? {
            false => Ok(TransactionDescrTag::MergePrepare),
            true => Ok(TransactionDescrTag::MergeInstall),
        },
    }
}

fn load_descr_tag_bit(slice: &mut Slice, actual_prefix: &'static str) -> Result<bool> {
    load_tag_bit(
        slice,
        "TransactionDescr",
        "0000|0001|001|0100|0101|0110|0111",
        actual_prefix,
    )
}
