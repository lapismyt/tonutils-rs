use tl_proto::{TlRead, TlResult};

pub fn lossy_read<'tl, T: TlRead<'tl>>(packet: &'tl [u8]) -> TlResult<Option<T>> {
    let mut slice = packet;
    let result = T::read_from(&mut slice);
    if let Ok(x) = result {
        Ok(Some(x))
    } else {
        Ok(None)
    }
}

pub fn fmt_string(bytes: &[u8], f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
    write!(
        f,
        "{}",
        std::string::String::from_utf8(bytes.to_vec()).unwrap()
    )
}

pub fn fmt_bytes(bytes: &[u8], f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
    write!(f, "0x{}", hex::encode(bytes))
}

pub fn fmt_opt_bytes<T: AsRef<[u8]>>(
    bytes: &Option<T>,
    f: &mut std::fmt::Formatter,
) -> Result<(), std::fmt::Error> {
    if let Some(bytes) = bytes {
        write!(f, "Some(0x{})", hex::encode(bytes))
    } else {
        write!(f, "None")
    }
}

pub mod struct_as_bytes {
    use tl_proto::{TlPacket, TlRead, TlResult, TlWrite};

    pub fn size_hint<T: TlWrite>(v: &T) -> usize {
        tl_proto::serialize(v).len()
    }

    pub fn write<P: TlPacket, T: TlWrite>(v: &T, packet: &mut P) {
        tl_proto::serialize(v).write_to(packet)
    }

    pub fn read<'tl, T: TlRead<'tl>>(packet: &'tl [u8]) -> TlResult<T> {
        let mut slice = packet;
        <&'tl [u8]>::read_from(&mut slice).and_then(|x| tl_proto::deserialize(x))
    }
}
