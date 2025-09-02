use std::str::FromStr;

use crate::models::toncenter_v3::Message as MessageTC3;
use crate::models::toncenter_v3::Transaction as TransactionTC3;
use crate::models::toncenter_v3::MessageContent as MessageContentTC3;
use crate::models::basic::{AccountInfo, Message, Transaction};
use num_bigint::BigUint;
use tonlib_core::tlb_types::block::state_init::StateInit;
use tonlib_core::cell::{ArcCell, Cell};
use tonlib_core::tlb_types::block::msg_address::MsgAddress;
use tonlib_core::tlb_types::tlb::TLB;
use tonlib_core::{TonAddress};


impl Into<Message> for MessageTC3 {
    fn into(self) -> Message {
        let created_at = match self.created_at {
            Some(created_at_str) => if let Ok(created_at) = created_at_str.parse::<u32>() {
                Some(created_at)
            } else {
                None
            },
            None => None,
        };
        let created_lt = match self.created_lt {
            Some(created_lt_str) => if let Ok(created_lt) = created_lt_str.parse::<BigUint>() {
                Some(created_lt)
            } else {
                None
            },
            None => None,
        };
        let destination = match self.destination {
            Some(destination_str) => if let Ok(destination) = TonAddress::from_str(&destination_str) {
                Some(destination)
            } else {
                None
            },
            None => None,
        };
        let destination = match destination {
            Some(destination) => Some(destination.to_msg_address()),
            None => None,
        };
        let hash = match self.hash {
            Some(hash_str) => Some(hash_str),
            None => None,
        };
        let init_state: Option<MessageContentTC3> = match self.init_state {
            Some(state_init) => Some(state_init.into()),
            None => None,
        };
        let init_state_body_str: Option<String> = if let Some(init_state_body_str) = &init_state {
            init_state_body_str.body.clone()
        } else {
            None
        };
        let init_state_body_cell: Option<Cell> = if let Some(init_state_body_str) = &init_state_body_str {
            if let Ok(init_state_body) = Cell::from_boc_b64(init_state_body_str) {
                Some(init_state_body)
            } else {
                None
            }
        } else {
            None
        };
        let state_init: Option<StateInit> = if let Some(init_state_body_cell) = &init_state_body_cell {
            if let Ok(state_init) = StateInit::from_cell(init_state_body_cell) {
                Some(state_init)
            } else {
                None
            }
        } else {
            None
        };
        let message_body: Option<String> = match self.message_content {
            Some(body) => body.body,
            None => None,
        };
        let body: Option<Cell> = if let Some(message_body) = &message_body {
            if let Ok(message_body) = Cell::from_boc_b64(message_body) {
                Some(message_body)
            } else {
                None
            }
        } else {
            None
        };
        let source_ton_addr: Option<TonAddress> = match self.source {
            Some(source) => if let Ok(source_msg_addr) = TonAddress::from_str(&source) {
                Some(source_msg_addr)
            } else {
                None
            },
            None => None,
        };
        let source: Option<MsgAddress> = match source_ton_addr {
            Some(source_ton_addr) => Some(source_ton_addr.to_msg_address()),
            None => None,
        };
        let value = match self.value {
            Some(value_str) => if let Ok(value) = BigUint::from_str(&value_str) {
                Some(value)
            } else {
                None
            },
            None => None,
        };

        Message {
            created_at,
            created_lt,
            destination,
            hash,
            state_init,
            body,
            source,
            value,
        }
    }
}


impl Into<Transaction> for TransactionTC3 {
    fn into(self) -> Transaction {
        let account = match self.account {
            Some(account_str) => if let Ok(account_addr) = TonAddress::from_str(&account_str) {
                Some(account_addr.to_msg_address())
            } else {
                None
            },
            None => None,
        };
        
        let in_msg = match self.in_msg {
            Some(in_msg_tc3) => Some(in_msg_tc3.into()),
            None => None,
        };
        
        let out_msgs = match self.out_msgs {
            Some(out_msgs_tc3) => {
                let converted_msgs: Vec<Message> = out_msgs_tc3
                    .into_iter()
                    .map(|msg| msg.into())
                    .collect();
                Some(converted_msgs)
            },
            None => None,
        };
        
        let lt = match self.lt {
            Some(lt_str) => if let Ok(lt) = BigUint::from_str(&lt_str) {
                Some(lt)
            } else {
                None
            },
            None => None,
        };
        
        let hash = self.hash;
        
        let now = self.now;
        
        Transaction {
            account,
            in_msg,
            out_msgs,
            lt,
            hash,
            now,
        }
    }
}
