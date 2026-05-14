use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_key_canonicalizes_and_orders() {
        assert!(BitKey::new(vec![0b1010_0001], 4).is_err());
        let key = BitKey::from_bits(vec![0b1010_1111], 4).unwrap();
        assert_eq!(key.data(), &[0b1010_0000]);
        assert!(key.bit(0).unwrap());
        assert!(!key.bit(1).unwrap());
        assert_eq!(key.prefix(3).unwrap().data(), &[0b1010_0000]);

        let low = BitKey::from_bits(vec![0b0100_0000], 2).unwrap();
        let high = BitKey::from_bits(vec![0b1000_0000], 2).unwrap();
        assert!(low < high);
    }

    #[test]
    fn labels_choose_canonical_encoding() {
        assert_eq!(canonical_label(&[], 0).unwrap(), vec![false, false]);
        assert_eq!(
            canonical_label(&[true, false], 3).unwrap(),
            vec![false, true, true, false, true, false]
        );
        assert_eq!(
            canonical_label(&[false, false, false, false], 8).unwrap(),
            vec![true, true, false, false, true, false, false]
        );
    }

    #[test]
    fn hashmap_e_roundtrips_uint_values() {
        let mut dict = HashmapE::new(8);
        dict.insert_bit_key(BitKey::from_u64(0b1010_0000, 8).unwrap(), 10u64)
            .unwrap();
        dict.insert_bit_key(BitKey::from_u64(0b1010_1111, 8).unwrap(), 20u64)
            .unwrap();
        dict.insert_bit_key(BitKey::from_u64(0b1111_0000, 8).unwrap(), 30u64)
            .unwrap();

        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&dict, |builder, value| {
                builder.store_uint::<u16>(*value as u16)?;
                Ok(())
            })
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(8, |slice| slice.load_uint::<u16>())
            .unwrap();

        assert_eq!(decoded.len(), 3);
        assert_eq!(
            decoded
                .get_bit_key(&BitKey::from_u64(0b1010_1111, 8).unwrap())
                .unwrap(),
            Some(&20)
        );
    }

    #[test]
    fn hashmap_e_roundtrips_empty_and_wide_keys() {
        let empty: HashmapE<u64> = HashmapE::new(256);
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&empty, |builder, value| {
                builder.store_uint_custom::<u8>(*value as u8, 1)?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        assert!(
            slice
                .load_hashmap_e_with(256, |slice| slice.load_uint_custom::<u8>(1))
                .unwrap()
                .is_empty()
        );

        let mut dict = HashmapE::new(267);
        let key = BitKey::from_bits(vec![0xAA; 34], 267).unwrap();
        dict.insert_bit_key(key.clone(), 7u64).unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&dict, |builder, value| {
                builder.store_uint_custom::<u8>(*value as u8, 4)?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(267, |slice| slice.load_uint_custom::<u8>(4))
            .unwrap();
        assert_eq!(decoded.get_bit_key(&key).unwrap(), Some(&7));
    }

    #[test]
    fn hashmap_e_roundtrips_callback_value_codecs() {
        let mut coins = HashmapE::new(4);
        coins
            .insert_bit_key(BitKey::from_u64(1, 4).unwrap(), 1_000_000_000u128)
            .unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&coins, |builder, value| {
                builder.store_coins(*value)?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(4, |slice| slice.load_coins())
            .unwrap();
        assert_eq!(
            decoded
                .get_bit_key(&BitKey::from_u64(1, 4).unwrap())
                .unwrap(),
            Some(&1_000_000_000)
        );

        let address = Address::new(-1, [0x44; 32]);
        let mut addresses = HashmapE::new(4);
        addresses
            .insert_bit_key(BitKey::from_u64(2, 4).unwrap(), address.clone())
            .unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&addresses, |builder, value| {
                builder.store_address(Some(value))?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(4, |slice| {
                let data = slice.load_bits(267)?;
                let mut builder = Builder::new();
                builder.store_bits(&data, 267)?;
                Ok(builder.build()?)
            })
            .unwrap();
        let cell = decoded
            .get_bit_key(&BitKey::from_u64(2, 4).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(cell.bit_len(), 267);

        let mut raw_builder = Builder::new();
        raw_builder.store_bits(&[0b1010_0000], 4).unwrap();
        let raw_cell = raw_builder.build().unwrap();
        let mut cells = HashmapE::new(4);
        cells
            .insert_bit_key(BitKey::from_u64(3, 4).unwrap(), raw_cell.clone())
            .unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&cells, |builder, value| {
                builder.store_ref(value.clone())?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(4, |slice| slice.load_reference())
            .unwrap();
        assert_eq!(
            decoded
                .get_bit_key(&BitKey::from_u64(3, 4).unwrap())
                .unwrap()
                .unwrap()
                .hash(),
            raw_cell.hash()
        );
    }

    #[test]
    fn dict_address_keys_are_not_truncated() {
        let mut dict = Dict::new(267);
        let address = Address::new(0, [0x11; 32]);
        dict.set(DictKey::Address(address.clone()), DictValue::Coins(1))
            .unwrap();
        assert!(dict.get(&DictKey::Address(address)).unwrap().is_some());
    }

    #[test]
    fn malformed_labels_and_missing_refs_fail() {
        let mut builder = Builder::new();
        builder.store_bit(false).unwrap();
        builder.store_bit(true).unwrap();
        builder.store_bit(true).unwrap();
        builder.store_bit(false).unwrap();
        let mut slice = builder.to_slice().unwrap();
        assert!(load_label(&mut slice, 1).is_err());

        let mut dict = HashmapE::new(2);
        dict.insert_bit_key(BitKey::from_u64(0, 2).unwrap(), 1u64)
            .unwrap();
        dict.insert_bit_key(BitKey::from_u64(2, 2).unwrap(), 2u64)
            .unwrap();
        let root = serialize_hashmap_root(&dict, |builder, value| {
            builder.store_uint_custom::<u8>(*value as u8, 2)?;
            Ok(())
        })
        .unwrap()
        .unwrap();
        let mut broken = Builder::new();
        broken.store_bits(root.data(), root.bit_len()).unwrap();
        let broken = broken.build().unwrap();
        assert!(
            deserialize_hashmap_root(&broken, 2, |slice| slice.load_uint_custom::<u8>(2)).is_err()
        );
    }

    #[test]
    fn hashmap_aug_e_roundtrips_empty_with_top_extra() {
        let dict: HashmapAugE<u64, u64> = HashmapAugE::empty(8, 99);
        let mut builder = Builder::new();
        builder
            .store_hashmap_aug_e_with(
                &dict,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_aug_e_with(
                8,
                |slice| slice.load_uint::<u8>(),
                |slice| slice.load_uint::<u8>(),
            )
            .unwrap();

        assert!(decoded.is_empty());
        assert_eq!(*decoded.extra(), 99);
    }

    #[test]
    fn hashmap_aug_e_rejects_trailing_root_ref_data() {
        let dict = HashmapAug::from_entries(
            8,
            vec![HashmapAugLeaf {
                key: BitKey::from_u64(0xAB, 8).unwrap(),
                value: 7u64,
                extra: 11u64,
            }],
            0,
        )
        .unwrap();

        let mut root_builder = Builder::new();
        root_builder
            .store_hashmap_aug_with(
                &dict,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();
        root_builder.store_bit(true).unwrap();

        let mut builder = Builder::new();
        builder.store_bit(true).unwrap();
        builder.store_ref(root_builder.build().unwrap()).unwrap();
        builder.store_uint::<u8>(99 as u8).unwrap();

        let mut slice = builder.to_slice().unwrap();
        assert!(
            slice
                .load_hashmap_aug_e_with(
                    8,
                    |slice| slice.load_uint::<u8>(),
                    |slice| { slice.load_uint::<u8>() }
                )
                .is_err()
        );
    }

    #[test]
    fn hashmap_aug_roundtrips_single_leaf() {
        let dict = HashmapAug::from_entries(
            8,
            vec![HashmapAugLeaf {
                key: BitKey::from_u64(0xAB, 8).unwrap(),
                value: 7u64,
                extra: 11u64,
            }],
            0,
        )
        .unwrap();

        let mut builder = Builder::new();
        builder
            .store_hashmap_aug_with(
                &dict,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_aug_with(
                8,
                |slice| slice.load_uint::<u8>(),
                |slice| slice.load_uint::<u8>(),
            )
            .unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(
            decoded
                .get_bit_key(&BitKey::from_u64(0xAB, 8).unwrap())
                .unwrap(),
            Some((&7, &11))
        );
    }

    #[test]
    fn hashmap_aug_roundtrips_fork_and_preserves_extras() {
        let dict = HashmapAug::from_entries(
            4,
            vec![
                HashmapAugLeaf {
                    key: BitKey::from_u64(0b0000, 4).unwrap(),
                    value: 1u64,
                    extra: 10u64,
                },
                HashmapAugLeaf {
                    key: BitKey::from_u64(0b0100, 4).unwrap(),
                    value: 2u64,
                    extra: 20u64,
                },
                HashmapAugLeaf {
                    key: BitKey::from_u64(0b1100, 4).unwrap(),
                    value: 3u64,
                    extra: 30u64,
                },
            ],
            77,
        )
        .unwrap();

        let wrapped = HashmapAugE::with_root(4, dict, 88).unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_aug_e_with(
                &wrapped,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_aug_e_with(
                4,
                |slice| slice.load_uint::<u8>(),
                |slice| slice.load_uint::<u8>(),
            )
            .unwrap();
        let root = decoded.root().unwrap();
        let leaves: Vec<_> = root
            .iter()
            .map(|(key, value, extra)| (key.to_u64().unwrap(), *value, *extra))
            .collect();
        assert_eq!(leaves, vec![(0, 1, 10), (4, 2, 20), (12, 3, 30)]);
        assert_eq!(*decoded.extra(), 88);
        assert!(root.fork_extras().iter().all(|fork| fork.extra == 77));
    }

    #[test]
    fn hashmap_aug_rejects_empty_duplicate_and_wrong_width() {
        assert!(HashmapAug::<u64, u64>::from_entries(4, vec![], 0).is_err());
        assert!(
            HashmapAug::from_entries(
                4,
                vec![HashmapAugLeaf {
                    key: BitKey::from_u64(0, 5).unwrap(),
                    value: 1u64,
                    extra: 1u64,
                }],
                0,
            )
            .is_err()
        );
        assert!(
            HashmapAug::from_entries(
                4,
                vec![
                    HashmapAugLeaf {
                        key: BitKey::from_u64(1, 4).unwrap(),
                        value: 1u64,
                        extra: 1u64,
                    },
                    HashmapAugLeaf {
                        key: BitKey::from_u64(1, 4).unwrap(),
                        value: 2u64,
                        extra: 2u64,
                    },
                ],
                0,
            )
            .is_err()
        );
    }
}
