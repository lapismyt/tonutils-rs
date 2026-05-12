fn parse_edge<V, F>(
    slice: &mut Slice,
    remaining: usize,
    prefix: &mut Vec<bool>,
    dict: &mut HashmapE<V>,
    load_value: &F,
) -> Result<()>
where
    F: Fn(&mut Slice) -> Result<V>,
{
    let label = load_label(slice, remaining)?;
    let label_len = label.len();
    prefix.extend(label);
    let node_remaining = remaining - label_len;

    if node_remaining == 0 {
        let key = bit_vec_to_key(prefix, dict.key_bits())?;
        let value = load_value(slice)?;
        if dict.insert_bit_key(key, value)?.is_some() {
            bail!("Duplicate dictionary key");
        }
    } else {
        let left = slice.load_reference()?;
        let right = slice.load_reference()?;

        prefix.push(false);
        let mut left_slice = Slice::new(left);
        parse_edge(
            &mut left_slice,
            node_remaining - 1,
            prefix,
            dict,
            load_value,
        )?;
        prefix.pop();

        prefix.push(true);
        let mut right_slice = Slice::new(right);
        parse_edge(
            &mut right_slice,
            node_remaining - 1,
            prefix,
            dict,
            load_value,
        )?;
        prefix.pop();
    }

    prefix.truncate(prefix.len() - label_len);
    Ok(())
}

fn build_aug_node<V, E>(
    entries: &[(&BitKey, &V, &E)],
    depth: usize,
    remaining: usize,
    fork_extra: &E,
    forks: &mut Vec<HashmapAugFork<E>>,
) -> Result<HashmapAugNode<V, E>>
where
    V: Clone,
    E: Clone,
{
    if entries.is_empty() {
        bail!("Cannot build empty HashmapAug edge");
    }

    let label_len = common_aug_prefix_len(entries, depth, remaining)?;
    let label = collect_key_bits(entries[0].0, depth, label_len)?;
    let node_remaining = remaining - label_len;

    if node_remaining == 0 {
        if entries.len() != 1 {
            bail!("Duplicate dictionary key");
        }
        return Ok(HashmapAugNode::Leaf {
            label,
            extra: entries[0].2.clone(),
            value: entries[0].1.clone(),
        });
    }

    let split_depth = depth + label_len;
    let split = entries
        .iter()
        .position(|(key, _, _)| key.bit(split_depth).unwrap_or(false))
        .unwrap_or(entries.len());
    if split == 0 || split == entries.len() {
        bail!("Invalid augmented dictionary fork without both branches");
    }

    let mut prefix = Vec::with_capacity(split_depth);
    for index in 0..split_depth {
        prefix.push(entries[0].0.bit(index)?);
    }
    forks.push(HashmapAugFork {
        prefix: bit_vec_to_key(&prefix, split_depth)?,
        extra: fork_extra.clone(),
    });

    Ok(HashmapAugNode::Fork {
        label,
        left: Box::new(build_aug_node(
            &entries[..split],
            split_depth + 1,
            node_remaining - 1,
            fork_extra,
            forks,
        )?),
        right: Box::new(build_aug_node(
            &entries[split..],
            split_depth + 1,
            node_remaining - 1,
            fork_extra,
            forks,
        )?),
        extra: fork_extra.clone(),
    })
}

fn deserialize_hashmap_aug_from_slice<V, E, FV, FE>(
    slice: &mut Slice,
    key_bits: usize,
    load_value: FV,
    load_extra: FE,
) -> Result<HashmapAug<V, E>>
where
    V: Clone,
    E: Clone,
    FV: Fn(&mut Slice) -> Result<V>,
    FE: Fn(&mut Slice) -> Result<E>,
{
    let mut leaves = BTreeMap::new();
    let mut forks = Vec::new();
    let root = parse_aug_node(
        slice,
        key_bits,
        &mut Vec::new(),
        key_bits,
        &mut leaves,
        &mut forks,
        &load_value,
        &load_extra,
    )?;
    Ok(HashmapAug {
        key_bits,
        root,
        leaves,
        forks,
    })
}

#[allow(clippy::too_many_arguments)]
fn parse_aug_node<V, E, FV, FE>(
    slice: &mut Slice,
    remaining: usize,
    prefix: &mut Vec<bool>,
    key_bits: usize,
    leaves: &mut BTreeMap<BitKey, (V, E)>,
    forks: &mut Vec<HashmapAugFork<E>>,
    load_value: &FV,
    load_extra: &FE,
) -> Result<HashmapAugNode<V, E>>
where
    V: Clone,
    E: Clone,
    FV: Fn(&mut Slice) -> Result<V>,
    FE: Fn(&mut Slice) -> Result<E>,
{
    let label = load_label(slice, remaining)?;
    let label_len = label.len();
    prefix.extend(label.iter().copied());
    let node_remaining = remaining - label_len;

    let node = if node_remaining == 0 {
        let key = bit_vec_to_key(prefix, key_bits)?;
        let extra = load_extra(slice)?;
        let value = load_value(slice)?;
        if leaves
            .insert(key.clone(), (value.clone(), extra.clone()))
            .is_some()
        {
            bail!("Duplicate dictionary key");
        }
        HashmapAugNode::Leaf {
            label,
            extra,
            value,
        }
    } else {
        let fork_prefix = bit_vec_to_key(prefix, prefix.len())?;
        let left = slice.load_reference()?;
        let right = slice.load_reference()?;

        prefix.push(false);
        let mut left_slice = Slice::new(left);
        let left_node = parse_aug_node(
            &mut left_slice,
            node_remaining - 1,
            prefix,
            key_bits,
            leaves,
            forks,
            load_value,
            load_extra,
        )?;
        ensure_aug_ref_consumed(&left_slice)?;
        prefix.pop();

        prefix.push(true);
        let mut right_slice = Slice::new(right);
        let right_node = parse_aug_node(
            &mut right_slice,
            node_remaining - 1,
            prefix,
            key_bits,
            leaves,
            forks,
            load_value,
            load_extra,
        )?;
        ensure_aug_ref_consumed(&right_slice)?;
        prefix.pop();

        let extra = load_extra(slice)?;
        forks.push(HashmapAugFork {
            prefix: fork_prefix,
            extra: extra.clone(),
        });
        HashmapAugNode::Fork {
            label,
            left: Box::new(left_node),
            right: Box::new(right_node),
            extra,
        }
    };

    prefix.truncate(prefix.len() - label_len);
    Ok(node)
}

fn ensure_aug_ref_consumed(slice: &Slice) -> Result<()> {
    if slice.is_empty() {
        Ok(())
    } else {
        bail!(
            "Trailing data in HashmapAug reference: {} bits and {} refs remaining",
            slice.remaining_bits(),
            slice.remaining_refs()
        );
    }
}

fn store_aug_node<V, E, FV, FE>(
    builder: &mut Builder,
    node: &HashmapAugNode<V, E>,
    store_value: &FV,
    store_extra: &FE,
) -> Result<()>
where
    FV: Fn(&mut Builder, &V) -> Result<()>,
    FE: Fn(&mut Builder, &E) -> Result<()>,
{
    match node {
        HashmapAugNode::Leaf {
            label,
            extra,
            value,
        } => {
            store_label(builder, label, label.len())?;
            store_extra(builder, extra)?;
            store_value(builder, value)?;
        }
        HashmapAugNode::Fork {
            label,
            left,
            right,
            extra,
        } => {
            let node_remaining = node_remaining_after_label(node)?;
            store_label(builder, label, node_remaining + label.len())?;

            let mut left_builder = Builder::new();
            store_aug_node(&mut left_builder, left, store_value, store_extra)?;
            builder.store_ref(left_builder.build()?)?;

            let mut right_builder = Builder::new();
            store_aug_node(&mut right_builder, right, store_value, store_extra)?;
            builder.store_ref(right_builder.build()?)?;
            store_extra(builder, extra)?;
        }
    }
    Ok(())
}

fn node_remaining_after_label<V, E>(node: &HashmapAugNode<V, E>) -> Result<usize> {
    match node {
        HashmapAugNode::Leaf { .. } => Ok(0),
        HashmapAugNode::Fork { left, right, .. } => {
            Ok(1 + total_aug_edge_bits(left)?.max(total_aug_edge_bits(right)?))
        }
    }
}

fn total_aug_edge_bits<V, E>(node: &HashmapAugNode<V, E>) -> Result<usize> {
    match node {
        HashmapAugNode::Leaf { label, .. } => Ok(label.len()),
        HashmapAugNode::Fork { label, left, .. } => {
            Ok(label.len() + 1 + total_aug_edge_bits(left)?)
        }
    }
}

fn common_aug_prefix_len<V, E>(
    entries: &[(&BitKey, &V, &E)],
    depth: usize,
    max_len: usize,
) -> Result<usize> {
    let first = entries[0].0;
    let mut len = 0usize;
    'outer: while len < max_len {
        let bit = first.bit(depth + len)?;
        for (key, _, _) in &entries[1..] {
            if key.bit(depth + len)? != bit {
                break 'outer;
            }
        }
        len += 1;
    }
    Ok(len)
}

fn store_label(builder: &mut Builder, bits: &[bool], max_len: usize) -> Result<()> {
    let encoded = canonical_label(bits, max_len)?;
    for bit in encoded {
        builder.store_bit(bit)?;
    }
    Ok(())
}

fn load_label(slice: &mut Slice, max_len: usize) -> Result<Vec<bool>> {
    let first = slice.load_bit()?;
    if !first {
        let len = load_unary(slice, max_len)?;
        let mut bits = Vec::with_capacity(len);
        for _ in 0..len {
            bits.push(slice.load_bit()?);
        }
        return Ok(bits);
    }

    let second = slice.load_bit()?;
    let width = label_len_width(max_len);
    if !second {
        let len = load_label_len(slice, width)?;
        if len > max_len {
            bail!("Long Hashmap label length exceeds remaining key bits");
        }
        let mut bits = Vec::with_capacity(len);
        for _ in 0..len {
            bits.push(slice.load_bit()?);
        }
        Ok(bits)
    } else {
        let value = slice.load_bit()?;
        let len = load_label_len(slice, width)?;
        if len > max_len {
            bail!("Same Hashmap label length exceeds remaining key bits");
        }
        Ok(vec![value; len])
    }
}

fn canonical_label(bits: &[bool], max_len: usize) -> Result<Vec<bool>> {
    if bits.len() > max_len {
        bail!("Hashmap label length exceeds remaining key bits");
    }

    let mut candidates = vec![encode_short_label(bits), encode_long_label(bits, max_len)?];
    if bits.iter().all(|bit| *bit == false) || bits.iter().all(|bit| *bit == true) {
        candidates.push(encode_same_label(bits, max_len)?);
    }
    candidates.sort_by(|left, right| left.len().cmp(&right.len()).then_with(|| left.cmp(right)));
    Ok(candidates.remove(0))
}

fn encode_short_label(bits: &[bool]) -> Vec<bool> {
    let mut encoded = Vec::with_capacity(2 + bits.len() * 2);
    encoded.push(false);
    encoded.extend(std::iter::repeat(true).take(bits.len()));
    encoded.push(false);
    encoded.extend_from_slice(bits);
    encoded
}

fn encode_long_label(bits: &[bool], max_len: usize) -> Result<Vec<bool>> {
    let width = label_len_width(max_len);
    let mut encoded = Vec::with_capacity(2 + width + bits.len());
    encoded.push(true);
    encoded.push(false);
    push_uint_bits(&mut encoded, bits.len(), width)?;
    encoded.extend_from_slice(bits);
    Ok(encoded)
}

fn encode_same_label(bits: &[bool], max_len: usize) -> Result<Vec<bool>> {
    let width = label_len_width(max_len);
    let mut encoded = Vec::with_capacity(3 + width);
    encoded.push(true);
    encoded.push(true);
    encoded.push(bits.first().copied().unwrap_or(false));
    push_uint_bits(&mut encoded, bits.len(), width)?;
    Ok(encoded)
}

fn load_unary(slice: &mut Slice, max_len: usize) -> Result<usize> {
    let mut len = 0usize;
    loop {
        let bit = slice.load_bit()?;
        if !bit {
            return Ok(len);
        }
        len += 1;
        if len > max_len {
            bail!("Short Hashmap label length exceeds remaining key bits");
        }
    }
}

fn load_label_len(slice: &mut Slice, width: usize) -> Result<usize> {
    let mut value = 0usize;
    for _ in 0..width {
        value <<= 1;
        if slice.load_bit()? {
            value |= 1;
        }
    }
    Ok(value)
}

fn label_len_width(max_len: usize) -> usize {
    let mut width = 0usize;
    let mut value = max_len;
    while value > 0 {
        width += 1;
        value >>= 1;
    }
    width
}

fn push_uint_bits(bits: &mut Vec<bool>, value: usize, width: usize) -> Result<()> {
    if width < usize::BITS as usize && value >= (1usize << width) {
        bail!("Value {} does not fit in {} bits", value, width);
    }
    for shift in (0..width).rev() {
        bits.push(((value >> shift) & 1) != 0);
    }
    Ok(())
}

fn common_prefix_len<V>(entries: &[(&BitKey, &V)], depth: usize, max_len: usize) -> Result<usize> {
    let first = entries[0].0;
    let mut len = 0usize;
    'outer: while len < max_len {
        let bit = first.bit(depth + len)?;
        for (key, _) in &entries[1..] {
            if key.bit(depth + len)? != bit {
                break 'outer;
            }
        }
        len += 1;
    }
    Ok(len)
}

fn collect_key_bits(key: &BitKey, offset: usize, len: usize) -> Result<Vec<bool>> {
    let mut bits = Vec::with_capacity(len);
    for index in offset..offset + len {
        bits.push(key.bit(index)?);
    }
    Ok(bits)
}

fn bit_vec_to_key(bits: &[bool], bit_len: usize) -> Result<BitKey> {
    if bits.len() != bit_len {
        bail!(
            "Decoded key length {} does not match {}",
            bits.len(),
            bit_len
        );
    }
    let mut data = vec![0u8; bits_to_bytes(bit_len)];
    for (index, bit) in bits.iter().copied().enumerate() {
        set_bit(&mut data, index, bit);
    }
    BitKey::new(data, bit_len)
}

fn get_bit(data: &[u8], index: usize) -> bool {
    (data[index / 8] >> (7 - (index % 8))) & 1 == 1
}

fn set_bit(data: &mut [u8], index: usize, bit: bool) {
    if bit {
        data[index / 8] |= 1 << (7 - (index % 8));
    }
}

fn bits_to_bytes(bits: usize) -> usize {
    (bits + 7) / 8
}

fn clear_unused_bits(data: &mut [u8], bit_len: usize) {
    if data.is_empty() {
        return;
    }
    let unused = data.len() * 8 - bit_len;
    if unused > 0 {
        data[data.len() - 1] &= !((1u8 << unused) - 1);
    }
}
