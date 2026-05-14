use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tvm::cell::ExoticCellKind;
    use crate::tvm::cell::{CellBuilder, MAX_CELL_BITS};

    const EMPTY_CELL_BOC_HEX: &str = "b5ee9c72010101010002000000";
    const ONE_BYTE_CELL_BOC_HEX: &str = "b5ee9c72010101010003000002aa";
    const REF_CELL_BOC_HEX: &str = "b5ee9c72010102010007010002aa0102bb00";
    const INDEXED_REF_CELL_BOC_HEX: &str = "b5ee9c728101020100070103070002aa0102bb00";
    const LIBRARY_REFERENCE_BOC_HEX: &str = "b5ee9c72010101010023000842023333333333333333333333333333333333333333333333333333333333333333";
    const TWO_ROOT_BOC_HEX: &str = "b5ee9c72010102020005000100000002aa";

    fn single_cell_boc(cell_bytes: &[u8]) -> Vec<u8> {
        let mut boc = vec![
            0xB5,
            0xEE,
            0x9C,
            0x72, // generic magic
            0x01, // no flags, size_bytes = 1
            0x01, // offset_bytes = 1
            0x01, // cells count
            0x01, // roots count
            0x00, // absent count
            cell_bytes.len() as u8,
            0x00, // root index
        ];
        boc.extend_from_slice(cell_bytes);
        boc
    }

    fn decode_hex_fixture(hex: &str) -> Vec<u8> {
        hex::decode(hex).unwrap()
    }

    #[test]
    fn test_ordinary_boc_golden_fixtures() {
        let empty = deserialize_boc(&decode_hex_fixture(EMPTY_CELL_BOC_HEX)).unwrap();
        assert_eq!(empty.bit_len(), 0);
        assert_eq!(empty.reference_count(), 0);
        assert_eq!(
            serialize_boc(&empty, false).unwrap(),
            decode_hex_fixture(EMPTY_CELL_BOC_HEX)
        );

        let one_byte = deserialize_boc(&decode_hex_fixture(ONE_BYTE_CELL_BOC_HEX)).unwrap();
        assert_eq!(one_byte.bit_len(), 8);
        assert_eq!(one_byte.data(), &[0xAA]);
        assert_eq!(
            serialize_boc(&one_byte, false).unwrap(),
            decode_hex_fixture(ONE_BYTE_CELL_BOC_HEX)
        );
    }

    #[test]
    fn test_indexed_boc_golden_fixture_decodes_to_canonical_unindexed_form() {
        let decoded = deserialize_boc(&decode_hex_fixture(INDEXED_REF_CELL_BOC_HEX)).unwrap();
        assert_eq!(decoded.bit_len(), 8);
        assert_eq!(decoded.data(), &[0xBB]);
        assert_eq!(decoded.reference_count(), 1);
        assert_eq!(decoded.reference(0).unwrap().data(), &[0xAA]);
        assert_eq!(
            serialize_boc(&decoded, false).unwrap(),
            decode_hex_fixture(REF_CELL_BOC_HEX)
        );
    }

    #[test]
    fn test_exotic_library_reference_boc_golden_fixture() {
        let decoded = deserialize_boc(&decode_hex_fixture(LIBRARY_REFERENCE_BOC_HEX)).unwrap();
        let expected_hash = [0x33u8; 32];

        assert!(decoded.is_exotic());
        assert_eq!(decoded.bit_len(), 264);
        assert_eq!(decoded.reference_count(), 0);
        assert_eq!(decoded.descriptors(), [0x08, 0x42]);
        assert_eq!(
            decoded.exotic_kind(),
            Some(&ExoticCellKind::LibraryReference {
                hash: expected_hash
            })
        );
        assert_eq!(
            serialize_boc(&decoded, false).unwrap(),
            decode_hex_fixture(LIBRARY_REFERENCE_BOC_HEX)
        );
    }

    #[test]
    fn test_deserialize_multi_root_boc() {
        let roots = deserialize_boc_roots(&decode_hex_fixture(TWO_ROOT_BOC_HEX)).unwrap();

        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].bit_len(), 0);
        assert_eq!(roots[1].bit_len(), 8);
        assert_eq!(roots[1].data(), &[0xAA]);
    }

    #[test]
    fn test_deserialize_single_root_wrapper_rejects_multi_root_boc() {
        let err = deserialize_boc(&decode_hex_fixture(TWO_ROOT_BOC_HEX))
            .unwrap_err()
            .to_string();

        assert!(err.contains("Expected single-root BoC"));
        assert!(err.contains("2 roots"));
    }

    #[test]
    fn test_inspect_two_root_boc_returns_root_hashes() {
        let boc = decode_hex_fixture(TWO_ROOT_BOC_HEX);
        let inspection = inspect_boc(&boc).unwrap();
        let roots = deserialize_boc_roots(&boc).unwrap();

        assert_eq!(inspection.root_count(), 2);
        assert_eq!(
            inspection.root_hashes,
            roots.iter().map(|root| root.hash()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_inspect_single_root_hash_matches_deserialize_boc() {
        let boc = decode_hex_fixture(ONE_BYTE_CELL_BOC_HEX);
        let inspection = inspect_boc(&boc).unwrap();
        let root = deserialize_boc(&boc).unwrap();

        assert_eq!(inspection.root_count(), 1);
        assert_eq!(inspection.root_hashes, vec![root.hash()]);
    }

    #[test]
    fn test_inspect_rejects_crc_mismatch() {
        let cell = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut boc = serialize_boc(&cell, true).unwrap();
        let last = boc.len() - 1;
        boc[last] ^= 1;

        let err = inspect_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("CRC32 mismatch"));
    }

    #[test]
    fn test_inspect_rejects_invalid_reference_index() {
        let child = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut root = Cell::with_data(vec![0xBB], 8).unwrap();
        root.add_reference(child).unwrap();
        let root = Arc::new(root);

        let mut boc = serialize_boc(&root, false).unwrap();
        let last = boc.len() - 1;
        boc[last] = 2;

        let err = inspect_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("Invalid reference index"));
    }

    #[test]
    fn test_inspect_exotic_payload_without_semantic_validation() {
        let boc = single_cell_boc(&[
            0x08, // exotic descriptor, level 0, no refs
            0x02, // one full payload byte
            0xFF, // unsupported semantic exotic tag
        ]);

        let inspection = inspect_boc(&boc).unwrap();
        assert_eq!(inspection.root_count(), 1);

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("Unsupported exotic cell type"));
    }

    #[test]
    fn test_cache_bit_fixture_is_rejected_by_policy() {
        let mut boc = decode_hex_fixture(EMPTY_CELL_BOC_HEX);
        boc[4] |= 0x20;

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("cache bits flag"));
        assert!(err.contains("unsupported"));
    }

    #[test]
    fn test_serialize_deserialize_simple() {
        let mut builder = CellBuilder::new();
        builder.store_u32(0x12345678).unwrap();
        let cell = builder.build().unwrap();

        let boc = serialize_boc(&cell, false).unwrap();
        let deserialized = deserialize_boc(&boc).unwrap();

        assert_eq!(cell.hash(), deserialized.hash());
    }

    #[test]
    fn test_hex_conversion() {
        let mut builder = CellBuilder::new();
        builder.store_byte(0xFF).unwrap();
        let cell = builder.build().unwrap();

        let hex = boc_to_hex(&cell, false).unwrap();
        let decoded = hex_to_boc(&hex).unwrap();

        assert_eq!(cell.hash(), decoded.hash());
    }

    #[test]
    fn test_partial_byte_boc_roundtrips() {
        for bit_len in [1, 7, 9, MAX_CELL_BITS] {
            let data = vec![0xFF; bit_len.div_ceil(8)];
            let cell = Arc::new(Cell::with_data(data, bit_len).unwrap());
            let boc = serialize_boc(&cell, false).unwrap();
            let decoded = deserialize_boc(&boc).unwrap();

            assert_eq!(decoded.bit_len(), bit_len);
            assert_eq!(decoded.data(), cell.data());
            assert_eq!(decoded.hash(), cell.hash());
        }
    }

    #[test]
    fn test_partial_byte_boc_decoding_removes_top_up_marker() {
        let cell = Arc::new(Cell::with_data(vec![0x80], 1).unwrap());
        let boc = serialize_boc(&cell, false).unwrap();
        let decoded = deserialize_boc(&boc).unwrap();

        assert_eq!(decoded.bit_len(), 1);
        assert_eq!(decoded.data(), &[0x80]);
        assert_eq!(decoded.serialize_data(), vec![0xC0]);
    }

    #[test]
    fn test_nested_reference_roundtrip_with_partial_child() {
        let partial_child = Arc::new(Cell::with_data(vec![0xFE], 7).unwrap());
        let full_child = Arc::new(Cell::with_data(vec![0x12, 0x34], 16).unwrap());
        let mut root = Cell::with_data(vec![0x80], 1).unwrap();
        root.add_reference(partial_child.clone()).unwrap();
        root.add_reference(full_child).unwrap();
        let root = Arc::new(root);

        let boc = serialize_boc(&root, false).unwrap();
        let decoded = deserialize_boc(&boc).unwrap();

        assert_eq!(decoded.hash(), root.hash());
        assert_eq!(decoded.reference_count(), 2);
        assert_eq!(decoded.reference(0).unwrap().bit_len(), 7);
        assert_eq!(decoded.reference(0).unwrap().data(), partial_child.data());
    }

    #[test]
    fn test_deserialize_accepts_index_table() {
        let cell = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut boc = serialize_boc(&cell, false).unwrap();
        boc[4] |= 0x80;
        let cells_size = boc[9];
        boc.insert(11, cells_size);

        let decoded = deserialize_boc(&boc).unwrap();
        assert_eq!(decoded.hash(), cell.hash());
    }

    #[test]
    fn test_deserialize_rejects_malformed_index_table() {
        let cell = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut boc = serialize_boc(&cell, false).unwrap();
        boc[4] |= 0x80;
        boc.insert(11, 0);

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("index table"));
    }

    #[test]
    fn test_deserialize_rejects_unsupported_cache_bits_flag() {
        let cell = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut boc = serialize_boc(&cell, false).unwrap();
        boc[4] |= 0x20;

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("cache bits flag"));
    }

    #[test]
    fn test_deserialize_rejects_crc_mismatch() {
        let cell = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut boc = serialize_boc(&cell, true).unwrap();
        let last = boc.len() - 1;
        boc[last] ^= 1;

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("CRC32 mismatch"));
    }

    #[test]
    fn test_deserialize_rejects_invalid_root_index() {
        let cell = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut boc = serialize_boc(&cell, false).unwrap();
        boc[10] = 1;

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("Invalid root index"));
    }

    #[test]
    fn test_deserialize_rejects_invalid_reference_index() {
        let child = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut root = Cell::with_data(vec![0xBB], 8).unwrap();
        root.add_reference(child).unwrap();
        let root = Arc::new(root);

        let mut boc = serialize_boc(&root, false).unwrap();
        let last = boc.len() - 1;
        boc[last] = 2;

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("Invalid reference index"));
    }

    #[test]
    fn test_deserialize_rejects_trailing_bytes_without_checksum() {
        let cell = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let mut boc = serialize_boc(&cell, false).unwrap();
        boc.push(0);

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("Trailing bytes"));
    }

    #[test]
    fn test_deserialize_rejects_malformed_partial_byte_without_top_up() {
        let boc = vec![
            0xB5, 0xEE, 0x9C, 0x72, // generic magic
            0x01, // no flags, size_bytes = 1
            0x01, // offset_bytes = 1
            0x01, // cells count
            0x01, // roots count
            0x00, // absent count
            0x03, // cells size
            0x00, // root index
            0x00, // d1
            0x01, // d2: one partial data byte
            0x00, // malformed: no top-up marker
        ];

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("missing top-up bit"));
    }

    #[test]
    fn test_deserialize_rejects_top_up_only_partial_byte() {
        let boc = vec![
            0xB5, 0xEE, 0x9C, 0x72, 0x01, 0x01, 0x01, 0x01, 0x00, 0x03, 0x00, 0x00, 0x01, 0x80,
        ];

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("top-up bit without data bits"));
    }

    #[test]
    fn test_base64_conversion_with_crc_roundtrips() {
        let mut builder = CellBuilder::new();
        builder.store_u32(0xDEADBEEF).unwrap();
        let cell = builder.build().unwrap();

        let b64 = boc_to_base64(&cell, true).unwrap();
        let decoded = base64_to_boc(&b64).unwrap();

        assert_eq!(cell.hash(), decoded.hash());
    }

    #[test]
    fn test_deserialize_exotic_descriptor_does_not_become_ordinary_cell() {
        let library_hash = [0x11u8; 32];
        let mut data = vec![0x02];
        data.extend_from_slice(&library_hash);
        let cell = Arc::new(Cell::with_exotic_data(data, 264, Vec::new()).unwrap());

        let boc = serialize_boc(&cell, false).unwrap();
        let decoded = deserialize_boc(&boc).unwrap();

        assert!(decoded.is_exotic());
        assert_eq!(decoded.level(), 0);
        assert_eq!(decoded.reference_count(), 0);
        assert_eq!(decoded.descriptors(), [0x08, 0x42]);
        assert_eq!(decoded.hash(), cell.hash());
        assert_eq!(
            decoded.exotic_kind(),
            Some(&ExoticCellKind::LibraryReference { hash: library_hash })
        );
    }

    #[test]
    fn test_deserialize_rejects_invalid_exotic_payload() {
        let boc = single_cell_boc(&[
            0x08, // exotic descriptor, level 0, no refs
            0x02, // one full payload byte
            0x02, // library reference tag without the required 256-bit hash
        ]);

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("Invalid exotic cell"));
        assert!(err.contains("library reference payload length"));
    }

    #[test]
    fn test_deserialize_rejects_unsupported_exotic_type() {
        let boc = single_cell_boc(&[
            0x08, // exotic descriptor, level 0, no refs
            0x02, // one full payload byte
            0xFF, // unsupported exotic tag
        ]);

        let err = deserialize_boc(&boc).unwrap_err().to_string();
        assert!(err.contains("Unsupported exotic cell type"));
    }

    #[test]
    fn test_exotic_pruned_branch_descriptor_level_depth_and_hash_roundtrip() {
        let pruned_hash = [0x22u8; 32];
        let pruned_depth = 7u16;
        let mut data = vec![0x01, 0x01];
        data.extend_from_slice(&pruned_hash);
        data.extend_from_slice(&pruned_depth.to_be_bytes());
        let cell = Arc::new(Cell::with_exotic_data(data, 288, Vec::new()).unwrap());

        assert!(cell.is_exotic());
        assert_eq!(cell.level(), 1);
        assert_eq!(cell.depth(), 0);
        assert_eq!(cell.descriptors(), [0x28, 0x48]);
        assert_eq!(
            cell.exotic_kind(),
            Some(&ExoticCellKind::PrunedBranch {
                level_mask: 0x01,
                hashes: vec![pruned_hash],
                depths: vec![pruned_depth],
            })
        );

        let decoded = deserialize_boc(&serialize_boc(&cell, false).unwrap()).unwrap();
        assert_eq!(decoded.exotic_kind(), cell.exotic_kind());
        assert_eq!(decoded.hash(), cell.hash());
    }

    #[test]
    fn test_exotic_library_reference_descriptor_level_depth_and_hash_roundtrip() {
        let library_hash = [0x33u8; 32];
        let mut data = vec![0x02];
        data.extend_from_slice(&library_hash);
        let cell = Arc::new(Cell::with_exotic_data(data, 264, Vec::new()).unwrap());

        assert!(cell.is_exotic());
        assert_eq!(cell.level(), 0);
        assert_eq!(cell.depth(), 0);
        assert_eq!(cell.descriptors(), [0x08, 0x42]);

        let decoded = deserialize_boc(&serialize_boc(&cell, false).unwrap()).unwrap();
        assert_eq!(decoded.exotic_kind(), cell.exotic_kind());
        assert_eq!(decoded.hash(), cell.hash());
    }

    #[test]
    fn test_exotic_merkle_proof_descriptor_level_depth_and_hash_roundtrip() {
        let child = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let proof_hash = child.hash();
        let proof_depth = child.depth();
        let mut data = vec![0x03];
        data.extend_from_slice(&proof_hash);
        data.extend_from_slice(&proof_depth.to_be_bytes());
        let cell = Arc::new(Cell::with_exotic_data(data, 280, vec![child]).unwrap());

        assert!(cell.is_exotic());
        assert_eq!(cell.level(), 0);
        assert_eq!(cell.depth(), 1);
        assert_eq!(cell.descriptors(), [0x09, 0x46]);

        let decoded = deserialize_boc(&serialize_boc(&cell, false).unwrap()).unwrap();
        assert_eq!(decoded.exotic_kind(), cell.exotic_kind());
        assert_eq!(decoded.hash(), cell.hash());
    }

    #[test]
    fn test_exotic_merkle_update_descriptor_level_depth_and_hash_roundtrip() {
        let old = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        let new = Arc::new(Cell::with_data(vec![0xBB], 8).unwrap());
        let mut data = vec![0x04];
        data.extend_from_slice(&old.hash());
        data.extend_from_slice(&new.hash());
        data.extend_from_slice(&old.depth().to_be_bytes());
        data.extend_from_slice(&new.depth().to_be_bytes());
        let cell = Arc::new(Cell::with_exotic_data(data, 552, vec![old, new]).unwrap());

        assert!(cell.is_exotic());
        assert_eq!(cell.level(), 0);
        assert_eq!(cell.depth(), 1);
        assert_eq!(cell.descriptors(), [0x0A, 0x8A]);

        let decoded = deserialize_boc(&serialize_boc(&cell, false).unwrap()).unwrap();
        assert_eq!(decoded.exotic_kind(), cell.exotic_kind());
        assert_eq!(decoded.hash(), cell.hash());
    }
}
