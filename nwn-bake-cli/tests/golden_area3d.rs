// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `area-3d` on a real NWN:EE install: `q1rural` of the base game's `XP1-Chapter 1.nwm` (16 by 16 tiles, hills, water, 208
//! placeables, 12 doors), baked with real WebP textures. Its placements must give the scene's triangles bit for bit, every door of
//! the `.git` with a model must be a node carrying its tag and position, and the counts must match `tests/golden/area3d-q1rural.json`.
//!
//! Runs only when `NWN_BAKE_GAME` names the install, as `golden.rs`; otherwise it says it is skipped and passes. Write the counts
//! again after an intended change with `NWN_BAKE_GOLDEN=write`.

use std::path::{Path, PathBuf};

use nwn_bake::area::SceneOptions;
use nwn_bake::area::draft::Draft;
use nwn_bake::area::scene::placements::placements;
use nwn_bake::area3d::gltf::read_glb;
use nwn_bake::area3d::{self, Inputs, Options, TextureFiles};
use nwn_bake::game::{self, GameOptions, Haks, Tlk};
use nwn_bake::module::ModuleSource;
use serde_json::{Value, json};

const MODULE: &str = "nwm/XP1-Chapter 1.nwm";
const AREA: &str = "q1rural";

#[test]
fn golden_area3d_xp1_q1rural() {
    let Some(root) = std::env::var_os("NWN_BAKE_GAME").filter(|v| !v.is_empty()).map(PathBuf::from) else {
        eprintln!("golden_area3d_xp1_q1rural: skipped (set NWN_BAKE_GAME to an NWN:EE install to run the golden tests)");
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("nwn-bake-golden-a3d-{}", std::process::id()));
    let user = std::env::var_os("NWN_BAKE_HOME").filter(|v| !v.is_empty()).map_or_else(|| tmp.join("home"), PathBuf::from);
    std::fs::create_dir_all(&user).unwrap();
    let game = game::open(&GameOptions { root: Some(root.clone()), user: Some(user), haks: Haks::None, tlk: Tlk::None }).unwrap();
    let source = ModuleSource::open(&root.join("data").join(MODULE)).unwrap();
    let area = source.area(AREA).unwrap();

    let o = SceneOptions::default();
    let scene = nwn_bake::area::scene(&game, &area, &o).unwrap();
    placements(&game, &area, &o, &scene).unwrap().check(&scene).unwrap();

    let tex = TextureFiles::new(&tmp.join("tex"), area3d::webp, "webp");
    let inputs = Inputs::of(&area, &[], &[], None).unwrap();
    let baked = area3d::bake(&game, &area, &Options::default(), &tex, inputs.clone(), None).unwrap();
    let again = area3d::bake(&game, &area, &Options::default(), &tex, inputs, None).unwrap();
    assert_eq!(baked.glb, again.glb, "the same bytes twice");

    // Every door drawn is a node with its `.git` tag and position.
    let (j, _) = read_glb(&baked.glb).unwrap();
    let nodes = j["nodes"].as_array().unwrap();
    let git = area.git.as_ref().unwrap();
    let doors = git.root.list("Door List").unwrap();
    assert_eq!(baked.manifest.doors.len(), doors.len());
    for d in &baked.manifest.doors {
        let Some(name) = &d.node else { continue };
        let n = nodes.iter().find(|n| n["name"] == name.as_str()).unwrap_or_else(|| panic!("no node {name}"));
        let s = &doors[d.index as usize];
        assert_eq!(n["extras"]["tag"], String::from_utf8_lossy(s.string("Tag").unwrap()).trim());
        assert_eq!(n["extras"]["x"].as_f64().unwrap() as f32, s.float("X").unwrap());
        assert_eq!(n["extras"]["y"].as_f64().unwrap() as f32, s.float("Y").unwrap());
    }
    for t in &baked.manifest.textures {
        let bytes = std::fs::read(tmp.join("tex").join(&t.file)).unwrap();
        assert_eq!(&bytes[8..12], b"WEBP");
    }

    let m = &baked.manifest;
    let got = json!({
        "module": MODULE, "area": AREA, "counts": m.counts, "doors_drawn": m.doors.iter().filter(|d| d.node.is_some()).count(),
        "door_tags": m.doors.iter().map(|d| d.tag.clone()).collect::<Vec<_>>(), "tileset": m.area.tileset, "tiles": m.area.tiles,
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/area3d-q1rural.json");
    if std::env::var("NWN_BAKE_GOLDEN").is_ok_and(|v| v == "write") {
        std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        eprintln!("wrote {}", path.display());
    } else {
        let want: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(got, want, "counts changed: if intended, NWN_BAKE_GOLDEN=write");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// A draft of `q1rural`'s own layout (its size and every tile, `nwn_bake::area::draft`) is `q1rural`: the same `.are` and `.git`
/// bytes, so the same key, and the same `.glb` and floor file; grown a tile west and south, every door moves 10 m east and north.
#[test]
fn golden_area3d_xp1_q1rural_as_a_draft() {
    let Some(root) = std::env::var_os("NWN_BAKE_GAME").filter(|v| !v.is_empty()).map(PathBuf::from) else {
        eprintln!("golden_area3d_xp1_q1rural_as_a_draft: skipped (set NWN_BAKE_GAME to an NWN:EE install to run the golden tests)");
        return;
    };
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("nwn-bake-golden-a3d-draft-{}", std::process::id()));
    let user = std::env::var_os("NWN_BAKE_HOME").filter(|v| !v.is_empty()).map_or_else(|| tmp.join("home"), PathBuf::from);
    std::fs::create_dir_all(&user).unwrap();
    let game = game::open(&GameOptions { root: Some(root.clone()), user: Some(user), haks: Haks::None, tlk: Tlk::None }).unwrap();
    let area = ModuleSource::open(&root.join("data").join(MODULE)).unwrap().area(AREA).unwrap();
    let r = &area.are.root;
    let (w, h) = (r.integer("Width").unwrap(), r.integer("Height").unwrap());
    let tiles: Vec<Value> = r
        .list("Tile_List")
        .unwrap()
        .iter()
        .map(|t| json!({"tileid": t.integer("Tile_ID"), "orientation": t.integer("Tile_Orientation"), "height": t.integer("Tile_Height")}))
        .collect();
    let draft = Draft::parse(json!({"width": w, "height": h, "tiles": tiles}).to_string().as_bytes()).unwrap();
    let made = draft.area(Some(&area)).unwrap();
    assert_eq!(made.are.to_bytes().unwrap(), area.are.to_bytes().unwrap());
    assert_eq!(made.git.as_ref().unwrap().to_bytes().unwrap(), area.git.as_ref().unwrap().to_bytes().unwrap());

    let tex = TextureFiles::new(&tmp.join("tex"), area3d::webp, "webp");
    let bake = |a: &nwn_bake::module::AreaFiles| {
        area3d::bake(&game, a, &Options::default(), &tex, Inputs::of(a, &[], &[], None).unwrap(), None).unwrap()
    };
    let (normal, drafted) = (bake(&area), bake(&made));
    assert_eq!(drafted.glb, normal.glb);
    assert_eq!(drafted.floor, normal.floor);

    // Grown one tile west and one south (the new row and column the first tile again).
    let first = &tiles[0];
    let mut grown = Vec::new();
    for y in 0..=h {
        for x in 0..=w {
            grown.push(if x == 0 || y == 0 { first.clone() } else { tiles[((y - 1) * w + x - 1) as usize].clone() });
        }
    }
    let draft = json!({"width": w + 1, "height": h + 1, "origin_dx": 1, "origin_dy": 1, "tiles": grown});
    let made = Draft::parse(draft.to_string().as_bytes()).unwrap().area(Some(&area)).unwrap();
    let baked = bake(&made);
    assert_eq!(baked.manifest.area.tiles, [w as u32 + 1, h as u32 + 1]);
    assert_eq!(baked.manifest.doors.len(), normal.manifest.doors.len());
    for (a, b) in baked.manifest.doors.iter().zip(&normal.manifest.doors) {
        assert_eq!((a.tag.as_str(), a.x, a.y), (b.tag.as_str(), b.x + 10.0, b.y + 10.0));
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
