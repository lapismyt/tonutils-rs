use crc::{Crc, CRC_16_IBM_SDLC, CRC_32_ISO_HDLC};

/// CRC16 implementation
pub const CRC16: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_SDLC);

/// CRC32 implementation
pub const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

#[cfg(test)]
mod tests;