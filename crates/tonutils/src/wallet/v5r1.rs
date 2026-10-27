use super::*;

impl WalletV5R1ExternalBody {
    /// Decodes a signed Wallet V5R1 external body cell.
    pub fn from_cell(cell: Arc<Cell>) -> tonutils::tlb::Result<Self> {
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
) -> tonutils::tlb::Result<()> {
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
) -> tonutils::tlb::Result<(Option<OutList>, Option<WalletV5R1ExtendedActionList>)> {
    let out_list = if slice.load_bit()? {
        Some(tonutils::tlb::load_ref_tlb(
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
