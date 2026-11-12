// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A scene's spec, on the fixture (`support/fixture.rs`) with the made-up creature kit: what each object is drawn as
//! (`nwn_bake::scene::parts`, on the CPU: no GPU needed), the items a creature wears found or named, and the spec's schema
//! (`docs/scene.schema.json`) agreeing with the checks the code makes.

#![cfg(feature = "creature")]

#[path = "support/fixture.rs"]
mod fixture;

use std::path::PathBuf;

use nwn_bake::game::{self, Game, GameOptions, Haks, Tlk};
use nwn_bake::items::{Items, MissingItem};
use nwn_bake::scene::{Spec, parts};

/// `Equip_ItemList` slot ids.
const RIGHT_HAND: u32 = 0x10;
const LEFT_HAND: u32 = 0x20;

/// The fixture with its tiles' hak (the placeables) and the creature kit's, and a folder of item blueprints holding `fx_modrod`
/// (drawn as `fxrod_001`), a creature wearing it and `fx_nowhere` as JSON, and the game's own rod as a binary item blueprint.
fn kit(tag: &str) -> (fixture::Fixture, Game, PathBuf) {
    let f = fixture::write(&fixture::dir(tag));
    fixture::write_hak(&f.user.join("hak/fx_kit.hak"), &fixture::kit_hak());
    let game = game::open(&GameOptions {
        root: Some(f.root.clone()),
        user: Some(f.user.clone()),
        haks: Haks::List(vec!["fx_kit".into(), "fx_tiles".into()]),
        tlk: Tlk::None,
    })
    .unwrap();
    let dir = f.base.join("scene");
    std::fs::create_dir_all(dir.join("uti")).unwrap();
    std::fs::write(dir.join("uti/fx_modrod.uti.json"), fixture::json(&fixture::rod("fx_modrod", 1))).unwrap();
    let utc = fixture::creature(&[(RIGHT_HAND, "fx_modrod"), (LEFT_HAND, "fx_nowhere")]);
    std::fs::write(dir.join("fx_guard.utc.json"), fixture::json(&utc)).unwrap();
    std::fs::write(dir.join("fx_rod.uti"), fixture::rod(fixture::GAME_ROD, 2).to_bytes().unwrap()).unwrap();
    (f, game, dir)
}

fn spec(v: serde_json::Value) -> Spec {
    serde_json::from_value(v).unwrap()
}

#[test]
fn each_object_is_drawn_as_the_one_object_pictures_draw_it() {
    let (_f, game, dir) = kit("scene-parts");
    let s = spec(serde_json::json!({
        "objects": [
            {"placeable": 0, "position": [1, 2, 0]},
            {"model": "FX_MARKER", "textures": [["fx_marker", "fx_box"]]},
            {"creature": dir.join("fx_guard.utc.json"), "textures": [["fx_marker", "fx_box"]], "anim": "walk"},
            {"item": dir.join("fx_rod.uti"), "facing": 0},
        ],
    }));
    let found = parts(&game, &s, &Items::folder(&dir.join("uti")).unwrap()).unwrap();
    let kinds: Vec<&str> = found.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, ["placeable", "model", "creature", "item"]);
    assert_eq!(found[0].models[0].model, "fx_box", "placeables.2da row 0");
    assert_eq!(found[0].anim, "default", "a placeable without an idle state");
    assert_eq!(found[1].models[0].textures.get("fx_marker").map(String::as_str), Some("fx_box"), "the texture replaced");

    let guard = &found[2];
    assert_eq!(guard.anim, "walk");
    let models: Vec<&str> = guard.models.iter().map(|m| m.model.as_str()).collect();
    assert_eq!(models, ["fx_body", "fxrod_001"], "the body and the item from the folder");
    assert!(guard.models[0].textures.contains_key("fx_marker"), "the body's texture is replaced");
    assert!(guard.models[1].item && guard.models[1].textures.is_empty(), "what it holds keeps its own");
    assert_eq!(guard.missing_items, [MissingItem { slot: "lefthand".into(), resref: "fx_nowhere".into(), error: None }]);

    assert_eq!(found[3].models[0].model, "fxrod_002", "the item on its own");
    assert_eq!(found[3].anim, "pause1");
}

#[test]
fn a_strict_lookup_names_the_object_and_the_item() {
    let (_f, game, dir) = kit("scene-strict");
    let s = spec(serde_json::json!({"objects": [{"placeable": 1}, {"creature": dir.join("fx_guard.utc.json")}]}));
    let items = Items::folder(&dir.join("uti")).unwrap().strict(true);
    let e = format!("{:#}", parts(&game, &s, &items).unwrap_err());
    assert!(e.contains("object 1 (creature") && e.contains("fx_nowhere (lefthand)"), "{e}");
}

#[test]
fn what_is_not_there_is_an_error_that_names_it() {
    let (_f, game, dir) = kit("scene-absent");
    for (object, says) in [
        (serde_json::json!({"model": "fx_nothing"}), "model fx_nothing: not found"),
        (serde_json::json!({"creature": dir.join("nobody.utc")}), "nobody.utc"),
        (serde_json::json!({"placeable": 99}), "placeables.2da row 99"),
    ] {
        let e = format!("{:#}", parts(&game, &spec(serde_json::json!({"objects": [object]})), &Items::game()).unwrap_err());
        assert!(e.contains("object 0") && e.contains(says), "{e}");
    }
}

/// The schema, compiled.
fn schema() -> jsonschema::Validator {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/scene.schema.json")).unwrap();
    jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap()
}

#[test]
fn the_schema_and_the_checks_agree() {
    let v = schema();
    let good = [
        serde_json::json!({"objects": [{"placeable": 3}]}),
        serde_json::json!({"lines": [{"from": [0, 0, 0], "to": [1, 1, 0], "colour": "#ff000080"}]}),
        serde_json::json!({
            "width": 640, "height": 480,
            "objects": [{"creature": "a.utc", "position": [1, 2, 3], "facing": 90, "scale": 1.5, "textures": [["a", "b"]], "anim": "walk", "t": 0.5},
                        {"item": "b.uti"}, {"model": "plc_a01"}],
            "camera": {"target": [0, 0, 1], "yaw": 45, "pitch": 89.9, "orthographic": true, "scale": 40},
            "axes": [{"at": [0, 0, 0]}], "points": [[1, 2, 3]],
            "light": {"ambient": "#202020", "diffuse": "#ffffff", "toward": [1, 1, 1]}, "shadows": true, "background": "transparent",
        }),
        serde_json::json!({"objects": [{"placeable": 0}], "camera": {"position": [0, -10, 5], "target": [0, 0, 0], "fov": 60}, "light": "day"}),
    ];
    for g in &good {
        let errors: Vec<String> = v.iter_errors(g).map(|e| format!("{e} at {}", e.instance_path())).collect();
        assert!(errors.is_empty(), "{g}: {errors:?}");
        spec(g.clone()).check().unwrap_or_else(|e| panic!("{g}: {e:#}"));
    }
    let bad = [
        serde_json::json!({}),
        serde_json::json!({"objects": [{"placeable": 3, "model": "x"}]}),
        serde_json::json!({"objects": [{"position": [0, 0, 0]}]}),
        serde_json::json!({"objects": [{"placeable": 3}], "width": 0}),
        serde_json::json!({"objects": [{"placeable": 3}], "camera": {"pitch": 90}}),
        serde_json::json!({"objects": [{"placeable": 3}], "camera": {"scale": 10}}),
        serde_json::json!({"objects": [{"placeable": 3}], "light": "night"}),
        serde_json::json!({"objects": [{"placeable": 3}], "background": "#abc"}),
        serde_json::json!({"objects": [{"placeable": 3, "scale": 0}]}),
        serde_json::json!({"lines": [{"from": [0, 0, 0], "to": [1, 0, 0], "colour": "red"}]}),
    ];
    for b in &bad {
        assert!(!v.is_valid(b), "the schema takes {b}");
        assert!(serde_json::from_value::<Spec>(b.clone()).map_or(true, |s| s.check().is_err()), "the code takes {b}");
    }
    assert!(!v.is_valid(&serde_json::json!({"objects": [{"placeable": 3}], "zoom": 2})), "an unknown field");
}
