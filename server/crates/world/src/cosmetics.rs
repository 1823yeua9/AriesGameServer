//! cosmetic actor opens (range_weapon_cosmetics.py, spec 07 7.1-7.3): the
//! classic's initial open with the saved skin (a21), and the generic catalog
//! cosmetic open shared by the knife and every bought gun.
//!
//! the open exports the actor archetype plus the skin, level and chroma cdos in
//! one ReceiveNetGUIDBunch header, then the dynamic actor header, then one actor
//! content block with the three pointers and their FGuid leaves (and, for guns,
//! the attachment arrays) before actor setup runs the cosmetic pass.

use aries_bits::{BitWriter, Bits};
use aries_rep::block::{Target, append_dynamic_actor_header, append_export_header, append_rep_block, append_static_export_entry, dynamic_actor_reference};
use aries_rep::{RepError, RepResult};

use crate::attachments::{ATTACHMENT_ASSET_IDS_HANDLE, ATTACHMENT_ASSETS_HANDLE, GUN_OPEN_BODY_GUARD_BITS, append_guid_array_property, append_object_array_property};
use crate::skins::{CosmeticCdoPath, EquippedCdoPaths, EquippedCosmetic};

/// range_weapon_cosmetics.py:22-29.
pub const BASE_PISTOL_CHANNEL: u32 = 9;
pub const BASE_PISTOL_ACTOR_GUID: u32 = 94;
pub const BASE_PISTOL_WEAPON_ID: &str = "29a0cfab-485b-f5d5-779a-b59f85e204a8";
pub const BASE_PISTOL_ARCHETYPE_NAME: &str = "Default__BasePistol_C";
pub const BASE_PISTOL_ARCHETYPE_PACKAGE: &str = "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol";
pub const BASE_PISTOL_ARCHETYPE_GUID: u32 = 95;
pub const BASE_PISTOL_PACKAGE_GUID: u32 = 97;
pub const RANGE_LEVEL_GUID: u32 = 5;

/// skin / level / chroma UObject-pointer handles, read from the live 13.06 ch9
/// AAresEquippable FRepLayout (range_weapon_cosmetics.py:31-33).
pub const COSMETIC_POINTER_HANDLES: [u32; 3] = [18, 19, 21];
/// first of the four FGuid u32 leaves of skin / level / chroma (range_weapon_cosmetics.py:34).
pub const COSMETIC_UUID_HANDLES: [u32; 3] = [24, 28, 41];
/// odd (object, package) static guids for the classic's skin / level / chroma;
/// one asset path per pair per connection (range_weapon_cosmetics.py:36-44).
pub const BASE_PISTOL_COSMETIC_GUID_PAIRS: [(u32, u32); 3] = [(229, 231), (233, 235), (237, 239)];
/// the classic's catalog root (range_weapon_cosmetics.py:46).
const BASE_PISTOL_SOURCE_ROOT: &str = "Equippables/Guns/Sidearms/BasePistol/";
/// a21 guard: the 13-bit bunch size field (range_weapon_cosmetics.py:220-221).
const BASE_PISTOL_OPEN_LIMIT_BITS: usize = 1 << 13;

/// python guid_text_to_words (range_weapon_cosmetics.py:51-62): canonical
/// 8-4-4-4-12 hex, hyphens removed, four big-endian 8-digit groups -> four u32.
/// proven against the live primary-asset cdos' FGuid at +0x38.
pub fn guid_text_to_words(guid_text: &str) -> RepResult<[u32; 4]> {
    let bytes = guid_text.as_bytes();
    let canonical = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    if !canonical {
        return Err(RepError::Invalid("asset GUID must use canonical 8-4-4-4-12 text".into()));
    }
    let digits: String = guid_text.chars().filter(|c| *c != '-').collect();
    Ok(std::array::from_fn(|index| u32::from_str_radix(&digits[index * 8..index * 8 + 8], 16).expect("hex checked above")))
}

/// measured actor identity, netguid allocation and cosmetic wire handles
/// (range_weapon_cosmetics.py:225-244).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CosmeticActorLayout {
    pub weapon_id: String,
    pub channel_index: u32,
    pub actor_net_guid: u32,
    pub archetype_name: String,
    pub archetype_package: String,
    pub archetype_net_guid: u32,
    pub package_net_guid: u32,
    pub level_net_guid: u32,
    /// skin, level, chroma
    pub pointer_handles: [u32; 3],
    pub uuid_handles: [u32; 3],
    pub asset_guid_pairs: [(u32, u32); 3],
}

/// python base_pistol_cosmetic_layout (range_weapon_cosmetics.py:247-262).
pub fn base_pistol_cosmetic_layout() -> CosmeticActorLayout {
    CosmeticActorLayout {
        weapon_id: BASE_PISTOL_WEAPON_ID.to_string(),
        channel_index: BASE_PISTOL_CHANNEL,
        actor_net_guid: BASE_PISTOL_ACTOR_GUID,
        archetype_name: BASE_PISTOL_ARCHETYPE_NAME.to_string(),
        archetype_package: BASE_PISTOL_ARCHETYPE_PACKAGE.to_string(),
        archetype_net_guid: BASE_PISTOL_ARCHETYPE_GUID,
        package_net_guid: BASE_PISTOL_PACKAGE_GUID,
        level_net_guid: RANGE_LEVEL_GUID,
        pointer_handles: COSMETIC_POINTER_HANDLES,
        uuid_handles: COSMETIC_UUID_HANDLES,
        asset_guid_pairs: BASE_PISTOL_COSMETIC_GUID_PAIRS,
    }
}

fn invalid(message: &str) -> RepError {
    RepError::Invalid(message.to_string())
}

/// the open body both python builders write once their inputs are checked:
/// four static exports, the actor header, then the property block. handles
/// must strictly ascend in one payload, so the attachment arrays (20, 40) go in
/// where the loop meets handle 21 and handle 41 (range_weapon_cosmetics.py:306-351).
fn cosmetic_open_body(layout: &CosmeticActorLayout, resolved: &[(&CosmeticCdoPath, [u32; 4]); 3], attachments: &[(u32, [u32; 4])]) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_export_header(&mut w, 4);
    append_static_export_entry(&mut w, &layout.archetype_name, &layout.archetype_package, layout.archetype_net_guid, layout.package_net_guid);
    for ((path, _), &(object_guid, package_guid)) in resolved.iter().zip(&layout.asset_guid_pairs) {
        append_static_export_entry(&mut w, &path.object_name, &path.package_path, object_guid, package_guid);
    }
    append_dynamic_actor_header(&mut w, layout.actor_net_guid, layout.archetype_net_guid, layout.level_net_guid, None);

    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    for (&handle, &(object_guid, _)) in layout.pointer_handles.iter().zip(&layout.asset_guid_pairs) {
        if !attachments.is_empty() && handle == ATTACHMENT_ASSETS_HANDLE + 1 {
            let guids: Vec<u32> = attachments.iter().map(|&(guid, _)| guid).collect();
            append_object_array_property(&mut rep, ATTACHMENT_ASSETS_HANDLE, &guids);
        }
        rep.append_packed_uint(handle as u64);
        rep.append_packed_uint(object_guid as u64);
    }
    for (&first_handle, (_, words)) in layout.uuid_handles.iter().zip(resolved) {
        if !attachments.is_empty() && first_handle == ATTACHMENT_ASSET_IDS_HANDLE + 1 {
            let words_list: Vec<[u32; 4]> = attachments.iter().map(|&(_, words)| words).collect();
            append_guid_array_property(&mut rep, ATTACHMENT_ASSET_IDS_HANDLE, &words_list);
        }
        for (offset, &word) in words.iter().enumerate() {
            rep.append_packed_uint(first_handle as u64 + offset as u64);
            rep.append_u32(word);
        }
    }
    rep.append_packed_uint(0);
    append_rep_block(&mut w, Target::Actor, &rep.finish()?, layout.actor_net_guid)?;
    Ok(w.finish()?)
}

/// python build_catalog_cosmetic_actor_open (range_weapon_cosmetics.py:265-354):
/// a cosmetic actor open from exact catalog cdos and a measured layout.
/// attachments: (object netguid, 4 uuid words) per attachment, already exported
/// to the client; each becomes an element of handles 20 and 40.
pub fn build_catalog_cosmetic_actor_open(cdo_paths: &EquippedCdoPaths, layout: &CosmeticActorLayout, attachments: &[(u32, [u32; 4])]) -> RepResult<Bits> {
    if cdo_paths.weapon_id != layout.weapon_id {
        return Err(invalid("cosmetic and actor weapon IDs differ"));
    }
    if layout.actor_net_guid == 0 || layout.actor_net_guid & 1 != 0 {
        return Err(invalid("dynamic actor NetGUID must be positive and even"));
    }
    let mut static_guids = vec![layout.archetype_net_guid, layout.package_net_guid];
    static_guids.extend(layout.asset_guid_pairs.iter().flat_map(|&(object, package)| [object, package]));
    let distinct = static_guids.iter().enumerate().all(|(index, guid)| !static_guids[..index].contains(guid));
    if !distinct || static_guids.iter().any(|guid| *guid == 0 || guid & 1 == 0) || static_guids.contains(&layout.actor_net_guid) {
        return Err(invalid("static NetGUIDs must be distinct positive odd values"));
    }
    if layout.pointer_handles.iter().chain(&layout.uuid_handles).any(|handle| *handle == 0) {
        return Err(invalid("cosmetic handles must be positive"));
    }
    if !layout.archetype_package.starts_with("/Game/Equippables/") {
        return Err(invalid("actor archetype package is outside Equippables"));
    }
    let mut resolved = Vec::with_capacity(3);
    for (kind, path) in cdo_paths.in_order() {
        if path.kind != kind || !path.packaged {
            return Err(invalid("cosmetic CDO is missing or not packaged"));
        }
        if !path.package_path.starts_with("/Game/Equippables/") {
            return Err(invalid("cosmetic package is outside Equippables"));
        }
        if !path.object_name.starts_with("Default__") {
            return Err(invalid("cosmetic object is not a class default"));
        }
        resolved.push((path, guid_text_to_words(&path.asset_id)?));
    }
    let resolved: [(&CosmeticCdoPath, [u32; 4]); 3] = resolved.try_into().expect("three kinds");
    if !attachments.is_empty() && (layout.pointer_handles != COSMETIC_POINTER_HANDLES || layout.uuid_handles != COSMETIC_UUID_HANDLES) {
        return Err(invalid("attachment handles 20/40 are only placed for the measured AAresEquippable layout"));
    }
    let body = cosmetic_open_body(layout, &resolved, attachments)?;
    if body.bit_count > GUN_OPEN_BODY_GUARD_BITS {
        return Err(invalid("cosmetic actor-open bunch exceeds protocol size limit"));
    }
    Ok(body)
}

/// python build_base_pistol_initial_cosmetic_open with catalog cdo paths
/// (range_weapon_cosmetics.py:147-222). byte-identical to the generic open with
/// the pistol layout, but checked against the saved selection and guarded at
/// the 13-bit size field. (the cdo_paths=None branch is dead in production.)
pub fn build_base_pistol_initial_cosmetic_open(cosmetic: &EquippedCosmetic, cdo_paths: &EquippedCdoPaths) -> RepResult<Bits> {
    if cosmetic.weapon_id != BASE_PISTOL_WEAPON_ID {
        return Err(invalid("expected an equipped BasePistol cosmetic selection"));
    }
    if cdo_paths.weapon_id != cosmetic.weapon_id {
        return Err(invalid("catalog CDO paths do not match equipped weapon"));
    }
    let selected = [
        (&cosmetic.skin_asset, &cosmetic.skin_id),
        (&cosmetic.level_asset, &cosmetic.level_id),
        (&cosmetic.chroma_asset, &cosmetic.chroma_id),
    ];
    let mut resolved = Vec::with_capacity(3);
    for ((kind, path), (source_asset, uuid_text)) in cdo_paths.in_order().into_iter().zip(selected) {
        if !source_asset.starts_with(BASE_PISTOL_SOURCE_ROOT)
            || path.kind != kind
            || path.asset_id != *uuid_text
            || path.source_asset != *source_asset
            || !path.package_path.starts_with(&format!("/Game/{BASE_PISTOL_SOURCE_ROOT}"))
            || !path.packaged
        {
            return Err(invalid("catalog CDO path differs from saved selection"));
        }
        resolved.push((path, guid_text_to_words(uuid_text)?));
    }
    let resolved: [(&CosmeticCdoPath, [u32; 4]); 3] = resolved.try_into().expect("three kinds");
    let body = cosmetic_open_body(&base_pistol_cosmetic_layout(), &resolved, &[])?;
    if body.bit_count >= BASE_PISTOL_OPEN_LIMIT_BITS {
        return Err(invalid("initial BasePistol bunch exceeds protocol size limit"));
    }
    Ok(body)
}

/// the plain classic open (94 / 95 / 97 / level 5, 837 bits): the a21 fallback
/// (range_udp_handshake.py:3002-3007).
pub fn base_pistol_plain_open() -> RepResult<Bits> {
    Ok(dynamic_actor_reference(
        (BASE_PISTOL_ARCHETYPE_NAME, BASE_PISTOL_ARCHETYPE_PACKAGE),
        BASE_PISTOL_ACTOR_GUID,
        BASE_PISTOL_ARCHETYPE_GUID,
        BASE_PISTOL_PACKAGE_GUID,
        RANGE_LEVEL_GUID,
        None,
    )?)
}

/// which classic open a21 sent (python events base_pistol_initial_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasePistolOpen {
    /// the saved skin (base_pistol_initial_skin_id / _level_id / _chroma_id)
    Cosmetic,
    /// a selection exists but a part is not packaged (base_pistol_initial_cosmetic_unshipped)
    Unshipped,
    /// no classic skin equipped (base_pistol_initial_cosmetic_unselected)
    Unselected,
}

/// the a21 classic open decision (range_udp_handshake.py:2989-3007) for the
/// selection python select_base_pistol_cosmetic_loadout returned. a build error
/// propagates: python lets it kill the responder (07 open question 3).
pub fn base_pistol_open(selection: Option<&(EquippedCosmetic, EquippedCdoPaths)>) -> RepResult<(Bits, BasePistolOpen)> {
    match selection {
        Some((cosmetic, paths)) if paths.all_packaged() => Ok((build_base_pistol_initial_cosmetic_open(cosmetic, paths)?, BasePistolOpen::Cosmetic)),
        Some(_) => Ok((base_pistol_plain_open()?, BasePistolOpen::Unshipped)),
        None => Ok((base_pistol_plain_open()?, BasePistolOpen::Unselected)),
    }
}
