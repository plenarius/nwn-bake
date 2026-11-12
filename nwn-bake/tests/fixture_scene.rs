// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The game, the module sources, the tileset and the scene on the generated fixture (`support/fixture.rs`; no game content): every
//! module form gives the same scene, the tileset's absent terrain, tiles turned and lifted, water apart, the material, hak order,
//! the floor probe and the roof cut on each layout, black void, the chasm rule, and the scene the same bit for bit.

#[path = "support/fixture.rs"]
mod fixture;

use std::path::Path;

use mg_core::{ResRef, ResType};
use mg_gff::Value;
use nwn_bake::area::{Kind, Part, RoofRule, Scene, SceneOptions};
use nwn_bake::game::{self, Game, GameOptions, Haks, Tlk};
use nwn_bake::iso::Inputs;
use nwn_bake::module::ModuleSource;

fn open(f: &fixture::Fixture, haks: &[&str]) -> Game {
    open_in(&f.root, &f.user, haks)
}

fn open_in(root: &Path, user: &Path, haks: &[&str]) -> Game {
    let g = game::open(&GameOptions {
        root: Some(root.to_path_buf()),
        user: Some(user.to_path_buf()),
        haks: Haks::List(haks.iter().map(ToString::to_string).collect()),
        tlk: Tlk::None,
    })
    .unwrap();
    assert!(g.missing_haks().is_empty(), "{:?}", g.missing_haks());
    g
}

fn scene_of(g: &Game, source: &ModuleSource, area: &str) -> Scene {
    nwn_bake::area::scene(g, &source.area(area).unwrap(), &SceneOptions::default()).unwrap()
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5)
}

const ABSENT: &str = "tileset fx01: [TERRAIN2] is counted but absent (read as empty, as the game does)";

#[test]
fn the_tileset_is_read_with_its_counted_but_absent_terrain() {
    let f = fixture::write(&fixture::dir("tileset"));
    let g = open(&f, &fixture::HAKS);
    let read = nwn_bake::area::tileset(&g, ResRef::from_str("fx01").unwrap()).unwrap();
    assert_eq!(read.absent, ["TERRAIN2"]);
    assert_eq!(read.tileset.terrains.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Grass", "Rock", "Chasm"]);
    assert_eq!(read.tileset.tiles.iter().map(|t| t.model.to_ascii_lowercase()).collect::<Vec<_>>(), fixture::TILE_MODELS);
    assert_eq!(read.tileset.general.transition, 2.0);
    // Every scene on it says so.
    let s = scene_of(&g, &ModuleSource::open(&f.module_mod()).unwrap(), "fx_main");
    assert_eq!(s.notes, [ABSENT]);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn every_module_form_gives_the_same_areas_scenes_and_inputs() {
    let f = fixture::write(&fixture::dir("forms"));
    let g = open(&f, &fixture::HAKS);
    let reference = ModuleSource::open(&f.module_mod()).unwrap();
    let want: Vec<(String, Inputs)> = fixture::AREAS
        .iter()
        .map(|a| {
            let files = reference.area(a).unwrap();
            (scene_of(&g, &reference, a).digest(), Inputs::of(&files, reference.haks(), &[], None).unwrap())
        })
        .collect();
    for (form, path) in f.forms() {
        let m = ModuleSource::open(&path).unwrap_or_else(|e| panic!("{form}: {e:#}"));
        assert_eq!(m.haks(), fixture::HAKS, "{form}");
        assert_eq!(m.areas(), fixture::AREAS, "{form}");
        assert!(m.warnings().is_empty(), "{form}: {:?}", m.warnings());
        for (a, (digest, inputs)) in fixture::AREAS.iter().zip(&want) {
            let files = m.area(a).unwrap();
            assert_eq!(&scene_of(&g, &m, a).digest(), digest, "{form}: {a}'s scene");
            let got = Inputs::of(&files, m.haks(), &[], None).unwrap();
            assert_eq!((&got.are, &got.git), (&inputs.are, &inputs.git), "{form}: {a}'s .are and .git digests");
        }
    }
    // One area file, binary or JSON: that area, and the module's haks from the module.ifo above it.
    for (form, path) in f.area_files("fx_main") {
        let m = ModuleSource::open(&path).unwrap_or_else(|e| panic!("{form}: {e:#}"));
        assert_eq!((m.haks(), m.areas()), (&fixture::HAKS.map(String::from)[..], vec!["fx_main".to_string()]), "{form}");
        assert_eq!(scene_of(&g, &m, "fx_main").digest(), want[0].0, "{form}");
    }
    // An area without a .git: no objects, and no .git digest.
    assert!(want[2].1.git.is_none());
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_main_area_has_its_tiles_turned_and_lifted_water_apart_and_its_objects() {
    let f = fixture::write(&fixture::dir("main"));
    let g = open(&f, &fixture::HAKS);
    let s = scene_of(&g, &ModuleSource::open(&f.project()).unwrap(), "fx_main");
    assert_eq!(
        (s.resref.as_str(), s.tileset.as_str(), s.tiles, s.size(), s.kind()),
        ("fx_main", "fx01", [3, 2], [30.0, 20.0], Kind::Surface)
    );
    assert_eq!((s.light.ambient, s.light.diffuse), ([0x11, 0x22, 0x33], [0xaa, 0xbb, 0xcc]));
    let c = &s.counts;
    // Four grass tiles of a ground quad and a rock, a void tile and a plain one of a quad; three models; each grass tile's pond is
    // water. The box is a cube without its bottom (10 triangles), the door a quad.
    assert_eq!((c.tiles, c.tile_models, c.tile_triangles, c.water_triangles, c.stray), (6, 3, 16, 4, 0));
    assert_eq!((c.placeables, c.doors, c.object_triangles), (1, 1, 12));
    // Textures numbered by first use; the rock's material means the stone image; all found.
    let names: Vec<&str> = s.textures.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["fx_grass", "fx_rock", "fx_black", "fx_crate", "fx_wood"]);
    assert_eq!(s.textures[1].image.as_ref().unwrap().image, "fx_stone");
    assert_eq!(c.textures_missing, 0);
    // The rock's top, (3.75, 4.5) about the tile's middle: tile 0 as it is, tile 1 a quarter turn counter-clockwise, tile 2 a half
    // turn; tile 4 three quarter turns and a step (2 m) up.
    let top = |tile: u32| s.triangles.iter().find(|t| t.part == Part::Tile(tile) && t.texture == 1).unwrap().p[2];
    assert!(close(top(0), [5.0 + 3.75, 5.0 + 4.5, 2.5]), "{:?}", top(0));
    assert!(close(top(1), [15.0 - 4.5, 5.0 + 3.75, 2.5]), "{:?}", top(1));
    assert!(close(top(2), [25.0 - 3.75, 5.0 - 4.5, 2.5]), "{:?}", top(2));
    assert!(close(top(4), [15.0 + 4.5, 15.0 - 3.75, 4.5]), "{:?}", top(4));
    // Each pond in its tile, 0.1 m over the tile's ground.
    for (w, (x, y, z)) in s.water.iter().zip([(5.0, 5.0, 0.0), (15.0, 5.0, 0.0), (25.0, 5.0, 0.0), (15.0, 15.0, 2.0)]) {
        assert!(w.iter().all(|p| (p[0] - x).abs() <= 4.0 && (p[1] - y).abs() <= 4.0 && (p[2] - (z + 0.1)).abs() < 1e-5), "{w:?}");
    }
    // The box, turned a quarter about its origin: its (1, 0, 0) corner goes to (0, 1).
    let corners: Vec<[f32; 3]> = s.triangles.iter().filter(|t| t.part == Part::Placeable(0)).flat_map(|t| t.p).collect();
    assert!(corners.iter().any(|p| close(*p, [12.0, 4.0, 0.0])) && !corners.iter().any(|p| close(*p, [13.0, 3.0, 0.0])), "{corners:?}");
    // The door, from genericdoors.2da, 3 m high where the .git puts it.
    let door: Vec<[f32; 3]> = s.triangles.iter().filter(|t| t.part == Part::Door(0)).flat_map(|t| t.p).collect();
    assert!(door.iter().any(|p| close(*p, [24.0, 15.0, 3.0])), "{door:?}");
    // Without objects: tiles only.
    let bare = nwn_bake::area::scene(
        &g,
        &ModuleSource::open(&f.project()).unwrap().area("fx_main").unwrap(),
        &SceneOptions { objects: false, ..Default::default() },
    )
    .unwrap();
    assert_eq!((bare.counts.object_triangles, bare.triangles.len(), bare.objects), (0, 16, false));
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_first_hak_listed_wins() {
    let f = fixture::write(&fixture::dir("haks"));
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let marker = |haks: &[&str]| {
        let s = scene_of(&open(&f, haks), &source, "fx_mark");
        let t = s.textures.iter().find(|t| t.name == "fx_marker").unwrap();
        t.image.as_ref().unwrap().levels[0].rgba[..4].to_vec()
    };
    assert_eq!(marker(&["fx_over", "fx_tiles"]), [255, 0, 0, 255], "fx_over is listed first: its red marker");
    assert_eq!(marker(&["fx_tiles", "fx_over"]), [0, 0, 255, 255], "fx_tiles is listed first: its blue marker");
    // A hak found nowhere is reported, and the rest are read.
    let g = game::open(&GameOptions {
        root: Some(f.root.clone()),
        user: Some(f.user.clone()),
        haks: Haks::List(vec!["nope".into(), "fx_tiles".into()]),
        tlk: Tlk::None,
    })
    .unwrap();
    assert_eq!(g.missing_haks(), ["nope".to_string()]);
    assert_eq!(scene_of(&g, &source, "fx_main").counts.tile_triangles, 16);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_floor_probe_and_the_roof_cut_on_each_layout() {
    let f = fixture::write(&fixture::dir("probe"));
    let g = open(&f, &fixture::HAKS);
    let source = ModuleSource::open(&f.folder()).unwrap();
    let rule = RoofRule::default();
    let at = |s: &Scene, x: f32, y: f32| {
        let p = &s.probe;
        let k = (y / 0.625).round() as usize * p.nx + (x / 0.625).round() as usize;
        (p.heights[k], p.is_void(k))
    };

    // The main area: grass at 0 m, the raised grass at 2 m, the void tile seen as void; over the pond's bed and under the rock's
    // foot (which does not walk) the grass is the floor. A surface area keeps its roof.
    let main = scene_of(&g, &source, "fx_main");
    assert_eq!((main.probe.nx, main.probe.ny), (49, 33));
    assert_eq!(at(&main, 1.25, 7.5), (0.0, false));
    assert_eq!(at(&main, 16.25, 17.5), (2.0, false));
    assert_eq!(at(&main, 3.75, 15.0), (0.0, true));
    assert_eq!(at(&main, 2.5, 2.5), (0.0, false));
    assert_eq!(at(&main, 8.75, 8.75), (0.0, false));
    assert!(main.roof_cut(&rule).is_none());

    // A flat floor of rooms: cut 3.5 m over it, which takes the 5 m ceiling.
    let flat = scene_of(&g, &source, "fx_flat");
    assert_eq!(flat.kind(), Kind::Interior);
    let cut = flat.roof_cut(&rule).unwrap();
    assert_eq!((cut.at, cut.slab), (3.5, false));
    let mut cut_flat = flat.clone();
    cut_flat.cut_roof(cut.at);
    assert_eq!((flat.triangles.len(), cut_flat.triangles.len()), (16, 8), "the ceilings are cut away");

    // A cave dug out of rock: two thirds of it the rock's top at 6 m, a third floor at 0 m: cut 2 m over the floor.
    let cave = scene_of(&g, &source, "fx_cave");
    assert_eq!(cave.kind(), Kind::Underground);
    let stats = cave.probe.stats();
    assert!(stats.top_share > 0.25 && stats.floor_share >= rule.slab_floors, "{stats:?}");
    assert_eq!((stats.roof, stats.floor_p95), (6.0, Some(0.0)));
    let cut = cave.roof_cut(&rule).unwrap();
    assert_eq!((cut.at, cut.slab), (2.0, true));

    // Walkable ground at the top (4 m) over black void is no cave: the void is no floor, so it is cut 3.5 m over the ground.
    let step = scene_of(&g, &source, "fx_step");
    let stats = step.probe.stats();
    assert_eq!((stats.roof, stats.floors), (4.0, 0), "{stats:?}");
    assert!(stats.top_share > 0.25 && stats.drawn_share < 0.6, "{stats:?}");
    let cut = step.roof_cut(&rule).unwrap();
    assert_eq!((cut.at, cut.slab), (7.5, false));
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn a_scene_is_the_same_bit_for_bit_and_changes_with_any_input() {
    let f = fixture::write(&fixture::dir("digest"));
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    // Two games opened apart (nothing cached shared), and one asked twice (its caches warm).
    let (g1, g2) = (open(&f, &fixture::HAKS), open(&f, &fixture::HAKS));
    for area in fixture::AREAS {
        let (a, b, c) = (scene_of(&g1, &source, area), scene_of(&g2, &source, area), scene_of(&g1, &source, area));
        assert_eq!(a.digest(), b.digest(), "{area}");
        assert_eq!(a.digest(), c.digest(), "{area}");
        assert_eq!((&a.triangles, &a.water, &a.probe, &a.notes), (&b.triangles, &b.water, &b.probe, &b.notes), "{area}");
    }
    let main = scene_of(&g1, &source, "fx_main");
    let digest = main.digest();
    let inputs = Inputs::of(&source.area("fx_main").unwrap(), source.haks(), &[], None).unwrap();

    // The box moved a centimetre: another scene and another .git digest; the .are's is the same.
    let mut moved = source.area("fx_main").unwrap();
    match moved.git.as_mut().unwrap().root.get_mut("Placeable List") {
        Some(Value::List(l)) => l[0].set("X", Value::Float(12.01)),
        _ => unreachable!(),
    }
    assert_ne!(nwn_bake::area::scene(&g1, &moved, &SceneOptions::default()).unwrap().digest(), digest);
    let moved_inputs = Inputs::of(&moved, source.haks(), &[], None).unwrap();
    assert_eq!(moved_inputs.are, inputs.are);
    assert_ne!(moved_inputs.git, inputs.git);

    // A tile a step higher: another scene and another .are digest.
    let mut raised = source.area("fx_main").unwrap();
    match raised.are.root.get_mut("Tile_List") {
        Some(Value::List(l)) => l[5].set("Tile_Height", Value::Int(1)),
        _ => unreachable!(),
    }
    assert_ne!(nwn_bake::area::scene(&g1, &raised, &SceneOptions::default()).unwrap().digest(), digest);
    assert_ne!(Inputs::of(&raised, source.haks(), &[], None).unwrap().are, inputs.are);

    // One texel of the grass changed, and one vertex of a tile model: other scenes.
    for (what, change) in [
        (
            "a texel of fx_grass",
            Box::new(|files: &mut Vec<(String, ResType, Vec<u8>)>| {
                let grass = files.iter_mut().find(|(n, t, _)| n == "fx_grass" && *t == ResType::TGA).unwrap();
                let last = grass.2.len() - 2;
                grass.2[last] ^= 1;
            }) as Box<dyn Fn(&mut Vec<(String, ResType, Vec<u8>)>)>,
        ),
        (
            "a vertex of fx01_c01",
            Box::new(|files: &mut Vec<(String, ResType, Vec<u8>)>| {
                let m = files.iter_mut().find(|(n, t, _)| n == "fx01_c01" && *t == ResType::MDL).unwrap();
                m.2 = String::from_utf8(m.2.clone()).unwrap().replacen("    5 5 0\n", "    5 5 0.01\n", 1).into_bytes();
            }),
        ),
    ] {
        let user = f.base.join(format!("user-{}", what.replace(' ', "-")));
        std::fs::create_dir_all(user.join("hak")).unwrap();
        let mut files = fixture::tiles_hak();
        change(&mut files);
        fixture::write_hak(&user.join("hak/fx_tiles.hak"), &files);
        fixture::write_hak(&user.join("hak/fx_over.hak"), &fixture::over_hak());
        let changed = scene_of(&open_in(&f.root, &user, &fixture::HAKS), &source, "fx_main");
        assert_ne!(changed.digest(), digest, "{what}");
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn a_black_floor_deep_in_a_tile_with_chasm_terrain_is_a_chasm_and_elsewhere_is_not() {
    let f = fixture::write(&fixture::dir("chasm"));
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let g = open(&f, &fixture::HAKS);
    let s = scene_of(&g, &source, "fx_chasm");
    let floor = |s: &Scene, tile: u32| -> Vec<bool> {
        s.triangles
            .iter()
            .filter(|t| t.part == Part::Tile(tile) && t.p.iter().all(|v| (v[2] - fixture::PIT_FLOOR).abs() < 1e-5))
            .map(|t| t.chasm)
            .collect()
    };
    // The pit's floor on the tile with chasm corners (the area's second tile) is a chasm's floor, and nothing else is.
    assert_eq!(floor(&s, 1), [true, true]);
    assert_eq!((s.counts.chasm_triangles, s.triangles.iter().filter(|t| t.chasm).count()), (2, 2));
    // The same model on the tile with grass corners (the third): a black floor as deep, but no chasm.
    assert_eq!(floor(&s, 2), [false, false]);
    // The same scene with the floor's walkmesh walkable: walked on, so no chasm either.
    let user = f.base.join("user-walked");
    std::fs::create_dir_all(user.join("hak")).unwrap();
    let mut files = fixture::tiles_hak();
    files.iter_mut().find(|(n, t, _)| n == "fx01_f01" && *t == ResType::WOK).unwrap().2 =
        fixture::walkmesh("fx01_f01", &fixture::pit_walk(fixture::WALK));
    fixture::write_hak(&user.join("hak/fx_tiles.hak"), &files);
    fixture::write_hak(&user.join("hak/fx_over.hak"), &fixture::over_hak());
    let walked = scene_of(&open_in(&f.root, &user, &fixture::HAKS), &source, "fx_chasm");
    assert_eq!((floor(&walked, 1), walked.counts.chasm_triangles), (vec![false, false], 0));
    // A scene with no chasm keeps the digest it had: the chasm is in the digest only where there is one.
    assert_ne!(walked.digest(), s.digest());
    let _ = std::fs::remove_dir_all(&f.base);
}
