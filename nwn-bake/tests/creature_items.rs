// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A creature blueprint's equipped items, on the made-up creature kit (`support/fixture.rs`, `kit_hak`; no game content): found in
//! a folder of item blueprints (binary and nwn_gff's JSON, subfolders too) before the game's, listed with their slot when they are
//! nowhere, and an error when the lookup is strict. Described on the CPU (`nwn_bake::look::describe`): no GPU needed.

#![cfg(feature = "creature")]

#[path = "support/fixture.rs"]
mod fixture;

use std::path::PathBuf;

use nwn_bake::game::{self, Game, GameOptions, Haks, Tlk};
use nwn_bake::items::{Items, MissingItem};

/// `Equip_ItemList` slot ids.
const HEAD: u32 = 0x1;
const RIGHT_HAND: u32 = 0x10;
const LEFT_HAND: u32 = 0x20;

/// The fixture with the creature kit's hak, the game opened with it alone, and a folder of item blueprints: `fx_modrod` as JSON
/// (drawn as `fxrod_001`) and, in a subfolder, `fx_gamerod` as a binary file (drawn as `fxrod_001`, where the game's is `fxrod_002`).
fn kit(tag: &str) -> (fixture::Fixture, Game, PathBuf) {
    let f = fixture::write(&fixture::dir(tag));
    fixture::write_hak(&f.user.join("hak/fx_kit.hak"), &fixture::kit_hak());
    let game = game::open(&GameOptions {
        root: Some(f.root.clone()),
        user: Some(f.user.clone()),
        haks: Haks::List(vec!["fx_kit".into()]),
        tlk: Tlk::None,
    })
    .unwrap();
    let items = f.base.join("uti");
    std::fs::create_dir_all(items.join("more")).unwrap();
    std::fs::write(items.join("fx_modrod.uti.json"), fixture::json(&fixture::rod("fx_modrod", 1))).unwrap();
    std::fs::write(items.join("more/fx_gamerod.uti"), fixture::rod(fixture::GAME_ROD, 1).to_bytes().unwrap()).unwrap();
    (f, game, items)
}

/// The models of a creature's parts, sorted.
fn models(look: &nwn_bake::look::Look) -> Vec<String> {
    let mut m: Vec<String> = look.parts.iter().map(|p| p.spec.model.clone()).collect();
    m.sort();
    m
}

#[test]
fn a_blueprint_wears_items_from_the_folder_and_from_the_game() {
    let (f, game, dir) = kit("items-found");
    let utc = fixture::creature(&[(RIGHT_HAND, "fx_modrod"), (LEFT_HAND, "fx_other")]).to_bytes().unwrap();
    std::fs::write(dir.join("fx_other.uti"), fixture::rod("fx_other", 2).to_bytes().unwrap()).unwrap();
    let look = nwn_bake::look::describe(&game, &utc, &Items::folder(&dir).unwrap(), "pause1", 0.0).unwrap();
    assert_eq!(models(&look), ["fxrod_001", "fxrod_002"], "both hands hold their item");
    assert!(look.missing_items.is_empty(), "{:?}", look.missing_items);
    assert_eq!(look.parts.iter().find(|p| p.spec.model == "fxrod_001").unwrap().spec.attach.as_deref(), Some("rhand"));

    // The game's own item, with no folder.
    let utc = fixture::creature(&[(RIGHT_HAND, fixture::GAME_ROD)]).to_bytes().unwrap();
    let look = nwn_bake::look::describe(&game, &utc, &Items::game(), "pause1", 0.0).unwrap();
    assert_eq!(models(&look), ["fxrod_002"]);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_folder_comes_before_the_game() {
    let (f, game, dir) = kit("items-first");
    let utc = fixture::creature(&[(RIGHT_HAND, fixture::GAME_ROD)]).to_bytes().unwrap();
    let items = Items::folder(&dir).unwrap();
    assert_eq!(items.in_folder(), 2, "the JSON file and the binary file in the subfolder");
    let look = nwn_bake::look::describe(&game, &utc, &items, "pause1", 0.0).unwrap();
    assert_eq!(models(&look), ["fxrod_001"], "the folder's fx_gamerod, not the hak's");
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn an_item_not_found_is_listed_with_its_slot_and_is_an_error_when_strict() {
    let (f, game, dir) = kit("items-missing");
    let utc = fixture::creature(&[(HEAD, "fx_nowhere"), (RIGHT_HAND, "fx_modrod")]).to_bytes().unwrap();

    // Without the folder, the module's item is missing too; the creature is described without both.
    let look = nwn_bake::look::describe(&game, &utc, &Items::game(), "pause1", 0.0).unwrap();
    assert!(look.parts.is_empty(), "{:?}", models(&look));
    let missing = |slot: &str, resref: &str| MissingItem { slot: slot.into(), resref: resref.into(), error: None };
    assert_eq!(look.missing_items, [missing("head", "fx_nowhere"), missing("righthand", "fx_modrod")]);
    let answer = serde_json::to_value(&look).unwrap();
    assert_eq!(answer["missing_items"][0], serde_json::json!({ "slot": "head", "resref": "fx_nowhere" }), "as serve answers");

    // With it, only the item that is nowhere.
    let look = nwn_bake::look::describe(&game, &utc, &Items::folder(&dir).unwrap(), "pause1", 0.0).unwrap();
    assert_eq!(models(&look), ["fxrod_001"]);
    assert_eq!(look.missing_items, [missing("head", "fx_nowhere")]);

    // Strict: an error that names it and its slot.
    let e = nwn_bake::look::describe(&game, &utc, &Items::folder(&dir).unwrap().strict(true), "pause1", 0.0).unwrap_err();
    assert_eq!(format!("{e:#}"), "equipped items not found: fx_nowhere (head)");
    let all_found = fixture::creature(&[(RIGHT_HAND, "fx_modrod")]).to_bytes().unwrap();
    assert!(nwn_bake::look::describe(&game, &all_found, &Items::folder(&dir).unwrap().strict(true), "pause1", 0.0).is_ok());
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn an_item_file_that_does_not_read_is_listed_with_why() {
    let (f, game, dir) = kit("items-broken");
    std::fs::write(dir.join("fx_broken.uti"), b"not a GFF").unwrap();
    let utc = fixture::creature(&[(RIGHT_HAND, "FX_Broken")]).to_bytes().unwrap();
    let look = nwn_bake::look::describe(&game, &utc, &Items::folder(&dir).unwrap(), "pause1", 0.0).unwrap();
    let [m] = look.missing_items.as_slice() else { panic!("{:?}", look.missing_items) };
    assert_eq!((m.slot.as_str(), m.resref.as_str()), ("righthand", "FX_Broken"));
    assert!(m.error.as_deref().is_some_and(|e| e.contains("fx_broken.uti")), "{m}");
    assert!(Items::folder(&dir.join("nope")).is_err(), "a folder that is not there");
    let _ = std::fs::remove_dir_all(&f.base);
}
