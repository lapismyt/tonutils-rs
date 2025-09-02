use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::models::basic::StringOrInt;


#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum StackItemType {
    Num,
    Cell,
    Slice,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct V2StackEntity {
    pub(crate) r#type: Option<StackItemType>,
    pub(crate) value: Option<StringOrInt>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct V2RunGetMethodRequest {
    pub(crate) address: Option<String>,
    pub(crate) method: Option<String>,
    pub(crate) stack: Option<Vec<V2StackEntity>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct V2SendMessageRequest {
    pub(crate) boc: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct V2SendMessageResult {
    pub(crate) message_hash: Option<String>,
    pub(crate) message_hash_norm: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TokenInfo {
    pub(crate) description: Option<String>,
    pub(crate) extra: Option<HashMap<String, Value>>,
    pub(crate) image: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) nft_index: Option<String>,
    pub(crate) symbol: Option<String>,
    pub(crate) r#type: Option<String>,
    pub(crate) valid: Option<bool>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AddressMetadata {
    pub(crate) is_indexed: Option<bool>,
    pub(crate) token_info: Option<Vec<TokenInfo>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(flatten)]
    pub(crate) addresses: Option<HashMap<String, AddressMetadata>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AddressBookRow {
    pub(crate) domain: Option<String>,
    pub(crate) user_friendly: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AddressBook {
    #[serde(flatten)]
    pub(crate) addresses: Option<HashMap<String, AddressBookRow>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct WalletState {
    pub(crate) address: Option<String>,
    pub(crate) balance: Option<String>,
    pub(crate) code_hash: Option<String>,
    pub(crate) extra_currencies: Option<HashMap<String, String>>,
    pub(crate) is_signature_allowed: Option<bool>,
    pub(crate) is_wallet: Option<bool>,
    pub(crate) last_transaction_hash: Option<String>,
    pub(crate) last_transaction_lt: Option<String>,
    pub(crate) seqno: Option<u64>,
    pub(crate) status: Option<String>,
    pub(crate) wallet_id: Option<u64>,
    pub(crate) wallet_type: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct WalletStatesResponse {
    pub(crate) address_book: Option<AddressBook>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) wallets: Option<Vec<WalletState>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct V2AddressInformation {
    pub(crate) balance: Option<String>,
    pub(crate) code: Option<String>,
    pub(crate) data: Option<String>,
    pub(crate) frozen_hash: Option<String>,
    pub(crate) last_transaction_hash: Option<String>,
    pub(crate) last_transaction_lt: Option<String>,
    pub(crate) status: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MsgSize {
    pub(crate) bits: Option<String>,
    pub(crate) cells: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ActionPhase {
    pub(crate) action_list_hash: Option<String>,
    pub(crate) msgs_created: Option<u32>,
    pub(crate) no_funds: Option<bool>,
    pub(crate) result_arg: Option<u32>,
    pub(crate) result_code: Option<u32>,
    pub(crate) skipped_actions: Option<u32>,
    pub(crate) spec_actions: Option<u32>,
    pub(crate) status_change: Option<String>,
    pub(crate) success: Option<bool>,
    pub(crate) tot_actions: Option<u32>,
    pub(crate) tot_msg_size: Option<MsgSize>,
    pub(crate) total_action_fees: Option<String>,
    pub(crate) total_fwd_fees: Option<String>,
    pub(crate) valid: Option<bool>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct BouncePhase {
    pub(crate) fwd_fees: Option<String>,
    pub(crate) msg_fees: Option<String>,
    pub(crate) msg_size: Option<MsgSize>,
    pub(crate) req_fwd_fees: Option<String>,
    pub(crate) r#type: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ComputePhase {
    pub(crate) account_activated: Option<bool>,
    pub(crate) exit_arg: Option<u32>,
    pub(crate) exit_code: Option<u32>,
    pub(crate) gas_credit: Option<String>,
    pub(crate) gas_fees: Option<String>,
    pub(crate) gas_limit: Option<String>,
    pub(crate) gas_used: Option<String>,
    pub(crate) mode: Option<u32>,
    pub(crate) msg_state_used: Option<bool>,
    pub(crate) reason: Option<String>,
    pub(crate) skipped: Option<bool>,
    pub(crate) success: Option<bool>,
    pub(crate) vm_final_state_hash: Option<String>,
    pub(crate) vm_init_state_hash: Option<String>,
    pub(crate) vm_steps: Option<u32>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CreditPhase {
    pub(crate) credit: Option<String>,
    pub(crate) credit_extra_currencies: Option<HashMap<String, String>>,
    pub(crate) due_fees_collected: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct StoragePhase {
    pub(crate) status_change: Option<String>,
    pub(crate) storage_fees_collected: Option<String>,
    pub(crate) storage_fees_due: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SplitInfo {
    pub(crate) acc_split_depth: Option<u32>,
    pub(crate) cur_shard_pfx_len: Option<u32>,
    pub(crate) sibling_addr: Option<String>,
    pub(crate) this_addr: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct DecodedContent {
    pub(crate) comment: Option<String>,
    pub(crate) r#type: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MessageContent {
    pub(crate) body: Option<String>,
    pub(crate) decoded: Option<DecodedContent>,
    pub(crate) hash: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AccountState {
    pub(crate) account_status: Option<String>,
    pub(crate) balance: Option<String>,
    pub(crate) code_boc: Option<String>,
    pub(crate) code_hash: Option<String>,
    pub(crate) data_boc: Option<String>,
    pub(crate) data_hash: Option<String>,
    pub(crate) extra_currencies: Option<HashMap<String, String>>,
    pub(crate) frozen_hash: Option<String>,
    pub(crate) hash: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct BlockId {
    pub(crate) seqno: Option<u32>,
    pub(crate) shard: Option<String>,
    pub(crate) workchain: Option<u32>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionDescr {
    pub(crate) aborted: Option<bool>,
    pub(crate) action: Option<ActionPhase>,
    pub(crate) bounce: Option<BouncePhase>,
    pub(crate) compute_ph: Option<ComputePhase>,
    pub(crate) credit_first: Option<bool>,
    pub(crate) credit_ph: Option<CreditPhase>,
    pub(crate) destroyed: Option<bool>,
    pub(crate) installed: Option<bool>,
    pub(crate) is_tock: Option<bool>,
    pub(crate) split_info: Option<SplitInfo>,
    pub(crate) storage_ph: Option<StoragePhase>,
    pub(crate) r#type: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub(crate) bounce: Option<bool>,
    pub(crate) bounced: Option<bool>,
    pub(crate) created_at: Option<String>,
    pub(crate) created_lt: Option<String>,
    pub(crate) destination: Option<String>,
    pub(crate) fwd_fee: Option<String>,
    pub(crate) hash: Option<String>,
    pub(crate) hash_norm: Option<String>,
    pub(crate) ihr_disabled: Option<bool>,
    pub(crate) ihr_fee: Option<String>,
    pub(crate) import_fee: Option<String>,
    pub(crate) in_msg_tx_hash: Option<String>,
    pub(crate) init_state: Option<MessageContent>,
    pub(crate) message_content: Option<MessageContent>,
    pub(crate) opcode: Option<u32>,
    pub(crate) out_msg_tx_hash: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) value: Option<String>,
    pub(crate) value_extra_currencies: Option<HashMap<String, String>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub(crate) account: Option<String>,
    pub(crate) account_state_after: Option<AccountState>,
    pub(crate) account_state_before: Option<AccountState>,
    pub(crate) block_ref: Option<BlockId>,
    pub(crate) description: Option<TransactionDescr>,
    pub(crate) emulated: Option<bool>,
    pub(crate) end_status: Option<String>,
    pub(crate) hash: Option<String>,
    pub(crate) in_msg: Option<Message>,
    pub(crate) lt: Option<String>,
    pub(crate) mc_block_seqno: Option<u32>,
    pub(crate) now: Option<u32>,
    pub(crate) orig_status: Option<String>,
    pub(crate) out_msgs: Option<Vec<Message>>,
    pub(crate) prev_trans_hash: Option<String>,
    pub(crate) prev_trans_lt: Option<String>,
    pub(crate) total_fees: Option<String>,
    pub(crate) total_fees_extra_currencies: Option<HashMap<String, String>>,
    pub(crate) trace_external_hash: Option<String>,
    pub(crate) trace_id: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Action {
    pub(crate) accounts: Option<Vec<String>>,
    pub(crate) action_id: Option<String>,
    pub(crate) details: Option<HashMap<String, Value>>,
    pub(crate) end_lt: Option<String>,
    pub(crate) end_utime: Option<u32>,
    pub(crate) start_lt: Option<String>,
    pub(crate) start_utime: Option<u32>,
    pub(crate) success: Option<bool>,
    pub(crate) trace_end_lt: Option<String>,
    pub(crate) trace_end_utime: Option<u32>,
    pub(crate) trace_external_hash: Option<String>,
    pub(crate) trace_external_hash_norm: Option<String>,
    pub(crate) trace_id: Option<String>,
    pub(crate) trace_mc_seqno_end: Option<u32>,
    pub(crate) transactions: Option<Vec<String>>,
    pub(crate) r#type: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TraceMeta {
    pub(crate) classification_state: Option<String>,
    pub(crate) messages: Option<u32>,
    pub(crate) pending_messages: Option<u32>,
    pub(crate) trace_state: Option<String>,
    pub(crate) transactions: Option<u32>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TraceNode {
    pub(crate) children: Option<Vec<TraceNode>>,
    pub(crate) in_msg: Option<Message>,
    pub(crate) in_msg_hash: Option<String>,
    pub(crate) transaction: Option<Transaction>,
    pub(crate) tx_hash: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Trace {
    pub(crate) actions: Option<Vec<Action>>,
    pub(crate) end_lt: Option<String>,
    pub(crate) end_utime: Option<u32>,
    pub(crate) external_hash: Option<String>,
    pub(crate) is_incomplete: Option<bool>,
    pub(crate) mc_seqno_end: Option<String>,
    pub(crate) mc_seqno_start: Option<String>,
    pub(crate) start_lt: Option<String>,
    pub(crate) start_utime: Option<u32>,
    pub(crate) trace: Option<TraceNode>,
    pub(crate) trace_id: Option<String>,
    pub(crate) trace_info: Option<TraceMeta>,
    pub(crate) transactions: Option<Vec<Transaction>>,
    pub(crate) transactions_order: Option<Vec<String>>,
    pub(crate) warning: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TracesResponse {
    pub(crate) address_book: Option<AddressBook>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) traces: Option<Vec<Trace>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ActionsResponse {
    pub(crate) address_book: Option<AddressBook>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) actions: Option<Vec<Action>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    CallContract,
    ContractDeploy,
    TonTransfer,
    AuctionBid,
    ChangeDns,
    DexDepositLiquidity,
    DexWithdrawLiquidity,
    DeleteDns,
    RenewDns,
    ElectionDeposit,
    ElectionRecover,
    JettonBurn,
    JettonSwap,
    JettonTransfer,
    JettonMint,
    NftMint,
    TickTock,
    StakeDeposit,
    StakeWithdrawal,
    StakeWithdrawalRequest,
    Subscribe,
    Unsubscribe,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AccountStateFull {
    pub(crate) account_state_hash: Option<String>,
    pub(crate) address: Option<String>,
    pub(crate) balance: Option<String>,
    pub(crate) code_boc: Option<String>,
    pub(crate) code_hash: Option<String>,
    pub(crate) contract_methods: Option<Vec<u32>>,
    pub(crate) data_boc: Option<String>,
    pub(crate) data_hash: Option<String>,
    pub(crate) extra_currencies: Option<HashMap<String, String>>,
    pub(crate) frozen_hash: Option<String>,
    pub(crate) last_transaction_hash: Option<String>,
    pub(crate) last_transaction_lt: Option<String>,
    pub(crate) status: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AccountStatesResponse {
    pub(crate) address_book: Option<AddressBook>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) account_states: Option<Vec<AccountStateFull>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionsResponse {
    pub(crate) address_book: Option<AddressBook>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) transactions: Option<Vec<Transaction>>,
}


#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Active,
    Uninit,
    Frozen,
    Nonexist,
}
