
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Tiny {
        value: u8,
    }

    impl TlbSerialize for Tiny {
        fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
            store_tag(builder, "101")?;
            builder.store_uint_custom::<u8>(self.value as u8, 5)?;
            Ok(())
        }
    }

    impl TlbDeserialize for Tiny {
        fn load_tlb(slice: &mut Slice) -> Result<Self> {
            expect_tag(slice, "tiny$101", "101")?;
            let value = slice.load_uint_custom::<u8>(5)? as u8;
            Ok(Self { value })
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct BitValue(bool);

    impl TlbSerialize for BitValue {
        fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
            builder.store_bit(self.0)?;
            Ok(())
        }
    }

    impl TlbDeserialize for BitValue {
        fn load_tlb(slice: &mut Slice) -> Result<Self> {
            Ok(Self(slice.load_bit()?))
        }
    }

    #[test]
    fn trait_roundtrip_for_hand_written_type() {
        let original = Tiny { value: 0b1_0110 };
        let cell = original.to_cell().unwrap();
        let decoded = Tiny::from_cell(cell).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn constructor_tag_success_and_mismatch() {
        let mut builder = Builder::new();
        store_tag(&mut builder, "10").unwrap();
        let mut slice = Slice::new(builder.build().unwrap());
        expect_tag(&mut slice, "ok$10", "10").unwrap();
        ensure_empty(&slice).unwrap();

        let mut builder = Builder::new();
        store_tag(&mut builder, "11").unwrap();
        let mut slice = Slice::new(builder.build().unwrap());
        let err = expect_tag(&mut slice, "bad$10", "10").unwrap_err();
        assert!(matches!(err, TlbError::TagMismatch { .. }));
    }

    #[test]
    fn exact_decode_rejects_trailing_bits_and_refs() {
        let mut builder = Builder::new();
        Tiny { value: 3 }.store_tlb(&mut builder).unwrap();
        builder.store_bit(true).unwrap();
        let err = Tiny::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));

        let child = Tiny { value: 1 }.to_cell().unwrap();
        let mut builder = Builder::new();
        Tiny { value: 3 }.store_tlb(&mut builder).unwrap();
        builder.store_ref(child).unwrap();
        let err = Tiny::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 0, refs: 1 }));
    }

    #[test]
    fn maybe_present_and_absent_paths() {
        let mut builder = Builder::new();
        store_maybe(&mut builder, &Some(BitValue(true))).unwrap();
        store_maybe::<BitValue>(&mut builder, &None).unwrap();

        let mut slice = Slice::new(builder.build().unwrap());
        assert_eq!(
            load_maybe::<BitValue>(&mut slice).unwrap(),
            Some(BitValue(true))
        );
        assert_eq!(load_maybe::<BitValue>(&mut slice).unwrap(), None);
        ensure_empty(&slice).unwrap();
    }

    #[test]
    fn either_left_and_right_paths() {
        let mut builder = Builder::new();
        store_either::<BitValue, Tiny>(&mut builder, &Either::Left(BitValue(false))).unwrap();
        store_either::<BitValue, Tiny>(&mut builder, &Either::Right(Tiny { value: 7 })).unwrap();

        let mut slice = Slice::new(builder.build().unwrap());
        assert_eq!(
            load_either::<BitValue, Tiny>(&mut slice).unwrap(),
            Either::Left(BitValue(false))
        );
        assert_eq!(
            load_either::<BitValue, Tiny>(&mut slice).unwrap(),
            Either::Right(Tiny { value: 7 })
        );
        ensure_empty(&slice).unwrap();
    }

    #[test]
    fn referenced_value_requires_child_slice_consumption() {
        let mut builder = Builder::new();
        store_ref_tlb(&mut builder, &Tiny { value: 9 }).unwrap();
        let mut slice = Slice::new(builder.build().unwrap());
        assert_eq!(
            load_ref_tlb::<Tiny>(&mut slice, "tiny_ref").unwrap(),
            Tiny { value: 9 }
        );
        ensure_empty(&slice).unwrap();

        let mut child = Builder::new();
        Tiny { value: 9 }.store_tlb(&mut child).unwrap();
        child.store_bit(false).unwrap();
        let mut parent = Builder::new();
        parent.store_ref(child.build().unwrap()).unwrap();
        let mut slice = Slice::new(parent.build().unwrap());
        let err = load_ref_tlb::<Tiny>(&mut slice, "tiny_ref").unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                source,
                ..
            } if matches!(*source, TlbError::TrailingData { bits: 1, refs: 0 })
        ));
    }

    #[test]
    fn var_uint_accepts_canonical_zero_and_non_zero() {
        let mut builder = Builder::new();
        store_var_uint(&mut builder, &BigUint::from(0u8), 4).unwrap();
        store_var_uint(&mut builder, &BigUint::from(0x1234u16), 4).unwrap();

        let mut slice = Slice::new(builder.build().unwrap());
        assert_eq!(load_var_uint(&mut slice, 4).unwrap(), BigUint::from(0u8));
        assert_eq!(
            load_var_uint(&mut slice, 4).unwrap(),
            BigUint::from(0x1234u16)
        );
        ensure_empty(&slice).unwrap();
    }

    #[test]
    fn var_uint_rejects_overlong_non_canonical_encoding() {
        let mut builder = Builder::new();
        builder.store_uint_custom::<u8>(2, 4).unwrap();
        builder.store_bytes(&[0, 1]).unwrap();

        let mut slice = Slice::new(builder.build().unwrap());
        let err = load_var_uint(&mut slice, 4).unwrap_err();
        assert!(matches!(err, TlbError::NonCanonicalValue { .. }));
    }
}
