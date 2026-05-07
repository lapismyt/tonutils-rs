//! Hand-written TL-B codecs for core blockchain message models.

use crate::tlb::{
    Either, Result, TlbDeserialize, TlbError, TlbSerialize, ensure_empty, load_maybe, load_ref_tlb,
    load_var_uint, store_maybe, store_ref_tlb, store_tag, store_var_uint,
};
use crate::tvm::{Address, Builder, Cell, HashmapE, Slice};
use num_bigint::BigUint;
use std::sync::Arc;

const EXTRA_CURRENCY_KEY_BITS: usize = 32;
const STATE_INIT_LIBRARY_KEY_BITS: usize = 256;
const VAR_UINT_16_LEN_BITS: usize = 4;
const VAR_UINT_32_LEN_BITS: usize = 5;
const VAR_UINT_7_LEN_BITS: usize = 3;
const VAR_UINT_7_MAX_BYTES: usize = 6;
const ACTION_SEND_MSG_TAG: u32 = 0x0ec3_c86d;
const ACTION_SET_CODE_TAG: u32 = 0xad4d_e08e;
const ACTION_RESERVE_CURRENCY_TAG: u32 = 0x36e6_b809;
const ACTION_CHANGE_LIBRARY_TAG: u32 = 0x26fa_1dd4;
const MAX_OUT_LIST_ACTIONS: usize = 255;

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
        builder.store_uint(self.depth as u64, 5)?;
        builder.store_bits(&self.rewrite_pfx, self.depth as usize)?;
        Ok(())
    }
}

impl TlbDeserialize for Anycast {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let depth = slice.load_uint(5)? as u8;
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
                builder.store_uint(*bit_len as u64, 9)?;
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
                let bit_len = slice.load_uint(9)? as usize;
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
                builder.store_uint(*bit_len as u64, 9)?;
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
                let bit_len = slice.load_uint(9)? as usize;
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
                let bit_len = slice.load_uint(9)? as usize;
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
                let bit_len = slice.load_uint(9)? as usize;
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
                builder.store_uint(value as u64, 5)?;
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
            Some(slice.load_uint(5)? as u8)
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
                builder.store_uint(ACTION_SEND_MSG_TAG as u64, 32)?;
                builder.store_uint(*mode as u64, 8)?;
                store_ref_tlb(builder, out_msg)?;
            }
            Self::SetCode { new_code } => {
                builder.store_uint(ACTION_SET_CODE_TAG as u64, 32)?;
                builder.store_ref(new_code.clone())?;
            }
            Self::ReserveCurrency { mode, currency } => {
                builder.store_uint(ACTION_RESERVE_CURRENCY_TAG as u64, 32)?;
                builder.store_uint(*mode as u64, 8)?;
                currency.store_tlb(builder)?;
            }
            Self::ChangeLibrary { mode, libref } => {
                if *mode > 0x7F {
                    return Err(TlbError::CustomSchema {
                        schema: "OutAction.action_change_library.mode",
                        message: format!("mode {mode} does not fit in seven bits"),
                    });
                }
                builder.store_uint(ACTION_CHANGE_LIBRARY_TAG as u64, 32)?;
                builder.store_uint(*mode as u64, 7)?;
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
                mode: slice.load_uint(8)? as u8,
                out_msg: load_ref_tlb(slice, "MessageRelaxed Any")?,
            }),
            ACTION_SET_CODE_TAG => Ok(Self::SetCode {
                new_code: slice.load_reference()?,
            }),
            ACTION_RESERVE_CURRENCY_TAG => Ok(Self::ReserveCurrency {
                mode: slice.load_uint(8)? as u8,
                currency: CurrencyCollection::load_tlb(slice)?,
            }),
            ACTION_CHANGE_LIBRARY_TAG => Ok(Self::ChangeLibrary {
                mode: slice.load_uint(7)? as u8,
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
        builder.store_uint(self.tot_actions as u64, 16)?;
        builder.store_uint(self.spec_actions as u64, 16)?;
        builder.store_uint(self.skipped_actions as u64, 16)?;
        builder.store_uint(self.msgs_created as u64, 16)?;
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
        let tot_actions = slice.load_uint(16)? as u16;
        let spec_actions = slice.load_uint(16)? as u16;
        let skipped_actions = slice.load_uint(16)? as u16;
        let msgs_created = slice.load_uint(16)? as u16;
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
            builder.store_uint(value as u64, 5)?;
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
        Some(slice.load_uint(5)? as u8)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tlb::{TlbSerialize, expect_tag};
    use crate::tvm::BitKey;

    fn roundtrip<T>(value: &T) -> T
    where
        T: TlbSerialize + TlbDeserialize + PartialEq + std::fmt::Debug,
    {
        T::from_cell(value.to_cell().unwrap()).unwrap()
    }

    fn cell_with_bits(data: &[u8], bit_len: usize) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_bits(data, bit_len).unwrap();
        builder.build().unwrap()
    }

    fn std_address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn ext_in_info() -> CommonMsgInfo {
        CommonMsgInfo::ExternalIn {
            src: MsgAddressExt::None,
            dest: MsgAddressInt::std(std_address(0x11)),
            import_fee: Grams::from(0),
        }
    }

    fn relaxed_internal_info(src: MsgAddress) -> CommonMsgInfoRelaxed {
        CommonMsgInfoRelaxed::Internal {
            ihr_disabled: true,
            bounce: false,
            bounced: false,
            src,
            dest: MsgAddressInt::std(std_address(0x22)),
            value: CurrencyCollection::grams(Grams::from(7)),
            extra_flags: BigUint::from(2u8),
            fwd_fee: Grams::from(3),
            created_lt: 4,
            created_at: 5,
        }
    }

    #[test]
    fn std_internal_address_roundtrips() {
        let value = MsgAddressInt::Std {
            anycast: None,
            address: Address::new(-1, [0xAA; 32]),
        };
        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn variable_internal_address_roundtrips() {
        let value = MsgAddressInt::Var {
            anycast: Some(Anycast {
                depth: 3,
                rewrite_pfx: vec![0b1010_0000],
            }),
            workchain_id: -239,
            address: vec![0b1101_0000],
            bit_len: 4,
        };
        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn external_addresses_roundtrip() {
        assert_eq!(roundtrip(&MsgAddressExt::None), MsgAddressExt::None);
        let raw = MsgAddressExt::Extern {
            data: vec![0b1010_0000],
            bit_len: 4,
        };
        assert_eq!(roundtrip(&raw), raw);
    }

    #[test]
    fn relaxed_msg_address_roundtrips_internal_and_external_forms() {
        let std = MsgAddress::Int(MsgAddressInt::std(std_address(0x10)));
        assert_eq!(roundtrip(&std), std);

        let var = MsgAddress::Int(MsgAddressInt::Var {
            anycast: None,
            workchain_id: -1,
            address: vec![0b1110_0000],
            bit_len: 3,
        });
        assert_eq!(roundtrip(&var), var);

        assert_eq!(
            roundtrip(&MsgAddress::Ext(MsgAddressExt::None)),
            MsgAddress::Ext(MsgAddressExt::None)
        );

        let raw = MsgAddress::Ext(MsgAddressExt::Extern {
            data: vec![0b0110_0000],
            bit_len: 3,
        });
        assert_eq!(roundtrip(&raw), raw);
    }

    #[test]
    fn malformed_anycast_depth_is_rejected() {
        for depth in [0u64, 31] {
            let mut builder = Builder::new();
            builder.store_uint(depth, 5).unwrap();
            if depth > 0 {
                builder
                    .store_bits(&vec![0; (depth as usize).div_ceil(8)], depth as usize)
                    .unwrap();
            }
            let err = Anycast::from_cell(builder.build().unwrap()).unwrap_err();
            assert!(matches!(
                err,
                TlbError::CustomSchema {
                    schema: "Anycast",
                    ..
                }
            ));
        }
    }

    #[test]
    fn grams_canonical_encodings_roundtrip() {
        assert_eq!(roundtrip(&Grams::from(0)), Grams::from(0));
        assert_eq!(
            roundtrip(&Grams::from(1_000_000_000)),
            Grams::from(1_000_000_000)
        );
    }

    #[test]
    fn currency_collection_roundtrips_empty_and_extra_currency() {
        let empty = CurrencyCollection::grams(Grams::from(123));
        assert_eq!(roundtrip(&empty), empty);

        let mut other = HashmapE::new(32);
        other
            .insert_bit_key(BitKey::from_u64(7, 32).unwrap(), BigUint::from(42u8))
            .unwrap();
        let value = CurrencyCollection {
            grams: Grams::from(1),
            other,
        };
        let decoded = roundtrip(&value);
        assert_eq!(
            decoded
                .other
                .get_bit_key(&BitKey::from_u64(7, 32).unwrap())
                .unwrap(),
            Some(&BigUint::from(42u8))
        );
        assert_eq!(decoded, value);
    }

    #[test]
    fn state_init_empty_roundtrips() {
        assert_eq!(roundtrip(&StateInit::empty()), StateInit::empty());
    }

    #[test]
    fn state_init_references_preserve_hashes() {
        let code = cell_with_bits(&[0xAA], 8);
        let data = cell_with_bits(&[0xBC], 6);
        let library = cell_with_bits(&[0xF0], 4);
        let value = StateInit {
            fixed_prefix_length: Some(5),
            special: Some(TickTock {
                tick: true,
                tock: false,
            }),
            code: Some(code.clone()),
            data: Some(data.clone()),
            library: Some(library.clone()),
        };
        let decoded = roundtrip(&value);
        assert_eq!(decoded.code.unwrap().hash(), code.hash());
        assert_eq!(decoded.data.unwrap().hash(), data.hash());
        assert_eq!(decoded.library.unwrap().hash(), library.hash());
    }

    #[test]
    fn common_msg_info_variants_roundtrip() {
        let internal = CommonMsgInfo::Internal {
            ihr_disabled: true,
            bounce: true,
            bounced: false,
            src: MsgAddressInt::std(std_address(0x01)),
            dest: MsgAddressInt::std(std_address(0x02)),
            value: CurrencyCollection::grams(Grams::from(100)),
            extra_flags: BigUint::from(3u8),
            fwd_fee: Grams::from(9),
            created_lt: 10,
            created_at: 11,
        };
        assert_eq!(roundtrip(&internal), internal);

        let ext_in = ext_in_info();
        assert_eq!(roundtrip(&ext_in), ext_in);

        let ext_out = CommonMsgInfo::ExternalOut {
            src: MsgAddressInt::std(std_address(0x33)),
            dest: MsgAddressExt::Extern {
                data: vec![0b1000_0000],
                bit_len: 1,
            },
            created_lt: 44,
            created_at: 55,
        };
        assert_eq!(roundtrip(&ext_out), ext_out);
    }

    #[test]
    fn tag_mismatch_failures_are_reported() {
        let mut builder = Builder::new();
        store_tag(&mut builder, "00").unwrap();
        let err = MsgAddressInt::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "MsgAddressInt",
                ..
            }
        ));

        let mut builder = Builder::new();
        store_tag(&mut builder, "11").unwrap();
        let err = MsgAddressExt::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "MsgAddressExt",
                ..
            }
        ));

        let mut builder = Builder::new();
        store_tag(&mut builder, "10").unwrap();
        let mut slice = Slice::new(builder.build().unwrap());
        expect_tag(&mut slice, "manual$11", "11").unwrap_err();
    }

    #[test]
    fn msg_address_truncated_tag_is_rejected() {
        let mut builder = Builder::new();
        builder.store_bit(true).unwrap();
        let err = MsgAddress::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "MsgAddress",
                actual_bits,
                ..
            } if actual_bits == "1"
        ));
    }

    #[test]
    fn simple_lib_and_state_init_with_libs_roundtrip() {
        assert_eq!(
            roundtrip(&StateInitWithLibs::empty()),
            StateInitWithLibs::empty()
        );

        let root = cell_with_bits(&[0xCE], 8);
        let lib = SimpleLib {
            public: true,
            root: root.clone(),
        };
        assert_eq!(roundtrip(&lib).root.hash(), root.hash());

        let mut library = HashmapE::new(256);
        library
            .insert_bit_key(BitKey::from_bits(vec![0xAB; 32], 256).unwrap(), lib)
            .unwrap();
        let value = StateInitWithLibs {
            fixed_prefix_length: Some(3),
            special: Some(TickTock {
                tick: false,
                tock: true,
            }),
            code: Some(cell_with_bits(&[0x11], 8)),
            data: Some(cell_with_bits(&[0x22], 8)),
            library,
        };
        let decoded = roundtrip(&value);
        let key = BitKey::from_bits(vec![0xAB; 32], 256).unwrap();
        let decoded_lib = decoded
            .library
            .get_bit_key(&key)
            .unwrap()
            .expect("library entry");
        assert_eq!(decoded_lib.root.hash(), root.hash());
        assert_eq!(decoded, value);
    }

    #[test]
    fn common_msg_info_relaxed_variants_roundtrip_and_reject_external_in() {
        let internal_src =
            relaxed_internal_info(MsgAddress::Int(MsgAddressInt::std(std_address(0x44))));
        assert_eq!(roundtrip(&internal_src), internal_src);

        let external_src = relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::Extern {
            data: vec![0b1010_0000],
            bit_len: 4,
        }));
        assert_eq!(roundtrip(&external_src), external_src);

        let ext_out = CommonMsgInfoRelaxed::ExternalOut {
            src: MsgAddress::Int(MsgAddressInt::std(std_address(0x55))),
            dest: MsgAddressExt::None,
            created_lt: 6,
            created_at: 7,
        };
        assert_eq!(roundtrip(&ext_out), ext_out);

        let mut builder = Builder::new();
        store_tag(&mut builder, "10").unwrap();
        let err = CommonMsgInfoRelaxed::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "CommonMsgInfoRelaxed",
                actual_bits,
                ..
            } if actual_bits == "10"
        ));
    }

    #[test]
    fn external_in_message_with_inline_empty_body_roundtrips() {
        let body = Builder::new().build().unwrap();
        let message = Message {
            info: ext_in_info(),
            init: None,
            body: Either::Left(body.clone()),
        };
        let decoded = roundtrip(&message);
        assert_eq!(decoded, message);
        assert_eq!(
            match decoded.body {
                Either::Left(cell) => cell.hash(),
                Either::Right(_) => [0; 32],
            },
            body.hash()
        );
    }

    #[test]
    fn message_with_referenced_state_init_roundtrips() {
        let init = StateInit {
            code: Some(cell_with_bits(&[0xAB], 8)),
            ..StateInit::empty()
        };
        let message = Message {
            info: ext_in_info(),
            init: Some(Either::Right(init.clone())),
            body: Either::Left(Builder::new().build().unwrap()),
        };
        assert_eq!(roundtrip(&message), message);
    }

    #[test]
    fn message_with_referenced_body_roundtrips() {
        let body = cell_with_bits(&[0xAB, 0xC0], 10);
        let message = Message {
            info: ext_in_info(),
            init: None,
            body: Either::Right(body.clone()),
        };
        let decoded = roundtrip(&message);
        assert_eq!(decoded, message);
        assert_eq!(
            match decoded.body {
                Either::Right(cell) => cell.hash(),
                Either::Left(_) => [0; 32],
            },
            body.hash()
        );
    }

    #[test]
    fn exact_message_decode_rejects_trailing_data_after_referenced_body() {
        let body = cell_with_bits(&[0xAB], 8);
        let mut builder = Builder::new();
        Message {
            info: ext_in_info(),
            init: None,
            body: Either::Right(body),
        }
        .store_tlb(&mut builder)
        .unwrap();
        builder.store_bit(true).unwrap();
        let err = Message::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));
    }

    #[test]
    fn relaxed_message_with_inline_empty_body_roundtrips() {
        let body = Builder::new().build().unwrap();
        let message = MessageRelaxed {
            info: relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::None)),
            init: None,
            body: Either::Left(body.clone()),
        };
        let decoded = roundtrip(&message);
        assert_eq!(decoded, message);
        assert_eq!(
            match decoded.body {
                Either::Left(cell) => cell.hash(),
                Either::Right(_) => [0; 32],
            },
            body.hash()
        );
    }

    #[test]
    fn relaxed_message_with_referenced_state_init_roundtrips() {
        let init = StateInit {
            data: Some(cell_with_bits(&[0xCD], 8)),
            ..StateInit::empty()
        };
        let message = MessageRelaxed {
            info: relaxed_internal_info(MsgAddress::Int(MsgAddressInt::std(std_address(0x66)))),
            init: Some(Either::Right(init.clone())),
            body: Either::Left(Builder::new().build().unwrap()),
        };
        assert_eq!(roundtrip(&message), message);
    }

    #[test]
    fn relaxed_message_with_referenced_body_roundtrips() {
        let body = cell_with_bits(&[0xAD, 0x80], 9);
        let message = MessageRelaxed {
            info: relaxed_internal_info(MsgAddress::Int(MsgAddressInt::std(std_address(0x77)))),
            init: None,
            body: Either::Right(body.clone()),
        };
        let decoded = roundtrip(&message);
        assert_eq!(decoded, message);
        assert_eq!(
            match decoded.body {
                Either::Right(cell) => cell.hash(),
                Either::Left(_) => [0; 32],
            },
            body.hash()
        );
    }

    #[test]
    fn exact_relaxed_message_decode_rejects_trailing_data_after_referenced_body() {
        let body = cell_with_bits(&[0xEF], 8);
        let mut builder = Builder::new();
        MessageRelaxed {
            info: relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::None)),
            init: None,
            body: Either::Right(body),
        }
        .store_tlb(&mut builder)
        .unwrap();
        builder.store_bit(false).unwrap();
        let err = MessageRelaxed::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));
    }

    #[test]
    fn out_action_send_msg_roundtrips_referenced_relaxed_message() {
        let body = cell_with_bits(&[0x42], 8);
        let out_msg = MessageRelaxed {
            info: relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::None)),
            init: None,
            body: Either::Right(body.clone()),
        };
        let action = OutAction::SendMsg {
            mode: 3,
            out_msg: out_msg.clone(),
        };

        let decoded = roundtrip(&action);
        assert_eq!(decoded, action);
        match decoded {
            OutAction::SendMsg { out_msg, .. } => match out_msg.body {
                Either::Right(decoded_body) => assert_eq!(decoded_body.hash(), body.hash()),
                Either::Left(_) => panic!("expected referenced body"),
            },
            _ => panic!("expected send message action"),
        }
    }

    #[test]
    fn out_action_set_code_preserves_cell_hash() {
        let code = cell_with_bits(&[0xAD, 0x80], 9);
        let action = OutAction::SetCode {
            new_code: code.clone(),
        };

        let decoded = roundtrip(&action);
        match decoded {
            OutAction::SetCode { new_code } => assert_eq!(new_code.hash(), code.hash()),
            _ => panic!("expected set code action"),
        }
    }

    #[test]
    fn out_action_reserve_currency_roundtrips_extra_currency_dictionary() {
        let mut other = HashmapE::new(32);
        other
            .insert_bit_key(
                BitKey::from_u64(0x1234_5678, 32).unwrap(),
                BigUint::from(9_999u16),
            )
            .unwrap();
        let currency = CurrencyCollection {
            grams: Grams::from(123),
            other,
        };
        let action = OutAction::ReserveCurrency {
            mode: 255,
            currency,
        };

        assert_eq!(roundtrip(&action), action);
    }

    #[test]
    fn out_action_change_library_roundtrips_hash_and_reference_forms() {
        let hash = [0x51; 32];
        let hash_action = OutAction::ChangeLibrary {
            mode: 127,
            libref: LibRef::Hash(hash),
        };
        assert_eq!(roundtrip(&hash_action), hash_action);

        let library = cell_with_bits(&[0xCE], 8);
        let ref_action = OutAction::ChangeLibrary {
            mode: 6,
            libref: LibRef::Ref(library.clone()),
        };
        let decoded = roundtrip(&ref_action);
        assert_eq!(decoded, ref_action);
        match decoded {
            OutAction::ChangeLibrary {
                libref: LibRef::Ref(decoded_library),
                ..
            } => assert_eq!(decoded_library.hash(), library.hash()),
            _ => panic!("expected change library reference action"),
        }
    }

    #[test]
    fn out_action_unknown_and_truncated_tags_are_rejected() {
        let mut builder = Builder::new();
        builder.store_uint(0xffff_ffff, 32).unwrap();
        let err = OutAction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "OutAction",
                actual_bits,
                ..
            } if actual_bits.len() == 32
        ));

        let mut builder = Builder::new();
        builder.store_bits(&[0x0e, 0xc0], 12).unwrap();
        let err = OutAction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "OutAction",
                actual_bits,
                ..
            } if actual_bits.len() == 12
        ));
    }

    #[test]
    fn libref_truncated_tag_is_rejected() {
        let err = LibRef::from_cell(Builder::new().build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "LibRef",
                actual_bits,
                ..
            } if actual_bits.is_empty()
        ));
    }

    #[test]
    fn change_library_mode_above_seven_bits_is_rejected() {
        let action = OutAction::ChangeLibrary {
            mode: 128,
            libref: LibRef::Hash([0; 32]),
        };
        let err = action.to_cell().unwrap_err();
        assert!(matches!(
            err,
            TlbError::CustomSchema {
                schema: "OutAction.action_change_library.mode",
                ..
            }
        ));
    }

    #[test]
    fn send_msg_invalid_referenced_payload_reports_reference_failure() {
        let mut invalid_message = Builder::new();
        store_tag(&mut invalid_message, "10").unwrap();
        let mut builder = Builder::new();
        builder.store_uint(ACTION_SEND_MSG_TAG as u64, 32).unwrap();
        builder.store_uint(0, 8).unwrap();
        builder.store_ref(invalid_message.build().unwrap()).unwrap();

        let err = OutAction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "MessageRelaxed Any",
                ..
            }
        ));
    }

    fn sample_send_action(mode: u8, body_byte: u8) -> OutAction {
        OutAction::SendMsg {
            mode,
            out_msg: MessageRelaxed {
                info: relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::None)),
                init: None,
                body: Either::Right(cell_with_bits(&[body_byte], 8)),
            },
        }
    }

    fn sample_set_code_action(byte: u8) -> OutAction {
        OutAction::SetCode {
            new_code: cell_with_bits(&[byte], 8),
        }
    }

    fn sample_reserve_action(mode: u8, grams: u64) -> OutAction {
        OutAction::ReserveCurrency {
            mode,
            currency: CurrencyCollection {
                grams: Grams::from(grams),
                other: HashmapE::new(32),
            },
        }
    }

    fn sample_change_library_action(mode: u8, byte: u8) -> OutAction {
        OutAction::ChangeLibrary {
            mode,
            libref: LibRef::Hash([byte; 32]),
        }
    }

    fn sample_action_phase() -> TrActionPhase {
        TrActionPhase {
            success: true,
            valid: true,
            no_funds: false,
            status_change: AccStatusChange::Unchanged,
            total_fwd_fees: None,
            total_action_fees: None,
            result_code: 0,
            result_arg: None,
            tot_actions: 0,
            spec_actions: 0,
            skipped_actions: 0,
            msgs_created: 0,
            action_list_hash: [0; 32],
            tot_msg_size: StorageUsed::new(BigUint::from(0u8), BigUint::from(0u8)),
        }
    }

    fn store_action_phase_prefix_through_hash(builder: &mut Builder) {
        builder.store_bit(true).unwrap();
        builder.store_bit(true).unwrap();
        builder.store_bit(false).unwrap();
        AccStatusChange::Unchanged.store_tlb(builder).unwrap();
        builder.store_bit(false).unwrap();
        builder.store_bit(false).unwrap();
        builder.store_int(0, 32).unwrap();
        builder.store_bit(false).unwrap();
        builder.store_uint(0, 16).unwrap();
        builder.store_uint(0, 16).unwrap();
        builder.store_uint(0, 16).unwrap();
        builder.store_uint(0, 16).unwrap();
        builder.store_bytes(&[0; 32]).unwrap();
    }

    fn contains_out_list_custom_schema(err: &TlbError) -> bool {
        match err {
            TlbError::CustomSchema {
                schema: "OutList", ..
            } => true,
            TlbError::InvalidReferencePayload { source, .. } => {
                contains_out_list_custom_schema(source)
            }
            _ => false,
        }
    }

    #[test]
    fn out_list_empty_roundtrips() {
        let list = OutList::default();
        let cell = list.to_cell().unwrap();

        assert_eq!(cell.bit_len(), 0);
        assert_eq!(cell.reference_count(), 0);
        assert_eq!(OutList::from_cell(cell).unwrap(), list);
    }

    #[test]
    fn out_list_single_action_roundtrips() {
        let list = OutList::new(vec![sample_send_action(1, 0xAA)]);

        assert_eq!(roundtrip(&list), list);
    }

    #[test]
    fn out_list_multi_action_roundtrip_preserves_order() {
        let list = OutList::new(vec![
            sample_set_code_action(0x10),
            sample_send_action(2, 0x20),
            sample_change_library_action(3, 0x30),
        ]);

        let decoded = roundtrip(&list);
        assert_eq!(decoded.actions, list.actions);
    }

    #[test]
    fn out_list_mixed_action_variants_roundtrip() {
        let list = OutList::new(vec![
            sample_send_action(4, 0x40),
            sample_set_code_action(0x50),
            sample_reserve_action(6, 7),
            sample_change_library_action(7, 0x80),
        ]);

        assert_eq!(roundtrip(&list), list);
    }

    #[test]
    fn out_list_serialization_rejects_more_than_255_actions() {
        let list = OutList::new(
            (0..=MAX_OUT_LIST_ACTIONS)
                .map(|idx| sample_set_code_action(idx as u8))
                .collect(),
        );

        let err = list.to_cell().unwrap_err();
        assert!(matches!(
            err,
            TlbError::CustomSchema {
                schema: "OutList",
                ..
            }
        ));
    }

    #[test]
    fn out_list_decode_rejects_more_than_255_nodes() {
        let mut current = Builder::new().build().unwrap();
        for idx in 0..=MAX_OUT_LIST_ACTIONS {
            let mut node = Builder::new();
            node.store_ref(current).unwrap();
            sample_set_code_action(idx as u8)
                .store_tlb(&mut node)
                .unwrap();
            current = node.build().unwrap();
        }

        let err = OutList::from_cell(current).unwrap_err();
        assert!(contains_out_list_custom_schema(&err));
    }

    #[test]
    fn out_list_non_empty_node_without_previous_ref_is_rejected() {
        let mut builder = Builder::new();
        builder.store_bit(true).unwrap();

        let err = OutList::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::CustomSchema {
                schema: "OutList",
                ..
            }
        ));
    }

    #[test]
    fn out_list_malformed_current_action_reports_action_decode_failure() {
        let mut builder = Builder::new();
        builder.store_ref(Builder::new().build().unwrap()).unwrap();
        builder.store_uint(0xffff_ffff, 32).unwrap();

        let err = OutList::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "OutAction",
                ..
            }
        ));
    }

    #[test]
    fn acc_status_change_variants_roundtrip() {
        assert_eq!(
            roundtrip(&AccStatusChange::Unchanged),
            AccStatusChange::Unchanged
        );
        assert_eq!(roundtrip(&AccStatusChange::Frozen), AccStatusChange::Frozen);
        assert_eq!(
            roundtrip(&AccStatusChange::Deleted),
            AccStatusChange::Deleted
        );
    }

    #[test]
    fn acc_status_change_truncated_tags_are_rejected() {
        let err = AccStatusChange::from_cell(Builder::new().build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "AccStatusChange",
                actual_bits,
                ..
            } if actual_bits.is_empty()
        ));

        let mut builder = Builder::new();
        builder.store_bit(true).unwrap();
        let err = AccStatusChange::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "AccStatusChange",
                actual_bits,
                ..
            } if actual_bits == "1"
        ));
    }

    #[test]
    fn storage_used_roundtrips_zero_and_non_zero_values() {
        let zero = StorageUsed::new(BigUint::from(0u8), BigUint::from(0u8));
        assert_eq!(roundtrip(&zero), zero);

        let non_zero = StorageUsed::new(BigUint::from(123u8), BigUint::from(65_535u32));
        assert_eq!(roundtrip(&non_zero), non_zero);
    }

    #[test]
    fn storage_used_rejects_non_canonical_varuint() {
        let mut builder = Builder::new();
        builder.store_uint(2, VAR_UINT_7_LEN_BITS).unwrap();
        builder.store_bytes(&[0, 1]).unwrap();
        builder.store_uint(0, VAR_UINT_7_LEN_BITS).unwrap();

        let err = StorageUsed::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::NonCanonicalValue { .. }));
    }

    #[test]
    fn storage_used_rejects_varuint_7_length_seven() {
        let value = StorageUsed::new(BigUint::from(1u64) << 48, BigUint::from(0u8));

        let err = value.to_cell().unwrap_err();
        assert!(matches!(
            err,
            TlbError::NonCanonicalValue {
                schema: "StorageUsed.cells",
                ..
            }
        ));

        let mut builder = Builder::new();
        builder.store_uint(7, VAR_UINT_7_LEN_BITS).unwrap();
        builder.store_bytes(&[1, 0, 0, 0, 0, 0, 0]).unwrap();
        builder.store_uint(0, VAR_UINT_7_LEN_BITS).unwrap();

        let err = StorageUsed::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::NonCanonicalValue {
                schema: "StorageUsed.cells",
                ..
            }
        ));
    }

    #[test]
    fn action_phase_roundtrips_without_optional_fields() {
        let value = sample_action_phase();

        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn action_phase_roundtrips_with_all_optional_fields_and_counters() {
        let value = TrActionPhase {
            success: false,
            valid: true,
            no_funds: true,
            status_change: AccStatusChange::Frozen,
            total_fwd_fees: Some(Grams::from(10_000)),
            total_action_fees: Some(Grams::from(20_000)),
            result_code: -14,
            result_arg: Some(32),
            tot_actions: 7,
            spec_actions: 1,
            skipped_actions: 2,
            msgs_created: 4,
            action_list_hash: [0xA5; 32],
            tot_msg_size: StorageUsed::new(BigUint::from(3u8), BigUint::from(777u16)),
        };

        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn action_phase_roundtrips_non_default_hash_and_message_size() {
        let mut action_list_hash = [0u8; 32];
        for (idx, byte) in action_list_hash.iter_mut().enumerate() {
            *byte = idx as u8;
        }
        let value = TrActionPhase {
            status_change: AccStatusChange::Deleted,
            result_code: 1,
            tot_actions: 255,
            spec_actions: 5,
            skipped_actions: 6,
            msgs_created: 250,
            action_list_hash,
            tot_msg_size: StorageUsed::new(BigUint::from(9u8), BigUint::from(1024u16)),
            ..sample_action_phase()
        };

        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn exact_action_phase_decode_rejects_trailing_data() {
        let value = sample_action_phase();
        let mut builder = Builder::new();
        value.store_tlb(&mut builder).unwrap();
        builder.store_bit(true).unwrap();
        let err = TrActionPhase::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));

        let mut builder = Builder::new();
        value.store_tlb(&mut builder).unwrap();
        builder.store_ref(Builder::new().build().unwrap()).unwrap();
        let err = TrActionPhase::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 0, refs: 1 }));
    }

    #[test]
    fn action_phase_malformed_optional_grams_propagates_error() {
        let mut builder = Builder::new();
        builder.store_bit(true).unwrap();
        builder.store_bit(true).unwrap();
        builder.store_bit(false).unwrap();
        AccStatusChange::Unchanged.store_tlb(&mut builder).unwrap();
        builder.store_bit(true).unwrap();
        builder.store_uint(2, VAR_UINT_16_LEN_BITS).unwrap();
        builder.store_bytes(&[0, 1]).unwrap();

        let err = TrActionPhase::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::NonCanonicalValue { .. }));
    }

    #[test]
    fn action_phase_malformed_storage_used_propagates_error() {
        let mut builder = Builder::new();
        store_action_phase_prefix_through_hash(&mut builder);
        builder.store_uint(2, VAR_UINT_7_LEN_BITS).unwrap();
        builder.store_bytes(&[0, 1]).unwrap();
        builder.store_uint(0, VAR_UINT_7_LEN_BITS).unwrap();

        let err = TrActionPhase::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::NonCanonicalValue { .. }));
    }
}
