use super::*;

impl WalletV5R1ExternalBody {
    /// Decodes a signed Wallet V5R1 external body cell.
    pub fn from_cell(cell: Arc<Cell>) -> crate::tlb::Result<Self> {
        let signature_bit_len = 512;
        if cell.bit_len() < signature_bit_len {
            return Err(TlbError::CustomSchema {
                schema: "WalletV5R1ExternalBody",
                message: "body is shorter than the 512-bit signature".to_string(),
            });
        }
        let mut slice = Slice::new(cell);
        let op = slice.load_u32()?;
        if op != WALLET_V5R1_EXTERNAL_SIGNED_OP {
            return Err(TlbError::CustomSchema {
                schema: "WalletV5R1ExternalBody",
                message: format!("unexpected op 0x{op:08x}"),
            });
        }
        let wallet_id = slice.load_u32()?;
        let valid_until = slice.load_u32()?;
        let seqno = slice.load_u32()?;
        let (out_list, extended_actions) = load_v5_inner_request(&mut slice)?;
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&slice.load_bytes(64)?);
        ensure_empty(&slice)?;
        Ok(Self {
            wallet_id,
            valid_until,
            seqno,
            out_list,
            extended_actions,
            signature,
        })
    }
}

pub(super) fn validate_action_count(count: usize) -> Result<(), WalletError> {
    if count > WALLET_V5R1_MAX_ACTIONS {
        return Err(WalletError::TooManyActions {
            count,
            max: WALLET_V5R1_MAX_ACTIONS,
        });
    }
    Ok(())
}

pub(super) fn store_v5_inner_request(
    builder: &mut Builder,
    out_list: Option<&OutList>,
    extended_actions: Option<&WalletV5R1ExtendedActionList>,
) -> crate::tlb::Result<()> {
    match out_list {
        Some(list) => {
            builder.store_bit(true)?;
            builder.store_ref(list.to_cell()?)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    match extended_actions {
        Some(list) => {
            builder.store_bit(true)?;
            list.store_tlb(builder)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

pub(super) fn load_v5_inner_request(
    slice: &mut Slice,
) -> crate::tlb::Result<(Option<OutList>, Option<WalletV5R1ExtendedActionList>)> {
    let out_list = if slice.load_bit()? {
        Some(crate::tlb::load_ref_tlb(
            slice,
            "WalletV5R1ExternalBody.out_list",
        )?)
    } else {
        None
    };
    let extended_actions = if slice.load_bit()? {
        Some(WalletV5R1ExtendedActionList::load_tlb(slice)?)
    } else {
        None
    };
    Ok((out_list, extended_actions))
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_stack_entry<'a, E>(
    method: &'static str,
    stack: &'a crate::tvm::TvmStack,
    index: usize,
) -> Result<&'a crate::tvm::TvmStackEntry, WalletGetMethodError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    stack
        .entries()
        .get(index)
        .ok_or(WalletGetMethodError::MissingStackEntry { method, index })
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_stack_int<'a, E>(
    method: &'static str,
    stack: &'a crate::tvm::TvmStack,
    index: usize,
) -> Result<&'a BigInt, WalletGetMethodError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    match wallet_stack_entry(method, stack, index)? {
        crate::tvm::TvmStackEntry::Int(value) => Ok(value),
        _ => Err(WalletGetMethodError::WrongStackType {
            method,
            index,
            expected: "integer",
        }),
    }
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_stack_u32<E>(
    method: &'static str,
    stack: &crate::tvm::TvmStack,
    index: usize,
) -> Result<u32, WalletGetMethodError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let value = wallet_stack_int(method, stack, index)?;
    if value.sign() == Sign::Minus || value > &BigInt::from(u32::MAX) {
        return Err(WalletGetMethodError::IntegerRange {
            method,
            index,
            expected: "uint32",
        });
    }
    value
        .to_string()
        .parse::<u32>()
        .map_err(|_| WalletGetMethodError::IntegerRange {
            method,
            index,
            expected: "uint32",
        })
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_stack_bool_int<E>(
    method: &'static str,
    stack: &crate::tvm::TvmStack,
    index: usize,
) -> Result<bool, WalletGetMethodError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let value = wallet_stack_int(method, stack, index)?;
    if value == &BigInt::from(0u8) {
        Ok(false)
    } else if value == &BigInt::from(1u8) {
        Ok(true)
    } else {
        Err(WalletGetMethodError::IntegerRange {
            method,
            index,
            expected: "0 or 1",
        })
    }
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_stack_public_key<E>(
    method: &'static str,
    stack: &crate::tvm::TvmStack,
    index: usize,
) -> Result<[u8; 32], WalletGetMethodError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    let value = wallet_stack_int(method, stack, index)?;
    let Some(value) = value.to_biguint() else {
        return Err(WalletGetMethodError::IntegerRange {
            method,
            index,
            expected: "uint256",
        });
    };
    let bytes = value.to_bytes_be();
    if bytes.len() > 32 {
        return Err(WalletGetMethodError::PublicKeyWidth {
            method,
            actual_bits: bytes.len() * 8,
        });
    }
    let mut public_key = [0u8; 32];
    public_key[32 - bytes.len()..].copy_from_slice(&bytes);
    Ok(public_key)
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_stack_cell<E>(
    method: &'static str,
    stack: &crate::tvm::TvmStack,
    index: usize,
) -> Result<Arc<Cell>, WalletGetMethodError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    match wallet_stack_entry(method, stack, index)? {
        crate::tvm::TvmStackEntry::Cell(cell) | crate::tvm::TvmStackEntry::Slice(cell) => {
            if cell.bit_len() == 0 && cell.references().is_empty() {
                Err(WalletGetMethodError::MissingCell { method })
            } else {
                Ok(cell.clone())
            }
        }
        _ => Err(WalletGetMethodError::WrongStackType {
            method,
            index,
            expected: "cell or slice",
        }),
    }
}
