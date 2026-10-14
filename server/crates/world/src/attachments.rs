//! default gun attachments (scopes, red dots, silencers) for bought guns
//! (range_attachments.py, spec 07 2.2 / 3.3).
//!
//! the client builds a gun's attachments in AAresEquippable BeginPlay (IDA
//! sub_7FF73B7D85A0) from two arrays in the gun's actor-open bunch: handle 20
//! AttachmentDataAssets (+0xFC0) and handle 40 AttachmentDataAssetIds (+0x1028).
//! a null or unresolved element crashes the client (no null check at
//! 0x7ff73b7d8b24), so every attachment cdo is exported (ch8) before the open.

use aries_bits::{BitWriter, Bits};
use aries_rep::RepResult;
use aries_rep::block::{append_export_header, append_static_export_entry};

/// AttachmentDataAssets, TArray<UEquippableAttachmentDataAsset*> (range_attachments.py:26).
pub const ATTACHMENT_ASSETS_HANDLE: u32 = 20;
/// AttachmentDataAssetIds, TArray<FGuid>, index-aligned (range_attachments.py:27).
pub const ATTACHMENT_ASSET_IDS_HANDLE: u32 = 40;
/// bunch body guard; the client's real limit is 8064 bits (range_attachments.py:101-102).
pub const GUN_OPEN_BODY_GUARD_BITS: usize = 7600;

/// one forced attachment: its primary-asset cdo and the cdo's FGuid at +0x38.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentAsset {
    pub package_path: &'static str,
    pub cdo_name: &'static str,
    pub uuid_words: [u32; 4],
}

macro_rules! guns {
    ($path:literal) => {
        concat!("/Game/Equippables/Guns/", $path)
    };
}

const fn asset(package_path: &'static str, cdo_name: &'static str, uuid_words: [u32; 4]) -> AttachmentAsset {
    AttachmentAsset { package_path, cdo_name, uuid_words }
}

/// each gun's EquippableDataAsset ForcedAttachments, read live in TSet order
/// (range_attachments.py:38-97). the order is the send order and must be kept.
/// none: BasePistol, SawedOffShotgun, AutomaticPistol, CompactPistol,
/// RevolverPistol, PumpShotgun, AutomaticShotgun (range_attachments.py:98-99).
pub const FORCED_ATTACHMENTS: [(&str, &[AttachmentAsset]); 13] = [
    ("AssaultRifle_ACR_C", &[
        asset(guns!("Rifles/Carbine/Attachments/Reflex/RedDot/Attachment_ACR_Reflex_RedDotPrimaryAsset"),
              "Default__Attachment_ACR_Reflex_RedDotPrimaryAsset_C", [0xc119da22, 0x419d5a7d, 0x2e83ce9c, 0x814719e5]),
        asset(guns!("Rifles/Carbine/Attachments/Muzzle/Silencer/Attachment_ACR_Silencer_PrimaryAsset"),
              "Default__Attachment_ACR_Silencer_PrimaryAsset_C", [0xf1278ba5, 0x45c7e3bc, 0xb2393d9e, 0xe61b989d]),
    ]),
    ("SubMachineGun_MP5_C", &[
        asset(guns!("SubMachineGuns/MP5/Attachments/Reflex/RedDot/Attachment_MP5_Reflex_RedDotPrimaryAsset"),
              "Default__Attachment_MP5_Reflex_RedDotPrimaryAsset_C", [0xc59421d8, 0xd7dc4e75, 0x8fb99071, 0x0386599e]),
        asset(guns!("SubMachineGuns/MP5/Attachments/Muzzle/Silencer/Attachment_MP5_Silencer_PrimaryAsset"),
              "Default__Attachment_MP5_Silencer_PrimaryAsset_C", [0xa711565a, 0xe57c43c8, 0x9395e7d0, 0x9229ff26]),
    ]),
    ("AssaultRifle_AK_C", &[
        asset(guns!("Rifles/AK/Attachments/Reflex/RedDot/Attachment_AK_ReflexPrimaryAsset"),
              "Default__Attachment_AK_ReflexPrimaryAsset_C", [0xd90ff390, 0x4d4dee48, 0x8d4cbc9f, 0x7b123692]),
    ]),
    ("AssaultRifle_Burst_C", &[
        asset(guns!("Rifles/Burst/Attachments/Rfx/RedDot/Attachment_Burst_Reflex_RedDotPrimaryAsset"),
              "Default__Attachment_Burst_Reflex_RedDotPrimaryAsset_C", [0x8c505c5f, 0x4b9e7338, 0xa54f24ab, 0xa13b2b6b]),
    ]),
    ("DMR_C", &[
        asset(guns!("SniperRifles/DMR/Attachments/Reflex/ACOG/Attachment_DMR_Reflex_ReddotPrimaryAsset"),
              "Default__Attachment_DMR_Reflex_ReddotPrimaryAsset_C", [0x2cabd857, 0x439b33b4, 0xcef703a8, 0x137ed1e4]),
    ]),
    ("BattleRifle_C", &[
        asset(guns!("Rifles/BattleRifle/Attachments/Reflex/Acog/Attachment_BattleRifle_ACOGScope_PrimaryAsset"),
              "Default__Attachment_BattleRifle_ACOGScope_PrimaryAsset_C", [0x8ff81891, 0x4c1acfbc, 0x82b201ba, 0x37250b8f]),
    ]),
    ("Vector_C", &[
        asset(guns!("SubMachineGuns/Vector/Attachments/Reflex/RedDot/Attachment_Vector_Reflex_RedDotPrimaryAsset"),
              "Default__Attachment_Vector_Reflex_RedDotPrimaryAsset_C", [0xd1d44263, 0x4192cebc, 0x0974d48e, 0xf2f15e1d]),
    ]),
    ("LightMachineGun_C", &[
        asset(guns!("HvyMachineGuns/LMG/Attachments/Reflex/RedDot/Attachment_LMG_Reflex_RedDotPrimaryAsset"),
              "Default__Attachment_LMG_Reflex_RedDotPrimaryAsset_C", [0xdb9e14fd, 0x4c573fcc, 0x38170bac, 0xfa8b518c]),
    ]),
    ("HeavyMachineGun_C", &[
        asset(guns!("HvyMachineGuns/HMG/Attachments/Reflex/Attachment_HMG_ReflexPrimaryAsset"),
              "Default__Attachment_HMG_ReflexPrimaryAsset_C", [0xc1ca73bb, 0x4dcff8c0, 0xe6aacf8a, 0x51eafc97]),
    ]),
    ("LeverSniperRifle_C", &[
        asset(guns!("SniperRifles/Leversniper/Attachments/Reflex/Default/Scope_Lever_DataAsset"),
              "Default__Scope_Lever_DataAsset_C", [0x03e545a4, 0x4e1998be, 0x421cedba, 0x218fda66]),
    ]),
    ("DS_Gun_C", &[
        asset(guns!("SniperRifles/Doublesniper/Attachments/Scope/Scope_DS_DataAsset"),
              "Default__Scope_DS_DataAsset_C", [0x73dd60ef, 0x4b02a29d, 0x2c33659d, 0x3da022e4]),
    ]),
    ("BoltSniper_C", &[
        asset(guns!("SniperRifles/Boltsniper/Attachments/2XScope/Attachment_BoltSniper_2XScope_PrimaryAsset"),
              "Default__Attachment_BoltSniper_2XScope_PrimaryAsset_C", [0xe27e029d, 0x47e6ef77, 0xa21da4ba, 0x46737070]),
    ]),
    ("LugerPistol_C", &[
        asset(guns!("Sidearms/Luger/Attachments/Silencer/Attachment_Luger_Silencer_PrimaryAsset"),
              "Default__Attachment_Luger_Silencer_PrimaryAsset_C", [0x106e210f, 0x7f8f408b, 0x801081c1, 0x5ab50032]),
    ]),
];

/// python forced_attachments (range_attachments.py:105-106): empty for guns without.
pub fn forced_attachments(class_name: &str) -> &'static [AttachmentAsset] {
    FORCED_ATTACHMENTS.iter().find(|(class, _)| *class == class_name).map_or(&[], |(_, assets)| assets)
}

/// python append_object_array_property (range_attachments.py:109-117): handle,
/// u16 count, (1-based element handle, guid) per element, then the array's 0.
pub fn append_object_array_property(w: &mut BitWriter, handle: u32, guids: &[u32]) {
    w.append_packed_uint(handle as u64);
    w.append_bits(guids.len() as u64, 16);
    for (index, &guid) in guids.iter().enumerate() {
        w.append_packed_uint(index as u64 + 1);
        w.append_packed_uint(guid as u64);
    }
    w.append_packed_uint(0);
}

/// python append_guid_array_property (range_attachments.py:120-129): four u32
/// leaves per FGuid, element handles 4*i+j+1 counting on across elements.
pub fn append_guid_array_property(w: &mut BitWriter, handle: u32, words_list: &[[u32; 4]]) {
    w.append_packed_uint(handle as u64);
    w.append_bits(words_list.len() as u64, 16);
    for (index, words) in words_list.iter().enumerate() {
        for (leaf, &word) in words.iter().enumerate() {
            w.append_packed_uint(4 * index as u64 + leaf as u64 + 1);
            w.append_u32(word);
        }
    }
    w.append_packed_uint(0);
}

/// python attachment_pre_export_body (range_attachments.py:132-143): an
/// exports-only body, each attachment cdo and its package as static exports.
pub fn attachment_pre_export_body(assets: &[AttachmentAsset], guid_pairs: &[(u32, u32)]) -> RepResult<Bits> {
    if assets.is_empty() || assets.len() != guid_pairs.len() {
        return Err(aries_rep::RepError::Invalid("each attachment needs exactly one (object, package) GUID pair".into()));
    }
    let mut w = BitWriter::new();
    append_export_header(&mut w, assets.len() as u32);
    for (asset, &(object_guid, package_guid)) in assets.iter().zip(guid_pairs) {
        append_static_export_entry(&mut w, asset.cdo_name, asset.package_path, object_guid, package_guid);
    }
    Ok(w.finish()?)
}
