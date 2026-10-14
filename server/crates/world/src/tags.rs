//! gameplay-tag net indices (gameplay_tag_net_index.py, spec 08 section 3).
//!
//! with FastReplication a FGameplayTag goes on the wire as its net index
//! (IDA sub_7FF7389AA370 -> SerializeTagNetIndexPacked sub_7FF7389A7EE0). the
//! client builds the index lazily (ConstructNetIndex sub_7FF7389B0590): node-map
//! values sorted by tag name, then each CommonlyReplicatedTags entry swapped to
//! the front in order. the node list and the settings come from the oracle;
//! this module is the pure computation predict_net_indices does on them.
//!
//! gtni = gameplay_tag_net_index.py.

use std::collections::HashMap;

/// gtni:23-26: what the oracle reads (manager node map, common tags, settings,
/// node NetIndex); kept here next to the computation that consumes them.
pub const MANAGER_NODE_MAP: u64 = 0x130;
pub const MANAGER_COMMON_TAGS: u64 = 0x180;
pub const SETTINGS_FIRST_BIT_SEGMENT: u64 = 0xBC;
/// gtni:100: a node's u16 NetIndex, 0xffff until the client constructs its index.
pub const NODE_NET_INDEX: u64 = 0x50;
pub const UNCONSTRUCTED_NET_INDEX: u16 = 0xFFFF;

/// one GameplayTagNodeMap value as the oracle decodes it (gtni:74-84).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagNode {
    /// complete tag name (the map key's FName text)
    pub name: String,
    /// the FName number (sort tie-break)
    pub number: u32,
    /// node+0x50
    pub net_index: u16,
}

/// gtni:29-34
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagNetIndex {
    /// complete tag name -> net index
    pub indices: HashMap<String, u32>,
    pub true_bit_num: u32,
    pub first_bit_segment: i64,
    /// the client had already built its index (its own NetIndex values are used)
    pub constructed: bool,
}

impl TagNetIndex {
    /// ruh:4061-4062: only the plain TrueBitNum-bit encoding is supported.
    pub fn is_plain(&self) -> bool {
        self.first_bit_segment >= self.true_bit_num as i64
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TagError {
    #[error("gameplay tag node map is empty")]
    Empty,
    #[error("common tag {name} sits past the end of the tag list (slot {slot})")]
    CommonPastEnd { name: String, slot: usize },
}

/// gtni:97: TrueBitNum = ceil(log2(n + 1)), i.e. the bit length of n.
pub fn true_bit_num(tag_count: usize) -> u32 {
    usize::BITS - tag_count.leading_zeros()
}

/// gtni:87: node-map values sorted by (name.lower(), number). the sort is stable,
/// so ties keep node-map order like python's list.sort.
fn sorted_nodes(nodes: &[TagNode]) -> Vec<&TagNode> {
    let mut sorted: Vec<&TagNode> = nodes.iter().collect();
    sorted.sort_by_cached_key(|node| (node.name.to_lowercase(), node.number));
    sorted
}

/// gtni:87-96: complete names in net-index order: the sorted names, then each
/// common tag swapped (not moved) with the slot of its position in the common
/// list; missing ones are skipped.
pub fn net_index_order(nodes: &[TagNode], common_tags: &[String]) -> Result<Vec<String>, TagError> {
    let mut order: Vec<String> = sorted_nodes(nodes).into_iter().map(|node| node.name.clone()).collect();
    for (slot, name) in common_tags.iter().enumerate() {
        let Some(found) = order.iter().position(|entry| entry == name) else { continue };
        if slot >= order.len() {
            return Err(TagError::CommonPastEnd { name: name.clone(), slot });
        }
        order.swap(found, slot);
    }
    Ok(order)
}

/// gtni:68-106 after the reads: the index map, TrueBitNum, FirstBitSegment
/// (min with the settings value) and whether the client's own NetIndex values
/// (node+0x50) replace the computed order. `nodes` is the node map in its
/// storage order (free slots dropped); `first_bit_segment_setting` is settings+0xBC.
/// the read itself must refuse a partial map (gtni:85-86 raises when the
/// decoded nodes are fewer than the map's count), so no index is cached then.
pub fn predict_net_indices(nodes: &[TagNode], common_tags: &[String], first_bit_segment_setting: i32) -> Result<TagNetIndex, TagError> {
    let order = net_index_order(nodes, common_tags)?;
    let true_bit_num = true_bit_num(order.len());
    let first_bit_segment = (true_bit_num as i64).min(first_bit_segment_setting as i64);
    // gtni:99: the first node in sorted order says whether the client built its index
    let sorted = sorted_nodes(nodes);
    let first = sorted.first().ok_or(TagError::Empty)?;
    let constructed = first.net_index != UNCONSTRUCTED_NET_INDEX;
    // later duplicates overwrite earlier ones, like the python dict comprehensions
    let mut indices = HashMap::with_capacity(order.len());
    if constructed {
        for node in &sorted {
            indices.insert(node.name.clone(), node.net_index as u32);
        }
    } else {
        for (index, name) in order.into_iter().enumerate() {
            indices.insert(name, index as u32);
        }
    }
    Ok(TagNetIndex { indices, true_bit_num, first_bit_segment, constructed })
}
