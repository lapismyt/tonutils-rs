use super::*;

use crate::tlb::{
    Either, Result, TlbDeserialize, TlbError, TlbSerialize, ensure_empty, load_maybe, load_ref_tlb,
    load_var_uint, store_maybe, store_ref_tlb, store_tag, store_var_uint,
};
use crate::tvm::{Address, Builder, Cell, HashmapE, Slice};
use num_bigint::BigUint;
use std::sync::Arc;

pub(super) const EXTRA_CURRENCY_KEY_BITS: usize = 32;
pub(super) const STATE_INIT_LIBRARY_KEY_BITS: usize = 256;
pub(super) const VAR_UINT_16_LEN_BITS: usize = 4;
pub(super) const VAR_UINT_32_LEN_BITS: usize = 5;
pub(super) const VAR_UINT_7_LEN_BITS: usize = 3;
pub(super) const VAR_UINT_7_MAX_BYTES: usize = 6;
pub(super) const ACTION_SEND_MSG_TAG: u32 = 0x0ec3_c86d;
pub(super) const ACTION_SET_CODE_TAG: u32 = 0xad4d_e08e;
pub(super) const ACTION_RESERVE_CURRENCY_TAG: u32 = 0x36e6_b809;
pub(super) const ACTION_CHANGE_LIBRARY_TAG: u32 = 0x26fa_1dd4;
pub(super) const MAX_OUT_LIST_ACTIONS: usize = 255;

/// TL-B `anycast_info$_ depth:(#<= 30) { depth >= 1 } rewrite_pfx:(bits depth) = Anycast`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anycast {
    /// Rewrite prefix depth, encoded in five bits and constrained to `1..=30`.
    pub depth: u8,
    /// Raw rewrite prefix bits packed MSB-first.
    pub rewrite_pfx: Vec<u8>,
}

impl TlbSerialize for Anycast {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        validate_bit_string(
            "Anycast.rewrite_pfx",
            &self.rewrite_pfx,
            self.depth as usize,
        )?;
        if !(1..=30).contains(&self.depth) {
            return Err(TlbError::CustomSchema {
                schema: "Anycast",
                message: format!("depth {} is outside 1..=30", self.depth),
            });
        }
        builder.store_uint_custom::<u8>(self.depth as u8, 5)?;
        builder.store_bits(&self.rewrite_pfx, self.depth as usize)?;
        Ok(())
    }
}

impl TlbDeserialize for Anycast {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let depth = slice.load_uint_custom::<u8>(5)? as u8;
        if !(1..=30).contains(&depth) {
            return Err(TlbError::CustomSchema {
                schema: "Anycast",
                message: format!("depth {depth} is outside 1..=30"),
            });
        }
        let rewrite_pfx = slice.load_bits(depth as usize)?;
        Ok(Self { depth, rewrite_pfx })
    }
}

/// TL-B `MsgAddressInt`, preserving optional anycast and variable-length addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgAddressInt {
    /// `addr_std$10 anycast:(Maybe Anycast) workchain_id:int8 address:bits256`.
    Std {
        /// Optional anycast prefix.
        anycast: Option<Anycast>,
        /// Existing crate address model for the standard workchain and 256-bit hash.
        address: Address,
    },
    /// `addr_var$11 anycast:(Maybe Anycast) addr_len:(## 9) workchain_id:int32 address:(bits addr_len)`.
    Var {
        /// Optional anycast prefix.
        anycast: Option<Anycast>,
        /// Signed 32-bit workchain id.
        workchain_id: i32,
        /// Raw address bits packed MSB-first.
        address: Vec<u8>,
        /// Number of meaningful bits in `address`.
        bit_len: usize,
    },
}

impl MsgAddressInt {
    /// Creates a standard internal address without anycast.
    pub fn std(address: Address) -> Self {
        Self::Std {
            anycast: None,
            address,
        }
    }
}

impl TlbSerialize for MsgAddressInt {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Std { anycast, address } => {
                store_tag(builder, "10")?;
                store_maybe_anycast(builder, anycast)?;
                builder.store_int(address.workchain as i64, 8)?;
                builder.store_bytes(&address.hash_part)?;
            }
            Self::Var {
                anycast,
                workchain_id,
                address,
                bit_len,
            } => {
                validate_bit_string("MsgAddressInt.addr_var.address", address, *bit_len)?;
                if *bit_len > 511 {
                    return Err(TlbError::CustomSchema {
                        schema: "MsgAddressInt.addr_var",
                        message: format!("address bit length {bit_len} exceeds 511"),
                    });
                }
                store_tag(builder, "11")?;
                store_maybe_anycast(builder, anycast)?;
                builder.store_uint_custom::<u16>(*bit_len as u16, 9)?;
                builder.store_int(*workchain_id as i64, 32)?;
                builder.store_bits(address, *bit_len)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for MsgAddressInt {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        match load_two_bit_tag(slice, "MsgAddressInt", "10|11")?.as_str() {
            "10" => {
                let anycast = load_maybe_anycast(slice)?;
                let workchain = slice.load_int(8)? as i8;
                let mut hash_part = [0u8; 32];
                hash_part.copy_from_slice(&slice.load_bytes(32)?);
                Ok(Self::Std {
                    anycast,
                    address: Address::new(workchain, hash_part),
                })
            }
            "11" => {
                let anycast = load_maybe_anycast(slice)?;
                let bit_len = slice.load_uint_custom::<u16>(9)? as usize;
                let workchain_id = slice.load_int(32)? as i32;
                let address = slice.load_bits(bit_len)?;
                Ok(Self::Var {
                    anycast,
                    workchain_id,
                    address,
                    bit_len,
                })
            }
            actual_bits => Err(TlbError::TagMismatch {
                constructor: "MsgAddressInt",
                expected_bits: "10|11",
                actual_bits: actual_bits.to_string(),
            }),
        }
    }
}

/// TL-B `MsgAddressExt` with raw external-address bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgAddressExt {
    /// `addr_none$00`.
    None,
    /// `addr_extern$01 len:(## 9) external_address:(bits len)`.
    Extern {
        /// Raw address bits packed MSB-first.
        data: Vec<u8>,
        /// Number of meaningful bits in `data`.
        bit_len: usize,
    },
}

impl TlbSerialize for MsgAddressExt {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::None => store_tag(builder, "00")?,
            Self::Extern { data, bit_len } => {
                validate_bit_string("MsgAddressExt.addr_extern.external_address", data, *bit_len)?;
                if *bit_len > 511 {
                    return Err(TlbError::CustomSchema {
                        schema: "MsgAddressExt.addr_extern",
                        message: format!("address bit length {bit_len} exceeds 511"),
                    });
                }
                store_tag(builder, "01")?;
                builder.store_uint_custom::<u16>(*bit_len as u16, 9)?;
                builder.store_bits(data, *bit_len)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for MsgAddressExt {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        match load_two_bit_tag(slice, "MsgAddressExt", "00|01")?.as_str() {
            "00" => Ok(Self::None),
            "01" => {
                let bit_len = slice.load_uint_custom::<u16>(9)? as usize;
                let data = slice.load_bits(bit_len)?;
                Ok(Self::Extern { data, bit_len })
            }
            actual_bits => Err(TlbError::TagMismatch {
                constructor: "MsgAddressExt",
                expected_bits: "00|01",
                actual_bits: actual_bits.to_string(),
            }),
        }
    }
}

/// TL-B `MsgAddress`, wrapping either internal or external address constructors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgAddress {
    /// Anonymous constructor `_ _:MsgAddressInt = MsgAddress`.
    Int(MsgAddressInt),
    /// Anonymous constructor `_ _:MsgAddressExt = MsgAddress`.
    Ext(MsgAddressExt),
}

impl TlbSerialize for MsgAddress {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Int(address) => address.store_tlb(builder),
            Self::Ext(address) => address.store_tlb(builder),
        }
    }
}

impl TlbDeserialize for MsgAddress {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        match load_two_bit_tag(slice, "MsgAddress", "00|01|10|11")?.as_str() {
            "00" => Ok(Self::Ext(MsgAddressExt::None)),
            "01" => {
                let bit_len = slice.load_uint_custom::<u16>(9)? as usize;
                let data = slice.load_bits(bit_len)?;
                Ok(Self::Ext(MsgAddressExt::Extern { data, bit_len }))
            }
            "10" => {
                let anycast = load_maybe_anycast(slice)?;
                let workchain = slice.load_int(8)? as i8;
                let mut hash_part = [0u8; 32];
                hash_part.copy_from_slice(&slice.load_bytes(32)?);
                Ok(Self::Int(MsgAddressInt::Std {
                    anycast,
                    address: Address::new(workchain, hash_part),
                }))
            }
            "11" => {
                let anycast = load_maybe_anycast(slice)?;
                let bit_len = slice.load_uint_custom::<u16>(9)? as usize;
                let workchain_id = slice.load_int(32)? as i32;
                let address = slice.load_bits(bit_len)?;
                Ok(Self::Int(MsgAddressInt::Var {
                    anycast,
                    workchain_id,
                    address,
                    bit_len,
                }))
            }
            actual_bits => Err(TlbError::TagMismatch {
                constructor: "MsgAddress",
                expected_bits: "00|01|10|11",
                actual_bits: actual_bits.to_string(),
            }),
        }
    }
}

/// TL-B `nanograms$_ amount:(VarUInteger 16) = Grams`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Grams(pub BigUint);

impl From<u64> for Grams {
    fn from(value: u64) -> Self {
        Self(BigUint::from(value))
    }
}

impl TlbSerialize for Grams {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_var_uint(builder, &self.0, VAR_UINT_16_LEN_BITS)
    }
}

impl TlbDeserialize for Grams {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self(load_var_uint(slice, VAR_UINT_16_LEN_BITS)?))
    }
}

/// TL-B `CurrencyCollection`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyCollection {
    /// Native TON amount.
    pub grams: Grams,
    /// Extra currencies, keyed by 32-bit currency id and encoded as `VarUInteger 32`.
    pub other: HashmapE<BigUint>,
}

impl CurrencyCollection {
    /// Creates a collection with no extra currencies.
    pub fn grams(grams: Grams) -> Self {
        Self {
            grams,
            other: HashmapE::new(EXTRA_CURRENCY_KEY_BITS),
        }
    }
}

impl TlbSerialize for CurrencyCollection {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        if self.other.key_bits() != EXTRA_CURRENCY_KEY_BITS {
            return Err(TlbError::CustomSchema {
                schema: "CurrencyCollection",
                message: format!(
                    "extra-currency dictionary key width {} is not 32",
                    self.other.key_bits()
                ),
            });
        }
        self.grams.store_tlb(builder)?;
        builder.store_hashmap_e_with(&self.other, |builder, value| {
            builder.store_var_big_uint(value, VAR_UINT_32_LEN_BITS)?;
            Ok(())
        })?;
        Ok(())
    }
}

impl TlbDeserialize for CurrencyCollection {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let grams = Grams::load_tlb(slice)?;
        let other = slice.load_hashmap_e_with(EXTRA_CURRENCY_KEY_BITS, |slice| {
            load_var_uint(slice, VAR_UINT_32_LEN_BITS).map_err(anyhow::Error::from)
        })?;
        Ok(Self { grams, other })
    }
}

/// TL-B `tick_tock$_ tick:Bool tock:Bool = TickTock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickTock {
    /// Whether tick transactions are enabled.
    pub tick: bool,
    /// Whether tock transactions are enabled.
    pub tock: bool,
}

impl TlbSerialize for TickTock {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_bit(self.tick)?;
        builder.store_bit(self.tock)?;
        Ok(())
    }
}

impl TlbDeserialize for TickTock {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            tick: slice.load_bit()?,
            tock: slice.load_bit()?,
        })
    }
}

/// Current upstream TL-B `StateInit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateInit {
    /// `fixed_prefix_length:(Maybe (## 5))`.
    pub fixed_prefix_length: Option<u8>,
    /// Optional tick/tock special behavior.
    pub special: Option<TickTock>,
    /// Optional referenced code cell.
    pub code: Option<Arc<Cell>>,
    /// Optional referenced data cell.
    pub data: Option<Arc<Cell>>,
    /// Optional referenced library cell.
    pub library: Option<Arc<Cell>>,
}

impl StateInit {
    /// Creates an empty state init.
    pub fn empty() -> Self {
        Self {
            fixed_prefix_length: None,
            special: None,
            code: None,
            data: None,
            library: None,
        }
    }
}

impl TlbSerialize for StateInit {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self.fixed_prefix_length {
            Some(value) => {
                if value >= 32 {
                    return Err(TlbError::CustomSchema {
                        schema: "StateInit.fixed_prefix_length",
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
        store_maybe_tick_tock(builder, &self.special)?;
        builder.store_maybe_ref(self.code.clone())?;
        builder.store_maybe_ref(self.data.clone())?;
        builder.store_maybe_ref(self.library.clone())?;
        Ok(())
    }
}

impl TlbDeserialize for StateInit {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let fixed_prefix_length = if slice.load_bit()? {
            Some(slice.load_uint_custom::<u8>(5)? as u8)
        } else {
            None
        };
        Ok(Self {
            fixed_prefix_length,
            special: load_maybe_tick_tock(slice)?,
            code: load_maybe_ref_cell(slice)?,
            data: load_maybe_ref_cell(slice)?,
            library: load_maybe_ref_cell(slice)?,
        })
    }
}

/// TL-B `simple_lib$_ public:Bool root:^Cell = SimpleLib`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleLib {
    /// Whether the library is public.
    pub public: bool,
    /// Referenced library root cell.
    pub root: Arc<Cell>,
}

impl TlbSerialize for SimpleLib {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_bit(self.public)?;
        builder.store_ref(self.root.clone())?;
        Ok(())
    }
}

impl TlbDeserialize for SimpleLib {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            public: slice.load_bit()?,
            root: slice.load_reference()?,
        })
    }
}

/// Upstream TL-B `StateInitWithLibs`, with shared libraries in `HashmapE 256 SimpleLib`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateInitWithLibs {
    /// `fixed_prefix_length:(Maybe (## 5))`.
    pub fixed_prefix_length: Option<u8>,
    /// Optional tick/tock special behavior.
    pub special: Option<TickTock>,
    /// Optional referenced code cell.
    pub code: Option<Arc<Cell>>,
    /// Optional referenced data cell.
    pub data: Option<Arc<Cell>>,
    /// Library dictionary keyed by 256-bit library hash.
    pub library: HashmapE<SimpleLib>,
}

impl StateInitWithLibs {
    /// Creates an empty state init with no library entries.
    pub fn empty() -> Self {
        Self {
            fixed_prefix_length: None,
            special: None,
            code: None,
            data: None,
            library: HashmapE::new(STATE_INIT_LIBRARY_KEY_BITS),
        }
    }
}

impl TlbSerialize for StateInitWithLibs {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_state_init_prefix(
            builder,
            self.fixed_prefix_length,
            &self.special,
            &self.code,
            &self.data,
            "StateInitWithLibs.fixed_prefix_length",
        )?;
        if self.library.key_bits() != STATE_INIT_LIBRARY_KEY_BITS {
            return Err(TlbError::CustomSchema {
                schema: "StateInitWithLibs.library",
                message: format!(
                    "library dictionary key width {} is not 256",
                    self.library.key_bits()
                ),
            });
        }
        builder.store_hashmap_e_with(&self.library, |builder, value| {
            value.store_tlb(builder)?;
            Ok(())
        })?;
        Ok(())
    }
}

impl TlbDeserialize for StateInitWithLibs {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let (fixed_prefix_length, special, code, data) = load_state_init_prefix(slice)?;
        let library = slice.load_hashmap_e_with(STATE_INIT_LIBRARY_KEY_BITS, |slice| {
            SimpleLib::load_tlb(slice).map_err(anyhow::Error::from)
        })?;
        Ok(Self {
            fixed_prefix_length,
            special,
            code,
            data,
            library,
        })
    }
}

/// TL-B `CommonMsgInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommonMsgInfo {
    /// `int_msg_info$0`.
    Internal {
        /// Whether IHR is disabled.
        ihr_disabled: bool,
        /// Whether the message should bounce.
        bounce: bool,
        /// Whether this is a bounced message.
        bounced: bool,
        /// Internal source address.
        src: MsgAddressInt,
        /// Internal destination address.
        dest: MsgAddressInt,
        /// Transferred value.
        value: CurrencyCollection,
        /// Current upstream extra flags, encoded as `VarUInteger 16`.
        extra_flags: BigUint,
        /// Forwarding fee.
        fwd_fee: Grams,
        /// Creation logical time.
        created_lt: u64,
        /// Creation unix time.
        created_at: u32,
    },
    /// `ext_in_msg_info$10`.
    ExternalIn {
        /// External source address.
        src: MsgAddressExt,
        /// Internal destination address.
        dest: MsgAddressInt,
        /// Import fee.
        import_fee: Grams,
    },
    /// `ext_out_msg_info$11`.
    ExternalOut {
        /// Internal source address.
        src: MsgAddressInt,
        /// External destination address.
        dest: MsgAddressExt,
        /// Creation logical time.
        created_lt: u64,
        /// Creation unix time.
        created_at: u32,
    },
}

impl TlbSerialize for CommonMsgInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Internal {
                ihr_disabled,
                bounce,
                bounced,
                src,
                dest,
                value,
                extra_flags,
                fwd_fee,
                created_lt,
                created_at,
            } => {
                store_tag(builder, "0")?;
                builder.store_bit(*ihr_disabled)?;
                builder.store_bit(*bounce)?;
                builder.store_bit(*bounced)?;
                src.store_tlb(builder)?;
                dest.store_tlb(builder)?;
                value.store_tlb(builder)?;
                store_var_uint(builder, extra_flags, VAR_UINT_16_LEN_BITS)?;
                fwd_fee.store_tlb(builder)?;
                builder.store_u64(*created_lt)?;
                builder.store_u32(*created_at)?;
            }
            Self::ExternalIn {
                src,
                dest,
                import_fee,
            } => {
                store_tag(builder, "10")?;
                src.store_tlb(builder)?;
                dest.store_tlb(builder)?;
                import_fee.store_tlb(builder)?;
            }
            Self::ExternalOut {
                src,
                dest,
                created_lt,
                created_at,
            } => {
                store_tag(builder, "11")?;
                src.store_tlb(builder)?;
                dest.store_tlb(builder)?;
                builder.store_u64(*created_lt)?;
                builder.store_u32(*created_at)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for CommonMsgInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let first = slice.load_bit()?;
        if !first {
            return Ok(Self::Internal {
                ihr_disabled: slice.load_bit()?,
                bounce: slice.load_bit()?,
                bounced: slice.load_bit()?,
                src: MsgAddressInt::load_tlb(slice)?,
                dest: MsgAddressInt::load_tlb(slice)?,
                value: CurrencyCollection::load_tlb(slice)?,
                extra_flags: load_var_uint(slice, VAR_UINT_16_LEN_BITS)?,
                fwd_fee: Grams::load_tlb(slice)?,
                created_lt: slice.load_u64()?,
                created_at: slice.load_u32()?,
            });
        }

        if slice.load_bit()? {
            Ok(Self::ExternalOut {
                src: MsgAddressInt::load_tlb(slice)?,
                dest: MsgAddressExt::load_tlb(slice)?,
                created_lt: slice.load_u64()?,
                created_at: slice.load_u32()?,
            })
        } else {
            Ok(Self::ExternalIn {
                src: MsgAddressExt::load_tlb(slice)?,
                dest: MsgAddressInt::load_tlb(slice)?,
                import_fee: Grams::load_tlb(slice)?,
            })
        }
    }
}

/// TL-B `CommonMsgInfoRelaxed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommonMsgInfoRelaxed {
    /// `int_msg_info$0`.
    Internal {
        /// Whether IHR is disabled.
        ihr_disabled: bool,
        /// Whether the message should bounce.
        bounce: bool,
        /// Whether this is a bounced message.
        bounced: bool,
        /// Relaxed source address, allowing either internal or external address forms.
        src: MsgAddress,
        /// Internal destination address.
        dest: MsgAddressInt,
        /// Transferred value.
        value: CurrencyCollection,
        /// Current upstream extra flags, encoded as `VarUInteger 16`.
        extra_flags: BigUint,
        /// Forwarding fee.
        fwd_fee: Grams,
        /// Creation logical time.
        created_lt: u64,
        /// Creation unix time.
        created_at: u32,
    },
    /// `ext_out_msg_info$11`.
    ExternalOut {
        /// Relaxed source address, allowing either internal or external address forms.
        src: MsgAddress,
        /// External destination address.
        dest: MsgAddressExt,
        /// Creation logical time.
        created_lt: u64,
        /// Creation unix time.
        created_at: u32,
    },
}

impl TlbSerialize for CommonMsgInfoRelaxed {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Internal {
                ihr_disabled,
                bounce,
                bounced,
                src,
                dest,
                value,
                extra_flags,
                fwd_fee,
                created_lt,
                created_at,
            } => {
                store_tag(builder, "0")?;
                builder.store_bit(*ihr_disabled)?;
                builder.store_bit(*bounce)?;
                builder.store_bit(*bounced)?;
                src.store_tlb(builder)?;
                dest.store_tlb(builder)?;
                value.store_tlb(builder)?;
                store_var_uint(builder, extra_flags, VAR_UINT_16_LEN_BITS)?;
                fwd_fee.store_tlb(builder)?;
                builder.store_u64(*created_lt)?;
                builder.store_u32(*created_at)?;
            }
            Self::ExternalOut {
                src,
                dest,
                created_lt,
                created_at,
            } => {
                store_tag(builder, "11")?;
                src.store_tlb(builder)?;
                dest.store_tlb(builder)?;
                builder.store_u64(*created_lt)?;
                builder.store_u32(*created_at)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for CommonMsgInfoRelaxed {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let first = slice.load_bit()?;
        if !first {
            return Ok(Self::Internal {
                ihr_disabled: slice.load_bit()?,
                bounce: slice.load_bit()?,
                bounced: slice.load_bit()?,
                src: MsgAddress::load_tlb(slice)?,
                dest: MsgAddressInt::load_tlb(slice)?,
                value: CurrencyCollection::load_tlb(slice)?,
                extra_flags: load_var_uint(slice, VAR_UINT_16_LEN_BITS)?,
                fwd_fee: Grams::load_tlb(slice)?,
                created_lt: slice.load_u64()?,
                created_at: slice.load_u32()?,
            });
        }

        if slice.load_bit()? {
            Ok(Self::ExternalOut {
                src: MsgAddress::load_tlb(slice)?,
                dest: MsgAddressExt::load_tlb(slice)?,
                created_lt: slice.load_u64()?,
                created_at: slice.load_u32()?,
            })
        } else {
            Err(TlbError::TagMismatch {
                constructor: "CommonMsgInfoRelaxed",
                expected_bits: "0|11",
                actual_bits: "10".to_string(),
            })
        }
    }
}

/// Hand-written TL-B `Message Any`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Common routing and fee metadata.
    pub info: CommonMsgInfo,
    /// Optional state init, preserving inline (`Left`) versus referenced (`Right`) placement.
    pub init: Option<Either<StateInit, StateInit>>,
    /// Message body, preserving inline (`Left`) versus referenced (`Right`) placement.
    pub body: Either<Arc<Cell>, Arc<Cell>>,
}

impl TlbSerialize for Message {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.info.store_tlb(builder)?;
        store_message_init(builder, &self.init)?;
        store_message_body(builder, &self.body)?;
        Ok(())
    }
}

impl TlbDeserialize for Message {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let info = CommonMsgInfo::load_tlb(slice)?;
        let init = load_message_init(slice)?;
        let body = load_message_body(slice)?;
        Ok(Self { info, init, body })
    }
}

/// Hand-written TL-B `MessageRelaxed Any`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRelaxed {
    /// Relaxed routing and fee metadata.
    pub info: CommonMsgInfoRelaxed,
    /// Optional state init, preserving inline (`Left`) versus referenced (`Right`) placement.
    pub init: Option<Either<StateInit, StateInit>>,
    /// Message body, preserving inline (`Left`) versus referenced (`Right`) placement.
    pub body: Either<Arc<Cell>, Arc<Cell>>,
}

impl TlbSerialize for MessageRelaxed {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.info.store_tlb(builder)?;
        store_message_init(builder, &self.init)?;
        store_message_body(builder, &self.body)?;
        Ok(())
    }
}
