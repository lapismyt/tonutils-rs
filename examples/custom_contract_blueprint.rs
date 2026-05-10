use tonutils::Contract;
use tonutils::contracts::ContractBlueprint;
use tonutils::tlb::{Result as TlbResult, TlbDeserialize, TlbSerialize};
use tonutils::tvm::{Builder, Slice};

const EMPTY_CODE_BOC: &[u8] = &[
    0xb5, 0xee, 0x9c, 0x72, 0x01, 0x01, 0x01, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00,
];

#[derive(Debug, Clone)]
struct WalletData {
    seqno: u32,
    subwallet_id: u32,
    public_key: [u8; 32],
}

impl TlbSerialize for WalletData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_uint::<u32>(self.seqno)?;
        builder.store_uint::<u32>(self.subwallet_id)?;
        builder.store_bytes(&self.public_key)?;
        Ok(())
    }
}

impl TlbDeserialize for WalletData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        let seqno = slice.load_uint::<u32>()?;
        let subwallet_id = slice.load_uint::<u32>()?;
        let mut public_key = [0; 32];
        public_key.copy_from_slice(&slice.load_bytes(32)?);
        Ok(Self {
            seqno,
            subwallet_id,
            public_key,
        })
    }
}

#[derive(Debug, Clone, Contract)]
#[contract(code = EMPTY_CODE_BOC, workchain = 0)]
struct WalletFromConst {
    data: WalletData,
}

#[derive(Debug, Clone, Contract)]
#[contract(code_hex = "b5ee9c72010101010002000000")]
struct WalletFromHex {
    data: WalletData,
}

#[derive(Debug, Clone, Contract)]
#[contract(code_file = "custom_contract_blueprint.rs")]
struct WalletFromFile {
    data: WalletData,
}

fn main() -> anyhow::Result<()> {
    let wallet = WalletFromConst {
        data: WalletData {
            seqno: 0,
            subwallet_id: 698_983_191,
            public_key: [0; 32],
        },
    };
    let hex_wallet = WalletFromHex {
        data: wallet.data.clone(),
    };
    let decoded = WalletData::from_cell(wallet.data.to_cell()?)?;

    println!("const_address={}", wallet.address()?.to_base64());
    println!("hex_address={}", hex_wallet.address()?.to_base64());
    println!("decoded_seqno={}", decoded.seqno);
    let _file_style = WalletFromFile { data: wallet.data };
    Ok(())
}
