use num_bigint::BigUint;
use tonutils::tlb::{
    Account, AccountState, AccountStorage, CurrencyCollection, Grams, MsgAddressInt, StateInit,
    StorageExtraInfo, StorageInfo, StorageUsed, TlbDeserialize, TlbSerialize,
};
use tonutils::tvm::Address;

fn main() -> anyhow::Result<()> {
    let address = Address::new(0, [0x42; 32]);
    let account = Account::Full {
        addr: MsgAddressInt::std(address.clone()),
        storage_stat: StorageInfo {
            used: StorageUsed::new(BigUint::from(1_u8), BigUint::from(5_u8)),
            last_paid: 1_700_000_000,
            due_payment: None,
            extra: StorageExtraInfo::None,
        },
        storage: AccountStorage {
            last_trans_lt: 11,
            balance: CurrencyCollection::grams(Grams::from(1_000_000_u64)),
            state: AccountState::Active {
                state_init: StateInit::empty(),
            },
        },
    };

    let cell = account.to_cell()?;
    let decoded = Account::from_cell(cell)?;

    let (state, balance) = match &decoded {
        Account::Full { storage, .. } => (&storage.state, &storage.balance.grams.0),
        Account::None => unreachable!("fixture uses a full account"),
    };

    println!(
        "addr_hash={} state={state:?} balance={balance}",
        hex::encode(address.hash_part)
    );

    Ok(())
}
