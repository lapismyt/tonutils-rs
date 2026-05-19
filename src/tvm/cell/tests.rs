use super::*;

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::*;

    fn hex_bytes(hex: &str) -> Vec<u8> {
        hex::decode(hex).unwrap()
    }

    fn representation_preimage(cell: &Cell) -> Vec<u8> {
        let mut preimage = Vec::new();
        preimage.extend_from_slice(&cell.descriptors());
        preimage.extend_from_slice(&cell.serialize_data());

        for reference in cell.references() {
            preimage.extend_from_slice(&reference.depth().to_be_bytes());
        }

        for reference in cell.references() {
            preimage.extend_from_slice(&reference.hash());
        }

        preimage
    }

    #[test]
    fn test_empty_cell() {
        let cell = Cell::new();
        assert_eq!(cell.bit_len(), 0);
        assert_eq!(cell.reference_count(), 0);
        assert_eq!(cell.level(), 0);
        assert!(!cell.is_exotic());
    }

    #[test]
    fn test_cell_with_data() {
        let data = vec![0x0F];
        let cell = Cell::with_data(data, 8).unwrap();
        assert_eq!(cell.bit_len(), 8);
        assert_eq!(cell.data()[0], 0x0F);
    }

    #[test]
    fn test_cell_with_data_canonicalizes_partial_byte() {
        let cell = Cell::with_data(vec![0xFF, 0xAA], 1).unwrap();
        assert_eq!(cell.bit_len(), 1);
        assert_eq!(cell.data(), &[0x80]);
        assert_eq!(cell.serialize_data(), vec![0xC0]);
    }

    #[test]
    fn test_cell_with_data_preserves_full_byte_data() {
        let cell = Cell::with_data(vec![0xFF, 0xAA], 8).unwrap();
        assert_eq!(cell.data(), &[0xFF]);
        assert_eq!(cell.serialize_data(), vec![0xFF]);
    }

    #[test]
    fn test_cell_descriptors_for_partial_and_full_bytes() {
        assert_eq!(
            Cell::with_data(vec![0x80], 1).unwrap().descriptors(),
            [0, 1]
        );
        assert_eq!(
            Cell::with_data(vec![0xFE], 7).unwrap().descriptors(),
            [0, 1]
        );
        assert_eq!(
            Cell::with_data(vec![0xFF], 8).unwrap().descriptors(),
            [0, 2]
        );
        assert_eq!(
            Cell::with_data(vec![0xFF, 0x80], 9).unwrap().descriptors(),
            [0, 3]
        );
    }

    #[test]
    fn test_cell_serialize_data_top_up_bit() {
        assert_eq!(
            Cell::with_data(vec![0x80], 1).unwrap().serialize_data(),
            vec![0xC0]
        );
        assert_eq!(
            Cell::with_data(vec![0xFE], 7).unwrap().serialize_data(),
            vec![0xFF]
        );
        assert_eq!(
            Cell::with_data(vec![0xAB], 8).unwrap().serialize_data(),
            vec![0xAB]
        );
        assert_eq!(
            Cell::with_data(vec![0xFF, 0x80], 9)
                .unwrap()
                .serialize_data(),
            vec![0xFF, 0xC0]
        );
    }

    #[test]
    fn test_cell_builder() {
        let mut builder = CellBuilder::new();
        builder.store_byte(0xFF).unwrap();
        builder.store_u32(0x12345678).unwrap();

        let cell = builder.build().unwrap();
        assert_eq!(cell.bit_len(), 40); // 8 + 32 bits
    }

    #[test]
    fn test_cell_builder_store_uint_natural_widths() {
        let mut builder = CellBuilder::new();
        builder.store_uint::<u8>(0x12).unwrap();
        builder.store_uint::<u16>(0x3456).unwrap();
        builder.store_uint::<u32>(0x789a_bcde).unwrap();
        builder.store_uint::<u64>(0x0123_4567_89ab_cdef).unwrap();
        builder
            .store_uint::<u128>(0x0123_4567_89ab_cdef_1122_3344_5566_7788)
            .unwrap();

        let cell = builder.build().unwrap();
        assert_eq!(cell.bit_len(), 8 + 16 + 32 + 64 + 128);
    }

    #[test]
    fn test_cell_builder_store_uint_custom_widths() {
        let mut builder = CellBuilder::new();
        builder.store_uint_custom::<u8>(0, 0).unwrap();
        builder.store_uint_custom::<u8>(0b101, 3).unwrap();
        builder.store_uint_custom::<u16>(0x01ff, 9).unwrap();
        builder.store_uint_custom::<u32>(0x00ab_cdef, 24).unwrap();
        assert!(builder.store_uint_custom::<u8>(8, 3).is_err());
        assert!(builder.store_uint_custom::<u8>(0, 9).is_err());

        let cell = builder.build().unwrap();
        let mut slice = crate::tvm::Slice::new(cell);
        assert_eq!(slice.load_uint_custom::<u8>(0).unwrap(), 0);
        assert_eq!(slice.load_uint_custom::<u8>(3).unwrap(), 0b101);
        assert_eq!(slice.load_uint_custom::<u16>(9).unwrap(), 0x01ff);
        assert_eq!(slice.load_uint_custom::<u32>(24).unwrap(), 0x00ab_cdef);
    }

    #[test]
    fn test_cell_hash() {
        let cell = Cell::with_data(vec![0x00, 0x00, 0x00, 0x0F], 32).unwrap();

        assert_eq!(representation_preimage(&cell), hex_bytes("00080000000f"));
        assert_eq!(
            cell.hash().as_slice(),
            hex_bytes("57b520dbcb9d135863fc33963cde9f6db2ded1430d88056810a2c9434a3860f9")
        );
    }

    #[test]
    fn test_ordinary_cell_hash_golden_fixtures() {
        let fixtures = [
            (
                Cell::new(),
                "0000",
                "96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7",
            ),
            (
                Cell::with_data(vec![0x80], 1).unwrap(),
                "0001c0",
                "7c6c1a965fd501d2938c2c0e06626bdaa3531357016e169070c9ef79c4c46bc0",
            ),
            (
                Cell::with_data(vec![0xAB], 8).unwrap(),
                "0002ab",
                "57c2a1a13baa2762109ed68be0c396f2303ce17e3dde7917d0e74b4072b1dbc7",
            ),
            (
                Cell::with_data(vec![0x00, 0x00, 0x00, 0x0F], 32).unwrap(),
                "00080000000f",
                "57b520dbcb9d135863fc33963cde9f6db2ded1430d88056810a2c9434a3860f9",
            ),
        ];

        for (cell, expected_preimage, expected_hash) in fixtures {
            assert_eq!(representation_preimage(&cell), hex_bytes(expected_preimage));
            assert_eq!(cell.hash().as_slice(), hex_bytes(expected_hash));
        }
    }

    #[test]
    fn test_ordinary_cell_hash_two_reference_preimage_order() {
        let first_child = Arc::new(Cell::new());
        let second_child = Arc::new(Cell::with_data(vec![0x80], 1).unwrap());

        let mut root = Cell::with_data(vec![0x80], 1).unwrap();
        root.add_reference(first_child.clone()).unwrap();
        root.add_reference(second_child.clone()).unwrap();

        assert_eq!(first_child.depth(), 0);
        assert_eq!(second_child.depth(), 0);
        assert_eq!(root.depth(), 1);
        assert_eq!(root.descriptors(), [2, 1]);

        let expected_preimage = format!(
            "0201c000000000{}{}",
            hex::encode(first_child.hash()),
            hex::encode(second_child.hash())
        );

        assert_eq!(
            first_child.hash().as_slice(),
            hex_bytes("96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7")
        );
        assert_eq!(
            second_child.hash().as_slice(),
            hex_bytes("7c6c1a965fd501d2938c2c0e06626bdaa3531357016e169070c9ef79c4c46bc0")
        );
        assert_eq!(
            representation_preimage(&root),
            hex_bytes(&expected_preimage)
        );
        assert_eq!(
            root.hash().as_slice(),
            hex_bytes("383598f93bde0afbe68b632ae75d5ffa6747df1284e2f4abb86cd2c5840514fe")
        );
    }

    #[test]
    fn test_ordinary_cell_hash_chained_reference_depths() {
        let leaf = Arc::new(Cell::new());

        let mut middle = Cell::with_data(vec![0x80], 1).unwrap();
        middle.add_reference(leaf.clone()).unwrap();
        let middle = Arc::new(middle);

        let mut root = Cell::with_data(vec![0xAB], 8).unwrap();
        root.add_reference(middle.clone()).unwrap();

        assert_eq!(leaf.depth(), 0);
        assert_eq!(middle.depth(), 1);
        assert_eq!(root.depth(), 2);

        assert_eq!(leaf.descriptors(), [0, 0]);
        assert_eq!(middle.descriptors(), [1, 1]);
        assert_eq!(root.descriptors(), [1, 2]);

        assert_eq!(representation_preimage(&leaf), hex_bytes("0000"));
        assert_eq!(
            leaf.hash().as_slice(),
            hex_bytes("96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7")
        );

        assert_eq!(
            representation_preimage(&middle),
            hex_bytes(
                "0101c00000\
                 96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7"
            )
        );
        assert_eq!(
            middle.hash().as_slice(),
            hex_bytes("9770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b")
        );

        assert_eq!(
            representation_preimage(&root),
            hex_bytes(
                "0102ab0001\
                 9770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b"
            )
        );
        assert_eq!(
            root.hash().as_slice(),
            hex_bytes("9f19f1fa052329a70f79c2adaef4e9f4e73eb88be389918473adc5f9a2801181")
        );
    }

    #[test]
    fn test_ordinary_cell_hash_two_reference_depth_order_with_nested_ref() {
        let leaf = Arc::new(Cell::new());

        let mut middle = Cell::with_data(vec![0x80], 1).unwrap();
        middle.add_reference(leaf.clone()).unwrap();
        let middle = Arc::new(middle);

        let mut root = Cell::with_data(vec![0xAB], 8).unwrap();
        root.add_reference(leaf.clone()).unwrap();
        root.add_reference(middle.clone()).unwrap();

        assert_eq!(leaf.depth(), 0);
        assert_eq!(middle.depth(), 1);
        assert_eq!(root.depth(), 2);
        assert_eq!(root.descriptors(), [2, 2]);

        assert_eq!(
            representation_preimage(&root),
            hex_bytes(
                "0202ab00000001\
                 96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7\
                 9770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b"
            )
        );
        assert_eq!(
            root.hash().as_slice(),
            hex_bytes("6d112e22e9b4f47922b27cb78ffb8c4c3be4be304cdcb9ad24560e3104827eb6")
        );
    }
}
