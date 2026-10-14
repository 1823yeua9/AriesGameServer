//! the gameplay-dependent pieces of bring-up: the classic and knife opens with
//! the player's saved skins (A21, B18 i=1) and the live ClientSetHUD index
//! (B13). every failure falls back to what the python sent without them.

use aries_bits::Bits;
use aries_rep::RepResult;
use aries_world::cosmetics::{BASE_PISTOL_WEAPON_ID, BasePistolOpen, base_pistol_open, base_pistol_plain_open};
use aries_world::melee::{MELEE_STARTING_INDEX, MeleeOpen, melee_open};
use aries_world::skins::select_cosmetic_loadout;
use serde_json::Value;

use crate::authority::Env;
use crate::event::Event;

/// A21 (python 2986-3045): the saved classic skin when every part is shipped,
/// else the plain open. a skin load error also opens it plain (python would
/// have raised; the session is kept instead).
pub fn pistol_open_body(env: &mut Env, event: &mut Event) -> RepResult<Bits> {
    if !env.config.features.pistol_initial_cosmetic {
        return base_pistol_plain_open();
    }
    let selection = match env.skins.get().and_then(|inventory| select_cosmetic_loadout(&inventory, BASE_PISTOL_WEAPON_ID)) {
        Ok(selection) => selection,
        Err(error) => {
            event.set("base_pistol_initial_cosmetic_error", error.to_string());
            return base_pistol_plain_open();
        }
    };
    let (body, kind) = base_pistol_open(selection.as_ref())?;
    event.set("base_pistol_initial_cosmetic_bits", body.bit_count);
    match (kind, &selection) {
        (BasePistolOpen::Cosmetic, Some((cosmetic, _))) => {
            event.set("base_pistol_initial_skin_id", cosmetic.skin_id.as_str());
            event.set("base_pistol_initial_level_id", cosmetic.level_id.as_str());
            event.set("base_pistol_initial_chroma_id", cosmetic.chroma_id.as_str());
        }
        (BasePistolOpen::Unshipped, _) => event.set("base_pistol_initial_cosmetic_unshipped", true),
        _ => event.set("base_pistol_initial_cosmetic_unselected", true),
    }
    Ok(body)
}

/// B18 (python 4024-4068): the knife opens with its saved skin; ability
/// tuning opens join here once the tag net indices are read live.
pub fn starting_equippable_open_body(index: usize, env: &mut Env, event: &mut Event) -> RepResult<Bits> {
    if env.config.features.ability_activation && aries_world::abilities::starting_ability_tunings(index).is_some() {
        // the blueprint reads its tuning at BeginPlay, so it rides in the opening bunch
        match ability_open_with_tuning(index, env) {
            Ok((body, tag_bits, constructed)) => {
                event.set("ability_open_tuning", serde_json::json!([index, tag_bits, constructed]));
                return Ok(body);
            }
            Err(error) => event.set("ability_open_tuning_error", error),
        }
    }
    if index != MELEE_STARTING_INDEX {
        return aries_rep::setup::starting_equippable_open_body(index);
    }
    let inventory = env.skins.get();
    let (body, kind) = melee_open(inventory.as_deref())?;
    match kind {
        MeleeOpen::Cosmetic(paths) => {
            event.set("melee_initial_skin_id", paths.skin.asset_id.as_str());
            event.set("melee_initial_level_id", paths.level.asset_id.as_str());
            event.set("melee_initial_chroma_id", paths.chroma.asset_id.as_str());
        }
        MeleeOpen::Unselected => event.set("melee_initial_cosmetic_unselected", true),
        MeleeOpen::Error(error) => event.set("melee_initial_cosmetic_error", Value::from(error)),
    }
    Ok(body)
}

fn ability_open_with_tuning(index: usize, env: &mut Env) -> Result<(Bits, u32, bool), String> {
    let read = env.live.tag_net_indices()?;
    let tags = aries_world::tags::TagNetIndex { indices: read.indices, true_bit_num: read.true_bit_num, first_bit_segment: read.first_bit_segment as i64, constructed: read.constructed };
    // python refuses a segmented index there too (ruh:4061-4062)
    let body = aries_world::abilities::build_ability_open_from_tags(index, &tags).map_err(|e| e.to_string())?;
    Ok((body, tags.true_bit_num, tags.constructed))
}

/// B13 (python 3784-3791): ClientSetHUD's (field, class max) from the live
/// controller class cache; 52/178 in every logged session.
pub fn client_set_hud_index(env: &mut Env, event: &mut Event) -> (u32, u32) {
    match env.live.client_set_hud_index() {
        Ok(Some(index)) => index,
        Ok(None) => (aries_rep::rpc::CLIENT_SET_HUD_FIELD, aries_rep::rpc::CONTROLLER_CLASS_MAX),
        Err(error) => {
            event.set("hud_rpc_error", error);
            (aries_rep::rpc::CLIENT_SET_HUD_FIELD, aries_rep::rpc::CONTROLLER_CLASS_MAX)
        }
    }
}
