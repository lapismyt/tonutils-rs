use tonutils::tvm::Address;
use tonutils::wallet::{
    MAINNET_GLOBAL_ID, TonMnemonic, WALLET_V4R2_DEFAULT_ID, WalletMessage, WalletV4R2, WalletV5R1,
    WalletV5R1WalletId, wallet_v4r2_code, wallet_v5r1_code,
};

fn main() -> anyhow::Result<()> {
    let mnemonic = TonMnemonic::from_phrase(
        "token holiday equip sell fragile blouse hammer worry health that pool eternal host alcohol list kit emotion tissue zone mail panic crack armed menu",
        None,
    )?;
    let public_key = mnemonic.public_key();

    let v4 = WalletV4R2::default(public_key, wallet_v4r2_code()?, 0);
    let v5_wallet_id = WalletV5R1WalletId::client(MAINNET_GLOBAL_ID, 0, 0, 0).pack()?;
    let v5 = WalletV5R1::new(public_key, v5_wallet_id, wallet_v5r1_code()?, 0);

    println!("v4r2: {}", v4.address()?.to_raw());
    println!("v5r1: {}", v5.address()?.to_raw());

    let transfer = WalletMessage::internal(
        Address::from_str("0:1111111111111111111111111111111111111111111111111111111111111111")?,
        1_000_000,
    )
    .with_mode(3);
    let boc = v4.build_external_message_boc(
        0,
        1_900_000_000,
        vec![transfer],
        mnemonic.signing_key(),
        true,
    )?;
    println!("v4r2_transfer_boc_hex: {}", hex::encode(boc));
    println!("v4r2_wallet_id: {}", WALLET_V4R2_DEFAULT_ID);
    Ok(())
}
