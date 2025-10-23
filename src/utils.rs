use pretty_env_logger::formatted_builder;
use crate::crc::CRC16;

pub fn init_logger() -> Result<(), log::SetLoggerError> {
    let mut builder = formatted_builder();

    if let Ok(s) = ::std::env::var("RUST_LOG") {
        builder.parse_filters(&s);
    } else {
        builder.parse_filters("info");
    }

    builder.try_init()
}

pub fn method_name_to_id(name: &str) -> u16 {
    let method_value = CRC16.checksum(name.as_bytes()) as u32;
    let result = (method_value & 0xFFFF) | 0x10000;
    result as u16
}