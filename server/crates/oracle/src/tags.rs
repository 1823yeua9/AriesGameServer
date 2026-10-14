//! gameplay tag list for the tag net index (spec 09 R16, gameplay_tag_net_index.py).
//! with FastReplication a FGameplayTag goes on the wire as its net index; the client
//! builds that index lazily (GTN:1-12), so we either read the built one or predict it.

use std::collections::HashMap;

use crate::objects::NamePool;
use crate::world::{HeaderRule, at, require_object_array};
use crate::{Memory, OracleError};

/// GameplayTagsManager +0x130 GameplayTagNodeMap, +0x180 CommonlyReplicatedTags (GTN:23-24)
const MANAGER_NODE_MAP: u64 = 0x130;
const MANAGER_COMMON_TAGS: u64 = 0x180;
/// GameplayTagsSettings +0xBC NetIndexFirstBitSegment (GTN:26)
const SETTINGS_FIRST_BIT_SEGMENT: u64 = 0xBC;
/// node map entries: +0 comparison, +4 number, +0x10 node (GTN:76-77)
const NODE_ENTRY: u64 = 0x28;
/// common tags are 12-byte FGameplayTags, comparison first (GTN:92)
const COMMON_ENTRY: u64 = 12;
/// tag node +0x50 NetIndex, 0xFFFF until built (GTN:100)
const NODE_NET_INDEX: u64 = 0x50;
const NOT_BUILT: u16 = 0xFFFF;
/// the GObjects window searched for the manager and the settings CDO (GTN:48)
const SCAN_FIRST: u32 = 0xEA40;
const SCAN_END: u32 = 0xFB00;
/// GTN:37-39 reads the node map in pieces of this size
const READ_PIECE: u64 = 0x1000;

/// GTN:29-34
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagNetIndex {
    /// complete tag name -> net index
    pub indices: HashMap<String, u32>,
    pub true_bit_num: u32,
    pub first_bit_segment: i32,
    /// the client already built its index (node +0x50 != 0xFFFF on the first sorted tag)
    pub constructed: bool,
}

/// the responder's R16 read (H:4054-4059): its own temporary EquippableStateReader,
/// so a fresh name key derivation (FNP:43-64, with every failure that brings), then
/// predict_net_indices with that key. nothing is cached across calls.
pub fn read_tag_net_indices(memory: &impl Memory) -> Result<TagNetIndex, OracleError> {
    let mut names = crate::client::derive_names(memory)?;
    predict_net_indices(memory, &mut names)
}

/// find_tags_manager (GTN:42-65): (manager, settings). a read failure on an object
/// slot propagates; anything that fails while naming an object skips it.
pub fn find_tags_manager(memory: &impl Memory, names: &mut NamePool) -> Result<(u64, u64), OracleError> {
    // GTN reads every chunk pointer with no check of its own; the fresh reader it runs
    // after has just applied the FNP rule to this header (H:4054-4057)
    let objects = require_object_array(memory, HeaderRule::NameKey)?;
    let mut manager = 0u64;
    let mut settings = 0u64;
    for index in SCAN_FIRST..SCAN_END {
        let object = objects.object(memory, index)?;
        if object == 0 {
            continue;
        }
        let named = (|| -> Result<(String, String), OracleError> {
            let name = names.name(memory, memory.u32(at(object, 0x18))?)?;
            let class = memory.u64(at(object, 0x10))?;
            let class_name = names.name(memory, memory.u32(at(class, 0x18))?)?;
            Ok((name, class_name))
        })();
        let Ok((name, class_name)) = named else {
            continue;
        };
        if class_name == "GameplayTagsSettings" && name.starts_with("Default__") {
            settings = object;
        } else if class_name == "GameplayTagsManager" && !name.starts_with("Default__") {
            manager = object;
        }
        if manager != 0 && settings != 0 {
            break;
        }
    }
    if manager == 0 || settings == 0 {
        return Err(OracleError::Invalid("gameplay tags manager/settings not found"));
    }
    Ok((manager, settings))
}

/// GTN:37-39: `size` bytes in 0x1000-byte reads. grows as it reads: the size comes
/// from client memory, and a junk capacity must fail on its first bad piece like the
/// python, not abort on one huge allocation.
fn read_block(memory: &impl Memory, address: u64, size: u64) -> Result<Vec<u8>, OracleError> {
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < size {
        out.extend(memory.read_bytes(at(address, offset), READ_PIECE.min(size - offset) as usize)?);
        offset += READ_PIECE;
    }
    Ok(out)
}

/// code points rust's tables (unicode 17) lowercase but python 3.12's (unicode 15.0)
/// leave alone (unassigned there). generated from both; tests/reads_golden.rs checks
/// every code point against python, so a toolchain with newer tables fails there.
const NEWER_LOWERCASE: [(char, char); 9] = [
    ('\u{1C89}', '\u{1C89}'),
    ('\u{A7CB}', '\u{A7CC}'),
    ('\u{A7CE}', '\u{A7CE}'),
    ('\u{A7D2}', '\u{A7D2}'),
    ('\u{A7D4}', '\u{A7D4}'),
    ('\u{A7DA}', '\u{A7DA}'),
    ('\u{A7DC}', '\u{A7DC}'),
    ('\u{10D50}', '\u{10D65}'),
    ('\u{16EA0}', '\u{16EB8}'),
];

/// code points whose final-sigma class differs between python 3.12 and rust, with a
/// stand-in of python's class: ' ' neither, '.' case-ignorable, 'A' cased. CPython's
/// handle_capital_sigma and rust's map_uppercase_sigma run the same rule on those
/// two properties, so only these change the outcome. covers NEWER_LOWERCASE too.
const SIGMA_CONTEXT: [(char, char, char); 40] = [
    ('\u{295}', '\u{295}', 'A'),
    ('\u{897}', '\u{897}', ' '),
    ('\u{1ACF}', '\u{1ADD}', ' '),
    ('\u{1AE0}', '\u{1AEB}', ' '),
    ('\u{1C89}', '\u{1C8A}', ' '),
    ('\u{A7CB}', '\u{A7CF}', ' '),
    ('\u{A7D2}', '\u{A7D2}', ' '),
    ('\u{A7D4}', '\u{A7D4}', ' '),
    ('\u{A7DA}', '\u{A7DC}', ' '),
    ('\u{A7F1}', '\u{A7F1}', ' '),
    ('\u{10D4E}', '\u{10D4E}', ' '),
    ('\u{10D50}', '\u{10D65}', ' '),
    ('\u{10D69}', '\u{10D6D}', ' '),
    ('\u{10D6F}', '\u{10D85}', ' '),
    ('\u{10EC5}', '\u{10EC5}', ' '),
    ('\u{10EFA}', '\u{10EFC}', ' '),
    ('\u{113BB}', '\u{113C0}', ' '),
    ('\u{113CE}', '\u{113CE}', ' '),
    ('\u{113D0}', '\u{113D0}', ' '),
    ('\u{113D2}', '\u{113D2}', ' '),
    ('\u{113E1}', '\u{113E2}', ' '),
    ('\u{1171E}', '\u{1171E}', '.'),
    ('\u{11B60}', '\u{11B60}', ' '),
    ('\u{11B62}', '\u{11B64}', ' '),
    ('\u{11B66}', '\u{11B66}', ' '),
    ('\u{11DD9}', '\u{11DD9}', ' '),
    ('\u{11F5A}', '\u{11F5A}', ' '),
    ('\u{1611E}', '\u{16129}', ' '),
    ('\u{1612D}', '\u{1612F}', ' '),
    ('\u{16D40}', '\u{16D42}', ' '),
    ('\u{16D6B}', '\u{16D6C}', ' '),
    ('\u{16EA0}', '\u{16EB8}', ' '),
    ('\u{16EBB}', '\u{16ED3}', ' '),
    ('\u{16FF2}', '\u{16FF3}', ' '),
    ('\u{1E5EE}', '\u{1E5EF}', ' '),
    ('\u{1E6E3}', '\u{1E6E3}', ' '),
    ('\u{1E6E6}', '\u{1E6E6}', ' '),
    ('\u{1E6EE}', '\u{1E6EF}', ' '),
    ('\u{1E6F5}', '\u{1E6F5}', ' '),
    ('\u{1E6FF}', '\u{1E6FF}', ' '),
];

const CAPITAL_SIGMA: char = '\u{3A3}';

fn newer_lowercase(c: char) -> bool {
    NEWER_LOWERCASE.iter().any(|&(first, last)| (first..=last).contains(&c))
}

fn sigma_stand_in(c: char) -> Option<char> {
    SIGMA_CONTEXT.iter().find(|&&(first, last, _)| (first..=last).contains(&c)).map(|&(_, _, stand_in)| stand_in)
}

/// python 3.12 str.lower, the tag sort key (GTN:87), over rust's newer tables.
pub fn python_lower(text: &str) -> String {
    if !text.chars().any(|c| sigma_stand_in(c).is_some()) {
        return text.to_lowercase();
    }
    // rust decides each sigma on a copy where python's properties stand in
    let context: String = text.chars().map(|c| sigma_stand_in(c).unwrap_or(c)).collect();
    let lowered_context = context.to_lowercase();
    let mut lowered = lowered_context.chars();
    let mut sigmas = Vec::new();
    for c in context.chars() {
        if c == CAPITAL_SIGMA {
            sigmas.extend(lowered.next());
        } else {
            lowered.by_ref().take(c.to_lowercase().count()).for_each(drop);
        }
    }
    let mut sigmas = sigmas.into_iter();
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == CAPITAL_SIGMA {
            out.push(sigmas.next().unwrap_or('\u{3C3}'));
        } else if newer_lowercase(c) {
            out.push(c);
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

struct NodeEntry {
    name: String,
    number: u32,
    node: u64,
}

/// predict_net_indices (GTN:68-106).
pub fn predict_net_indices(memory: &impl Memory, names: &mut NamePool) -> Result<TagNetIndex, OracleError> {
    let (manager, settings) = find_tags_manager(memory, names)?;
    let data = memory.u64(at(manager, MANAGER_NODE_MAP))?;
    let count = memory.u32(at(manager, MANAGER_NODE_MAP + 8))?;
    let capacity = memory.u32(at(manager, MANAGER_NODE_MAP + 12))?;
    let raw = read_block(memory, data, capacity as u64 * NODE_ENTRY)?;
    let mut entries = Vec::new();
    for chunk in raw.chunks_exact(NODE_ENTRY as usize) {
        let comparison = u32::from_le_bytes(chunk[0..4].try_into().unwrap());
        let number = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
        let node = u64::from_le_bytes(chunk[0x10..0x18].try_into().unwrap());
        if node == 0 {
            continue;
        }
        // an entry whose name cannot be decoded is skipped (GTN:80-83)
        let Ok(name) = names.name(memory, comparison) else {
            continue;
        };
        entries.push(NodeEntry { name, number, node });
    }
    if entries.len() != count as usize {
        return Err(OracleError::Invalid("node map tag count mismatch"));
    }
    // FName order: case-insensitive text, then number (python str.lower, stable sort)
    entries.sort_by_cached_key(|entry| (python_lower(&entry.name), entry.number));
    let mut order: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    // each commonly replicated tag is swapped to the front in order (GTN:89-96)
    let common = memory.u64(at(manager, MANAGER_COMMON_TAGS))?;
    let common_count = memory.u32(at(manager, MANAGER_COMMON_TAGS + 8))?;
    for common_index in 0..common_count as usize {
        let comparison = memory.u32(at(common, COMMON_ENTRY * common_index as u64))?;
        let name = names.name(memory, comparison)?;
        if let Some(found) = order.iter().position(|tag| *tag == name) {
            if common_index >= order.len() {
                return Err(OracleError::Invalid("common tag index past the tag list"));
            }
            order.swap(found, common_index);
        }
    }
    // ceil(log2(n + 1)) is the bit length of n
    let true_bit_num = usize::BITS - order.len().leading_zeros();
    let first_segment_setting = memory.i32(at(settings, SETTINGS_FIRST_BIT_SEGMENT))?;
    let first_bit_segment = (true_bit_num as i32).min(first_segment_setting);
    let first = entries.first().ok_or(OracleError::Invalid("gameplay tag node map is empty"))?;
    let constructed = memory.u16(at(first.node, NODE_NET_INDEX))? != NOT_BUILT;
    let mut indices = HashMap::new();
    if constructed {
        for entry in &entries {
            indices.insert(entry.name.clone(), memory.u16(at(entry.node, NODE_NET_INDEX))? as u32);
        }
    } else {
        for (index, name) in order.into_iter().enumerate() {
            indices.insert(name, index as u32);
        }
    }
    Ok(TagNetIndex { indices, true_bit_num, first_bit_segment, constructed })
}
