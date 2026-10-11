use derivative::Derivative;
use tl_proto::{TlRead, TlWrite};

use super::common::*;

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
#[tl(boxed)]
pub enum Message {
    /// adnl.message.createChannel key:int256 date:int = adnl.Message;
    #[tl(id = 0xe673c3bb)]
    CreateChannel { key: Int256, date: i32 },

    /// adnl.message.confirmChannel key:int256 peer_key:int256 date:int = adnl.Message;
    #[tl(id = 0x60dd1d69)]
    ConfirmChannel {
        key: Int256,
        peer_key: Int256,
        date: i32,
    },

    /// adnl.message.custom data:bytes = adnl.Message;
    #[tl(id = 0x204818f5)]
    Custom { data: Vec<u8> },

    /// adnl.message.nop = adnl.Message;
    #[tl(id = 0x17f8dfda)]
    Nop,

    /// adnl.message.reinit date:int = adnl.Message;
    #[tl(id = 0x10c20520)]
    Reinit { date: i32 },

    /// adnl.message.query query_id:int256 query:bytes = adnl.Message;
    #[tl(id = 0xb48bf97a)]
    Query { query_id: Int256, query: Vec<u8> },

    /// adnl.message.answer query_id:int256 answer:bytes = adnl.Message;
    #[tl(id = 0x0fac8416)]
    Answer { query_id: Int256, answer: Vec<u8> },

    /// adnl.message.part hash:int256 total_size:int offset:int data:bytes = adnl.Message;
    #[tl(id = 0xfd452d39)]
    Part {
        hash: Int256,
        total_size: i32,
        offset: i32,
        data: Vec<u8>,
    },

    /// tcp.ping random_id:long = tcp.Pong;
    #[tl(id = 0x4d082b9a)]
    Ping { random_id: u64 },

    /// tcp.pong random_id:long = tcp.Pong;
    #[tl(id = 0xdc69fb03)]
    Pong { random_id: u64 },
}
