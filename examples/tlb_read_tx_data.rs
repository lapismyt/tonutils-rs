use tonutils::tlb::{
    AccountStatus, CurrencyCollection, Grams, HashUpdateAccount, TlbDeserialize, TlbSerialize,
    TrComputePhase, Transaction, TransactionDescr,
};
use tonutils::tvm::{HashmapE, base64_to_boc, boc_to_base64, hex_to_boc};

fn main() -> anyhow::Result<()> {
    let cell = match (
        std::env::var("TON_TRANSACTION_BOC_HEX").ok(),
        std::env::var("TON_TRANSACTION_BOC_BASE64").ok(),
    ) {
        (Some(hex), _) => hex_to_boc(&hex)?,
        (_, Some(base64)) => base64_to_boc(&base64)?,
        _ => fixture_transaction().to_cell()?,
    };

    let tx = Transaction::from_cell(cell.clone())?;
    println!(
        "lt={} account={} fee={} status={:?}->{:?} in_msg={} out_msgs={} hash={} boc_base64_len={}",
        tx.lt,
        hex::encode(tx.account_addr),
        tx.total_fees.grams.0,
        tx.orig_status,
        tx.end_status,
        tx.in_msg.is_some(),
        tx.out_msgs.len(),
        hex::encode(cell.hash()),
        boc_to_base64(&cell, false)?.len()
    );
    Ok(())
}

fn fixture_transaction() -> Transaction {
    Transaction {
        account_addr: [0x10; 32],
        lt: 7,
        prev_trans_hash: [0x20; 32],
        prev_trans_lt: 6,
        now: 1_700_000_000,
        outmsg_cnt: 0,
        orig_status: AccountStatus::Active,
        end_status: AccountStatus::Active,
        in_msg: None,
        out_msgs: HashmapE::new(15),
        total_fees: CurrencyCollection::grams(Grams::from(3_u64)),
        state_update: HashUpdateAccount {
            old_hash: [0xaa; 32],
            new_hash: [0xbb; 32],
        },
        description: TransactionDescr::Ordinary {
            credit_first: false,
            storage_ph: None,
            credit_ph: None,
            compute_ph: TrComputePhase::Skipped {
                reason: tonutils::tlb::ComputeSkipReason::NoState,
            },
            action: None,
            aborted: true,
            bounce: None,
            destroyed: false,
        },
    }
}
