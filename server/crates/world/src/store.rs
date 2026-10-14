//! the range shop (range_store.py): BombGameStore_C items keyed by equippable
//! class, class-path helpers, and the repeat-purchase reference resolution.
//! spec docs/port/07_gameplay_guns.md 2.1, 6.1.

use std::collections::HashMap;

/// EAresItemSlot values the store items occupy (range_store.py:12).
pub const SLOT_PRIMARY: u32 = 0;
pub const SLOT_SECONDARY: u32 = 1;
pub const SLOT_ARMOR: u32 = 11;

/// one purchasable (range_store.py:15-22). cost is never read by the server:
/// infinite money is a GameState switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreItem {
    pub display_name: &'static str,
    /// live UClass name, e.g. "BattleRifle_C"
    pub class_name: &'static str,
    pub cost: u32,
    /// equippable / armor uuid; matches skin_catalog.json weapon ids
    pub weapon_id: &'static str,
    pub slot: u32,
    pub is_gun: bool,
}

const fn item(display_name: &'static str, class_name: &'static str, cost: u32, weapon_id: &'static str, slot: u32, is_gun: bool) -> StoreItem {
    StoreItem { display_name, class_name, cost, weapon_id, slot, is_gun }
}

/// range_store.py:25-49, python order.
pub const STORE_ITEMS: [StoreItem; 23] = [
    item("Classic", "BasePistol_C", 0, "29a0cfab-485b-f5d5-779a-b59f85e204a8", SLOT_SECONDARY, true),
    item("Shorty", "SawedOffShotgun_C", 300, "42da8ccc-40d5-affc-beec-15aa47b42eda", SLOT_SECONDARY, true),
    item("Frenzy", "AutomaticPistol_C", 450, "44d4e95c-4157-0037-81b2-17841bf2e8e3", SLOT_SECONDARY, true),
    item("Ghost", "LugerPistol_C", 500, "1baa85b4-4c70-1284-64bb-6481dfc3bb4e", SLOT_SECONDARY, true),
    item("Bandit", "CompactPistol_C", 600, "410b2e0b-4ceb-1321-1727-20858f7f3477", SLOT_SECONDARY, true),
    item("Sheriff", "RevolverPistol_C", 800, "e336c6b8-418d-9340-d77f-7a9e4cfe0702", SLOT_SECONDARY, true),
    item("Stinger", "Vector_C", 1100, "f7e1b454-4ad4-1063-ec0a-159e56b58941", SLOT_PRIMARY, true),
    item("Spectre", "SubMachineGun_MP5_C", 1600, "462080d1-4035-2937-7c09-27aa2a5c27a7", SLOT_PRIMARY, true),
    item("Bucky", "PumpShotgun_C", 850, "910be174-449b-c412-ab22-d0873436b21b", SLOT_PRIMARY, true),
    item("Judge", "AutomaticShotgun_C", 1850, "ec845bf4-4f79-ddda-a3da-0db3774b2794", SLOT_PRIMARY, true),
    item("Bulldog", "AssaultRifle_Burst_C", 2050, "ae3de142-4d85-2547-dd26-4e90bed35cf7", SLOT_PRIMARY, true),
    item("Guardian", "DMR_C", 2250, "4ade7faa-4cf1-8376-95ef-39884480959b", SLOT_PRIMARY, true),
    item("Phantom", "AssaultRifle_ACR_C", 2900, "ee8e8d15-496b-07ac-e5f6-8fae5d4c7b1a", SLOT_PRIMARY, true),
    item("Vandal", "AssaultRifle_AK_C", 2900, "9c82e19d-4575-0200-1a81-3eacf00cf872", SLOT_PRIMARY, true),
    item("Warden", "BattleRifle_C", 2900, "8db0a1bf-4a50-832a-4566-faaaa6d250ca", SLOT_PRIMARY, true),
    item("Marshal", "LeverSniperRifle_C", 950, "c4883e50-4494-202c-3ec3-6b8a9284f00b", SLOT_PRIMARY, true),
    item("Outlaw", "DS_Gun_C", 2400, "5f0aaf7a-4289-3998-d5ff-eb9a5cf7ef5c", SLOT_PRIMARY, true),
    item("Operator", "BoltSniper_C", 4700, "a03b24d3-4319-996d-0f8c-94bbfba1dfc7", SLOT_PRIMARY, true),
    item("Ares", "LightMachineGun_C", 1600, "55d8a0f4-4274-ca67-fe2c-06ab45efdf58", SLOT_PRIMARY, true),
    item("Odin", "HeavyMachineGun_C", 3200, "63e6c2b6-4a8e-869c-3d4c-e38355226584", SLOT_PRIMARY, true),
    item("Light Shields", "LightArmorItem_C", 400, "4dec83d5-4902-9ab3-bed6-a7a390761157", SLOT_ARMOR, false),
    item("Heavy Shields", "HeavyArmorItem_C", 1000, "822bcab2-40a2-324e-c137-e09195ad7692", SLOT_ARMOR, false),
    item("Regen Shield", "PlasmaArmorItem_C", 650, "b1b9086d-41bd-a516-5d29-e3b34a6f1644", SLOT_ARMOR, false),
];

/// python store_items[class_name] (range_store.py:50).
pub fn store_item(class_name: &str) -> Option<&'static StoreItem> {
    STORE_ITEMS.iter().find(|item| item.class_name == class_name)
}

/// a split "/Game/<package>.<Class>_C" path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassPath {
    pub package: String,
    pub class_name: String,
    /// "Default__" + class: the archetype in the same package
    pub archetype: String,
}

/// python split_class_path ValueErrors (range_store.py:55-59), same messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClassPathError {
    #[error("expected a /Game/<package>.<Class>_C path")]
    Shape,
    #[error("class does not match its package")]
    Mismatch,
}

/// python split_class_path (range_store.py:53-60).
pub fn split_class_path(class_path: &str) -> Result<ClassPath, ClassPathError> {
    if !class_path.starts_with("/Game/") || class_path.matches('.').count() != 1 {
        return Err(ClassPathError::Shape);
    }
    let (package, class_name) = class_path.split_once('.').expect("one dot checked above");
    let Some(stem) = class_name.strip_suffix("_C") else {
        return Err(ClassPathError::Mismatch);
    };
    if !package.ends_with(&format!("/{stem}")) {
        return Err(ClassPathError::Mismatch);
    }
    Ok(ClassPath { package: package.to_string(), class_name: class_name.to_string(), archetype: format!("Default__{class_name}") })
}

/// python store_item_for_path (range_store.py:88-90).
pub fn store_item_for_path(class_path: &str) -> Result<Option<&'static StoreItem>, ClassPathError> {
    Ok(store_item(&split_class_path(class_path)?.class_name))
}

/// python resolve_class_path (range_store.py:63-85). the first purchase of a
/// class carries its whole path; later ones name the package by the netguid we
/// exported: "<guid:525>.AssaultRifle_AK_C". package_guids maps our exported
/// package guids to paths, seen_paths maps class names to full paths received.
pub fn resolve_class_path(item_path: Option<&str>, package_guids: &HashMap<u32, String>, seen_paths: &HashMap<String, String>) -> Option<String> {
    let item_path = item_path.filter(|path| !path.is_empty())?;
    if !item_path.starts_with("<guid:") {
        return Some(item_path.to_string());
    }
    // python: head, _, class_name = item_path.partition(">."). "<guid:" has no '>',
    // so head always keeps the whole prefix.
    let (head, class_name) = match item_path.find(">.") {
        Some(at) => (&item_path[..at], &item_path[at + 2..]),
        None => (item_path, ""),
    };
    let guid = py_int(head["<guid:".len()..].trim_end_matches('>'))?;
    if !class_name.is_empty()
        && let Some(package) = guid.and_then(|g| u32::try_from(g).ok()).and_then(|g| package_guids.get(&g))
    {
        return Some(format!("{package}.{class_name}"));
    }
    if !class_name.is_empty()
        && let Some(path) = seen_paths.get(class_name)
    {
        return Some(path.clone());
    }
    None
}

/// first code point of every unicode decimal-digit run (general category Nd,
/// ten in a row valued 0..9), unicode 15.0 as in python 3.12's unicodedata.
/// python int() maps each to its ascii digit (_PyUnicode_TransformDecimalAndSpaceToASCII).
const DECIMAL_ZEROS: [u32; 68] = [
    0x30, 0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6, 0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0,
    0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10, 0x104A0, 0x10D30, 0x11066, 0x110F0, 0x11136, 0x111D0,
    0x112F0, 0x11450, 0x114D0, 0x11650, 0x116C0, 0x11730, 0x118E0, 0x11950, 0x11C50, 0x11D50, 0x11DA0, 0x11F50, 0x16A60, 0x16AC0, 0x16B50, 0x1D7CE, 0x1D7D8, 0x1D7E2,
    0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E4F0, 0x1E950, 0x1FBF0,
];

/// python's decimal value of one character (Py_UNICODE_TODECIMAL), None if it has none.
fn py_decimal(c: char) -> Option<u32> {
    let code = c as u32;
    let run = DECIMAL_ZEROS.partition_point(|&zero| zero <= code).checked_sub(1)?;
    let value = code - DECIMAL_ZEROS[run];
    (value < 10).then_some(value)
}

/// cpython int(text) in base 10: None = ValueError, Some(None) = a valid
/// integer too large to be a netguid key. underscores follow PyLong_FromString;
/// the whitespace int() strips is exactly unicode White_Space (char::is_whitespace,
/// checked against python 3.12); any unicode decimal digit counts (the "<guid:"
/// text can be a client FString name, range_client_rpc.py:144-148); the
/// 4300-digit cap is int_max_str_digits.
pub(crate) fn py_int(text: &str) -> Option<Option<i128>> {
    let text = text.trim_matches(char::is_whitespace);
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let chars: Vec<char> = body.chars().collect();
    let digit = |c: &char| py_decimal(*c).is_some();
    if !chars.first().is_some_and(digit) || !chars.last().is_some_and(digit) {
        return None;
    }
    let mut digits = 0usize;
    let mut value: Option<i128> = Some(0);
    for (index, c) in chars.iter().enumerate() {
        if *c == '_' {
            // single underscores between digits only
            if !digit(&chars[index + 1]) {
                return None;
            }
            continue;
        }
        let decimal = py_decimal(*c)?;
        digits += 1;
        value = value.and_then(|v| v.checked_mul(10)).and_then(|v| v.checked_add(decimal as i128));
    }
    if digits > 4300 {
        return None;
    }
    Some(value.map(|v| if negative { -v } else { v }))
}
