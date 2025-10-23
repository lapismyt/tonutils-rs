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
    let root_index = cells.iter().position(|cell| cell.hash() == root.hash())
        .ok_or_else(|| anyhow::anyhow!("Root cell not found in collected cells"))?;

    // Serialize each cell
    let mut serialized_cells = Vec::new();
    let mut cell_map = HashMap::new();

    for (idx, cell) in cells.iter().enumerate() {
        cell_map.insert(cell_hash(cell), idx);
        serialized_cells.push(serialize_cell(cell, &cell_map)?);
    }

    // Calculate total size of serialized cells
    let cells_size: usize = serialized_cells.iter().map(|c| c.len()).sum();

    // Determine size parameters
    let size_bytes = bytes_needed(cells.len());
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

    let _has_idx = (flags_and_size & 0x80) != 0;
    let has_crc32 = (flags_and_size & 0x40) != 0;
    let _has_cache_bits = (flags_and_size & 0x20) != 0;
    let size_bytes = (flags_and_size & 0x07) as usize;

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

    // Parse cells
    let cells_start = pos;
    let cells_end = cells_start + cells_size;

    if cells_end > data.len() - if has_crc32 { 4 } else { 0 } {
        bail!("Invalid cells size");
    }

    let cells_data = &data[cells_start..cells_end];
    let cells = parse_cells(cells_data, cells_count)?;

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
    if root_idx >= cells.len() {
        bail!("Invalid root index: {}", root_idx);
    }

    Ok(cells[root_idx].clone())
}

fn parse_cells(data: &[u8], count: usize) -> Result<Vec<Arc<Cell>>> {
    let mut cells = Vec::with_capacity(count);
    let mut cell_refs: Vec<Vec<usize>> = Vec::with_capacity(count);
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
        let _is_exotic = (d1 & 0x08) != 0;
        let _level = (d1 >> 5) & 0x03;

        // Parse descriptor 2
        // d2 = floor(b/8) + ceil(b/8) where b is the number of bits
        // This means: for full bytes, d2 = 2*bytes; for partial bytes, d2 = 2*bytes + 1
        // So actual data size in bytes = ceil(d2/2)
        let data_size = (d2 as usize + 1) / 2;

        // Read cell data
        if pos + data_size > data.len() {
            bail!("Cell data exceeds buffer");
        }

        let cell_data = data[pos..pos + data_size].to_vec();
        pos += data_size;

        // Read reference indices
        let mut refs = Vec::new();
        for _ in 0..ref_count {
            if pos >= data.len() {
                bail!("Unexpected end of cells data while reading references");
            }
            refs.push(data[pos] as usize);
            pos += 1;
        }
        cell_refs.push(refs);

        // Calculate bit length from descriptor d2
        // d2 = floor(b/8) + ceil(b/8)
        // For b bits: if b % 8 == 0, then d2 = 2*(b/8), so b = d2*4
        //             if b % 8 != 0, then d2 = 2*floor(b/8) + 1, so b is between (d2-1)*4 and d2*4
        // We need to find the exact bit length by looking at the padding bit
        let bit_len = if data_size > 0 && d2 > 0 {
            // Check if we have full bytes (d2 is even) or partial byte (d2 is odd)
            if d2 % 2 == 0 {
                // Full bytes, no padding needed
                (d2 as usize / 2) * 8
            } else {
                // Partial byte - find the padding bit
                let last_byte = cell_data[cell_data.len() - 1];
                let mut bits_in_last_byte = 0;

                // Find the rightmost 1 bit (padding marker) from right to left
                for i in 0..8 {
                    if (last_byte >> i) & 1 == 1 {
                        bits_in_last_byte = 8 - i;
                        break;
                    }
                }

                // Total bits = full bytes + bits in last byte - 1 (for padding bit)
                if bits_in_last_byte > 0 {
                    (cell_data.len() - 1) * 8 + bits_in_last_byte - 1
                } else {
                    // No padding bit found, assume full bytes
                    cell_data.len() * 8
                }
            }
        } else {
            0
        };

        // Create cell (without references for now)
        let cell = Cell::with_data(cell_data, bit_len)?;
        cells.push(Arc::new(cell));
    }

    // Second pass: resolve references
    for (i, refs) in cell_refs.iter().enumerate() {
        if !refs.is_empty() {
            // We need to create a new cell with references
            // Since Cell doesn't allow modification after creation, we need to rebuild
            let old_cell = &cells[i];
            let mut new_cell = Cell::with_data(old_cell.data().to_vec(), old_cell.bit_len())?;

            for &ref_idx in refs {
                if ref_idx >= cells.len() {
                    bail!("Invalid reference index: {}", ref_idx);
                }
                new_cell.add_reference(cells[ref_idx].clone())?;
            }

            cells[i] = Arc::new(new_cell);
        }
    }

    Ok(cells)
}

fn serialize_cell(cell: &Arc<Cell>, cell_map: &HashMap<[u8; 32], usize>) -> Result<Vec<u8>> {
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

        // Write reference index (size depends on total cell count)
        // For simplicity, using 1 byte for now
        result.push(*ref_idx as u8);
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
    use crate::tvm::cell::CellBuilder;

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
}
