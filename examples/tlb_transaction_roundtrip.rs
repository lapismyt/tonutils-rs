use tonutils::tlb::{
    AccountStatus, CurrencyCollection, Grams, HashUpdateAccount, TlbDeserialize, TlbSerialize,
    TrComputePhase, Transaction, TransactionDescr,
};
use tonutils::tvm::HashmapE;

fn main() -> anyhow::Result<()> {
    let transaction = Transaction {
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
    };

    let cell = transaction.to_cell()?;
    let decoded = Transaction::from_cell(cell)?;

    println!(
        "lt={} statuses={:?}->{:?} fee={}",
        decoded.lt, decoded.orig_status, decoded.end_status, decoded.total_fees.grams.0
    );

    Ok(())
}
