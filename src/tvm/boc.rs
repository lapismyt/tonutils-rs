//! Bag of Cells (BoC) serialization and deserialization
//!
//! BoC is a serialization format that encodes cells into byte arrays.
//! It allows storing and transmitting cell structures efficiently.

use crate::tvm::cell::Cell;
use anyhow::{Result, bail};
use std::collections::HashMap;
use std::sync::Arc;

/// BoC magic number for standard format
const BOC_GENERIC_MAGIC: u32 = 0xb5ee9c72;

/// BoC magic number for indexed format (with CRC32)
const BOC_INDEXED_MAGIC: u32 = 0x68ff65f3;

/// BoC magic number for indexed format (with CRC32C)
const BOC_INDEXED_CRC32C_MAGIC: u32 = 0xacc3a728;

/// Serializes a cell and its references into a Bag of Cells (BoC) format
pub fn serialize_boc(root: &Arc<Cell>, has_crc32: bool) -> Result<Vec<u8>> {
    // Collect all unique cells
    let cells = collect_cells(root)?;

    // Find the root index in the cells vector
    let root_index = cells
        .iter()
        .position(|cell| cell.hash() == root.hash())
        .ok_or_else(|| anyhow::anyhow!("Root cell not found in collected cells"))?;

    // Serialize each cell
    let mut serialized_cells = Vec::new();
    let mut cell_map = HashMap::new();
    let size_bytes = bytes_needed(cells.len());

    for (idx, cell) in cells.iter().enumerate() {
        cell_map.insert(cell_hash(cell), idx);
        serialized_cells.push(serialize_cell(cell, &cell_map, size_bytes)?);
    }

    // Calculate total size of serialized cells
    let cells_size: usize = serialized_cells.iter().map(|c| c.len()).sum();

    // Determine size parameters
    let offset_bytes = bytes_needed(cells_size);

    // Build header
    let mut result = Vec::new();

    // Magic number
    result.extend_from_slice(&BOC_GENERIC_MAGIC.to_be_bytes());

    // Flags and size
    let has_idx = 0u8;
    let has_crc32_flag = if has_crc32 { 1u8 } else { 0u8 };
    let has_cache_bits = 0u8;
    let flags = (has_idx << 7) | (has_crc32_flag << 6) | (has_cache_bits << 5);
    let flags_and_size = flags | (size_bytes as u8);
    result.push(flags_and_size);

    // Offset bytes
    result.push(offset_bytes as u8);

    // Number of cells
    write_uint(&mut result, cells.len(), size_bytes);

    // Number of roots (always 1 for now)
    write_uint(&mut result, 1, size_bytes);

    // Number of absent cells (always 0)
    write_uint(&mut result, 0, size_bytes);

    // Total cells size
    write_uint(&mut result, cells_size, offset_bytes);

    // Root cell index
    write_uint(&mut result, root_index, size_bytes);

    // Append serialized cells
    for cell_data in serialized_cells {
        result.extend_from_slice(&cell_data);
    }

    // Add CRC32 if requested
    if has_crc32 {
        let crc = crate::crc::CRC32.checksum(&result);
        result.extend_from_slice(&crc.to_le_bytes());
    }

    Ok(result)
}

/// Deserializes a Bag of Cells (BoC) into a root cell
pub fn deserialize_boc(data: &[u8]) -> Result<Arc<Cell>> {
    if data.len() < 4 {
        bail!("BoC data too short");
    }

    let magic = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);

    match magic {
        BOC_GENERIC_MAGIC => deserialize_boc_generic(data),
        BOC_INDEXED_MAGIC | BOC_INDEXED_CRC32C_MAGIC => {
            bail!("Indexed BoC format not yet supported");
        }
        _ => bail!("Invalid BoC magic number: 0x{:08x}", magic),
    }
}

fn deserialize_boc_generic(data: &[u8]) -> Result<Arc<Cell>> {
    let mut pos = 4; // Skip magic

    if pos >= data.len() {
        bail!("Unexpected end of BoC data");
    }

    // Parse flags and size
    let flags_and_size = data[pos];
    pos += 1;

    let has_idx = (flags_and_size & 0x80) != 0;
    let has_crc32 = (flags_and_size & 0x40) != 0;
    let has_cache_bits = (flags_and_size & 0x20) != 0;
    let size_bytes = (flags_and_size & 0x07) as usize;

    if has_cache_bits {
        bail!("BoC cache bits flag is unsupported for ordinary-cell decoding");
    }

    if size_bytes == 0 || size_bytes > 8 {
        bail!("Invalid size_bytes: {}", size_bytes);
    }

    // Offset bytes
    if pos >= data.len() {
        bail!("Unexpected end of BoC data");
    }
    let offset_bytes = data[pos] as usize;
    pos += 1;

    if offset_bytes == 0 || offset_bytes > 8 {
        bail!("Invalid offset_bytes: {}", offset_bytes);
    }

    // Number of cells
    let cells_count = read_uint(data, &mut pos, size_bytes)?;

    // Number of roots
    let roots_count = read_uint(data, &mut pos, size_bytes)?;
    if roots_count != 1 {
        bail!("Multiple roots not supported yet");
    }

    // Number of absent cells
    let _absent_count = read_uint(data, &mut pos, size_bytes)?;

    // Total cells size
    let cells_size = read_uint(data, &mut pos, offset_bytes)?;

    // Root cell index
    let root_idx = read_uint(data, &mut pos, size_bytes)?;
    if root_idx >= cells_count {
        bail!("Invalid root index: {}", root_idx);
    }

    if has_idx {
        let mut previous_offset = 0usize;
        for index in 0..cells_count {
            let offset = read_uint(data, &mut pos, offset_bytes)
                .map_err(|_| anyhow::anyhow!("Malformed BoC index table"))?;
            if offset < previous_offset || offset > cells_size {
                bail!("Malformed BoC index table");
            }
            previous_offset = offset;
            if index + 1 == cells_count && offset != cells_size {
                bail!("Malformed BoC index table");
            }
        }
    }

    // Parse cells
    let cells_start = pos;
    let cells_end = cells_start
        .checked_add(cells_size)
        .ok_or_else(|| anyhow::anyhow!("Invalid cells size"))?;

    let checksum_size = if has_crc32 { 4 } else { 0 };
    let payload_end = data
        .len()
        .checked_sub(checksum_size)
        .ok_or_else(|| anyhow::anyhow!("Missing CRC32"))?;

    if cells_end > payload_end {
        bail!("Invalid cells size");
    }

    let expected_len = cells_end + checksum_size;
    if data.len() != expected_len {
        bail!("Trailing bytes after BoC cell payload");
    }

    let cells_data = &data[cells_start..cells_end];
    let cells = parse_cells(cells_data, cells_count, size_bytes)?;

    // Verify CRC32 if present
    if has_crc32 {
        if data.len() < cells_end + 4 {
            bail!("Missing CRC32");
        }
        let expected_crc = u32::from_le_bytes([
            data[cells_end],
            data[cells_end + 1],
            data[cells_end + 2],
            data[cells_end + 3],
        ]);
        let actual_crc = crate::crc::CRC32.checksum(&data[..cells_end]);
        if expected_crc != actual_crc {
            bail!(
                "CRC32 mismatch: expected 0x{:08x}, got 0x{:08x}",
                expected_crc,
                actual_crc
            );
        }
    }

    // Return root cell
    Ok(cells[root_idx].clone())
}

fn parse_cells(data: &[u8], count: usize, ref_index_size: usize) -> Result<Vec<Arc<Cell>>> {
    let mut cells = Vec::with_capacity(count);
    let mut cell_refs: Vec<Vec<usize>> = Vec::with_capacity(count);
    let mut cell_is_exotic = Vec::with_capacity(count);
    let mut cell_levels = Vec::with_capacity(count);
    let mut cell_raw_data = Vec::with_capacity(count);
    let mut cell_bit_lens = Vec::with_capacity(count);
    let mut pos = 0;

    // First pass: parse cell data and reference indices
    for _ in 0..count {
        if pos >= data.len() {
            bail!("Unexpected end of cells data");
        }

        // Parse descriptors
        let d1 = data[pos];
        pos += 1;

        if pos >= data.len() {
            bail!("Unexpected end of cells data");
        }

        let d2 = data[pos];
        pos += 1;

        // Parse descriptor 1
        let ref_count = (d1 & 0x07) as usize;
        let is_exotic = (d1 & 0x08) != 0;
        let level = (d1 >> 5) & 0x03;
        if d1 & 0x10 != 0 || d1 & 0x80 != 0 {
            bail!("Invalid cell descriptor: reserved bits are set");
        }

        // Parse descriptor 2
        // d2 = floor(b/8) + ceil(b/8) where b is the number of bits
        // This means: for full bytes, d2 = 2*bytes; for partial bytes, d2 = 2*bytes + 1
        // So actual data size in bytes = ceil(d2/2)
        let data_size = (d2 as usize + 1) / 2;

        // Read cell data
        if pos + data_size > data.len() {
            bail!("Cell data exceeds buffer");
        }

        let serialized_cell_data = data[pos..pos + data_size].to_vec();
        pos += data_size;

        // Read reference indices
        let mut refs = Vec::new();
        for _ in 0..ref_count {
            refs.push(read_uint(data, &mut pos, ref_index_size).map_err(|_| {
                anyhow::anyhow!("Unexpected end of cells data while reading references")
            })?);
        }
        cell_refs.push(refs);
        cell_is_exotic.push(is_exotic);
        cell_levels.push(level);

        // Calculate bit length from descriptor d2
        // d2 = floor(b/8) + ceil(b/8)
        // For b bits: if b % 8 == 0, then d2 = 2*(b/8), so b = d2*4
        //             if b % 8 != 0, then d2 = 2*floor(b/8) + 1, so b is between (d2-1)*4 and d2*4
        // We need to find the exact bit length by looking at the padding bit
        let (cell_data, bit_len) = decode_cell_data(&serialized_cell_data, d2)?;
        cell_raw_data.push(cell_data.clone());
        cell_bit_lens.push(bit_len);

        // Create cell (without references for now)
        let cell = Cell::with_data(cell_data, bit_len)?;
        cells.push(Arc::new(cell));
    }

    if pos != data.len() {
        bail!("Trailing bytes after parsed cells");
    }

    // Second pass: resolve references
    for i in 0..cells.len() {
        let refs = &cell_refs[i];
        let mut references = Vec::with_capacity(refs.len());
        for &ref_idx in refs {
            if ref_idx >= cells.len() {
                bail!("Invalid reference index: {}", ref_idx);
            }
            references.push(cells[ref_idx].clone());
        }

        let new_cell = if cell_is_exotic[i] {
            Cell::with_exotic_data(cell_raw_data[i].clone(), cell_bit_lens[i], references)
                .map_err(|err| anyhow::anyhow!("Invalid exotic cell: {}", err))?
        } else {
            let mut cell = Cell::with_data(cell_raw_data[i].clone(), cell_bit_lens[i])?;
            for reference in references {
                cell.add_reference(reference)?;
            }
            cell
        };

        if new_cell.level() != cell_levels[i] {
            bail!(
                "Invalid cell descriptor level: expected {}, got {}",
                new_cell.level(),
                cell_levels[i]
            );
        }

        cells[i] = Arc::new(new_cell);
    }

    Ok(cells)
}

fn decode_cell_data(data: &[u8], d2: u8) -> Result<(Vec<u8>, usize)> {
    let data_size = (d2 as usize + 1) / 2;
    if data.len() != data_size {
        bail!("Cell data size does not match descriptor");
    }

    if d2 == 0 {
        return Ok((Vec::new(), 0));
    }

    if d2 % 2 == 0 {
        return Ok((data.to_vec(), (d2 as usize / 2) * 8));
    }

    let mut cell_data = data.to_vec();
    let last_byte = *cell_data
        .last()
        .ok_or_else(|| anyhow::anyhow!("Partial cell data is missing top-up byte"))?;
    if last_byte == 0 {
        bail!("Malformed partial cell data: missing top-up bit");
    }

    let zero_padding_bits = last_byte.trailing_zeros() as usize;
    let data_bits_in_last_byte = 7usize
        .checked_sub(zero_padding_bits)
        .ok_or_else(|| anyhow::anyhow!("Malformed partial cell data: invalid top-up bit"))?;
    if data_bits_in_last_byte == 0 {
        bail!("Malformed partial cell data: top-up bit without data bits");
    }

    let last_idx = cell_data.len() - 1;
    let data_mask = 0xFFu8 << (8 - data_bits_in_last_byte);
    cell_data[last_idx] &= data_mask;
    let bit_len = last_idx * 8 + data_bits_in_last_byte;

    Ok((cell_data, bit_len))
}

fn serialize_cell(
    cell: &Arc<Cell>,
    cell_map: &HashMap<[u8; 32], usize>,
    ref_index_size: usize,
) -> Result<Vec<u8>> {
    let mut result = Vec::new();

    // Add descriptors
    let descriptors = cell.descriptors();
    result.extend_from_slice(&descriptors);

    // Add cell data
    let data = cell.serialize_data();
    result.extend_from_slice(&data);

    // Add reference indices
    for reference in cell.references() {
        let ref_hash = cell_hash(reference);
        let ref_idx = cell_map
            .get(&ref_hash)
            .ok_or_else(|| anyhow::anyhow!("Reference not found in cell map"))?;

        write_uint(&mut result, *ref_idx, ref_index_size);
    }

    Ok(result)
}

fn collect_cells(root: &Arc<Cell>) -> Result<Vec<Arc<Cell>>> {
    let mut cells = Vec::new();
    let mut visited = HashMap::new();
    collect_cells_recursive(root, &mut cells, &mut visited)?;

    // Cells are already in topological order (children before parents)
    // No reverse needed for BoC serialization

    Ok(cells)
}

fn collect_cells_recursive(
    cell: &Arc<Cell>,
    cells: &mut Vec<Arc<Cell>>,
    visited: &mut HashMap<[u8; 32], usize>,
) -> Result<()> {
    let hash = cell_hash(cell);

    if visited.contains_key(&hash) {
        return Ok(());
    }

    // Visit children first
    for reference in cell.references() {
        collect_cells_recursive(reference, cells, visited)?;
    }

    // Add this cell
    visited.insert(hash, cells.len());
    cells.push(cell.clone());

    Ok(())
}

fn cell_hash(cell: &Arc<Cell>) -> [u8; 32] {
    cell.hash()
}

fn bytes_needed(value: usize) -> usize {
    if value == 0 {
        return 1;
    }

    let bits = (usize::BITS - value.leading_zeros()) as usize;
    (bits + 7) / 8
}

fn write_uint(buf: &mut Vec<u8>, value: usize, size: usize) {
    let bytes = value.to_be_bytes();
    let start = 8 - size;
    buf.extend_from_slice(&bytes[start..]);
}

fn read_uint(data: &[u8], pos: &mut usize, size: usize) -> Result<usize> {
    if *pos + size > data.len() {
        bail!("Not enough data to read uint");
    }

    let mut result = 0usize;
    for i in 0..size {
        result = (result << 8) | (data[*pos + i] as usize);
    }
    *pos += size;

    Ok(result)
}

/// Converts a hex string to a BoC
pub fn hex_to_boc(hex: &str) -> Result<Arc<Cell>> {
    let hex = hex.trim().replace(" ", "").replace("\n", "");
    let bytes = hex::decode(&hex).map_err(|e| anyhow::anyhow!("Failed to decode hex: {}", e))?;
    deserialize_boc(&bytes)
}

/// Converts a BoC to a hex string
pub fn boc_to_hex(cell: &Arc<Cell>, has_crc32: bool) -> Result<String> {
    let bytes = serialize_boc(cell, has_crc32)?;
    Ok(hex::encode(bytes))
}

/// Converts a BoC to base64
pub fn boc_to_base64(cell: &Arc<Cell>, has_crc32: bool) -> Result<String> {
    use base64::Engine;
    let bytes = serialize_boc(cell, has_crc32)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Converts a base64 string to a BoC
pub fn base64_to_boc(b64: &str) -> Result<Arc<Cell>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?;
    deserialize_boc(&bytes)
}

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
