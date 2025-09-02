use serde::{Deserialize, Serialize};
use num_bigint::BigUint;
use tonlib_core::cell::{ArcCell, Cell};
use tonlib_core::tlb_types::block::msg_address::MsgAddress;
use tonlib_core::tlb_types::primitives::either::{EitherRef, EitherRefLayout};
use tonlib_core::tlb_types::primitives::reference::Ref;
use tonlib_core::tlb_types::block::state_init::StateInit;


#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrInt {
    String(String),
    Int(i64),
}


#[derive(Debug, PartialEq, Clone)]
pub enum AccountStatus {
    Active,
    Uninit,
    Frozen,
    Nonexist,
}


#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    Desc
}


#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub(crate) address: Option<MsgAddress>,
    pub(crate) balance: Option<BigUint>,
    pub(crate) code_hash: Option<String>,
    pub(crate) data_hash: Option<String>,
    pub(crate) status: Option<AccountStatus>,
}


#[derive(Debug, Clone)]
pub struct Message {
    pub(crate) created_lt: Option<BigUint>,
    pub(crate) created_at: Option<u32>,
    pub(crate) destination: Option<MsgAddress>,
    pub(crate) hash: Option<String>,
    pub(crate) state_init: Option<StateInit>,
    pub(crate) body: Option<Cell>,
    pub(crate) source: Option<MsgAddress>,
    pub(crate) value: Option<BigUint>,
}


#[derive(Debug, Clone)]
pub struct Transaction {
    pub(crate) account: Option<MsgAddress>,
    pub(crate) in_msg: Option<Message>,
    pub(crate) out_msgs: Option<Vec<Message>>,
    pub(crate) lt: Option<BigUint>,
    pub(crate) hash: Option<String>,
    pub(crate) now: Option<u32>,
}