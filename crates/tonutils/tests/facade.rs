use tonutils::{Address, Builder, CRC16, Cell, LiteQuery};

#[test]
fn exposes_runtime_modules() {
    let _: Option<tonutils::adnl::AdnlHandshake> = None;
    let _: Option<tonutils::contracts::ContractBuildError> = None;
    let _ = tonutils::crc::CRC16;
    let _: Option<fn(tonutils::metadata::Tep64Content) -> tonutils::jetton::JettonMetadata> =
        Some(tonutils::jetton::JettonMetadata::from_content);
    let _: Option<tonutils::liteclient::peer::LitePeer<()>> = None;
    let _: Option<tonutils::metadata::Tep64KnownKey> = None;
    let _: Option<tonutils::network_config::ConfigGlobal> = None;
    let _: Option<tonutils::nft::NftCollectionData> = None;
    let _: Option<tonutils::tl::LiteQuery> = None;
    let _: Option<tonutils::tlb::Message> = None;
    let _: Option<tonutils::tvm::Builder> = None;
    let _: Option<tonutils::wallet::WalletV4R2> = None;
}

#[test]
fn exposes_foundational_types_flatly() {
    let _: Option<Address> = None;
    let _: Option<Builder> = None;
    let _: Option<Cell> = None;
    let _: Option<LiteQuery> = None;
    let _ = CRC16;
    let _: Option<fn(&str) -> u64> = None;
}

#[cfg(any(
    feature = "jetton-provider",
    feature = "nft-provider",
    feature = "wallet-provider"
))]
#[test]
fn enables_provider_features() {
    #[cfg(feature = "jetton-provider")]
    #[allow(dead_code)]
    fn assert_jetton_provider<T: tonutils::jetton::JettonContractExt>() {}

    #[cfg(feature = "nft-provider")]
    #[allow(dead_code)]
    fn assert_nft_provider<T: tonutils::nft::NftContractExt>() {}

    let _: Option<tonutils::wallet::WalletV5R1> = None;
}
