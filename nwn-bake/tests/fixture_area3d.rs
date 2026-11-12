// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `area-3d` on the generated fixture (`support/fixture.rs`; no game content): the placements give the scene's triangles, the `.glb`
//! drawn from its instances and nodes is the scene, the doors and placeables are named nodes with their `.git` values, textures are
//! files named by their content and shared between areas, the same inputs give the same bytes, and the batch reuses the output folder
//! and the cache by key.
//!
//! The textures are "encoded" by a stand-in (the texels as they are, behind a small header), so these tests need no WebP; the
//! command-line tests (`nwn-bake-cli/tests/area_3d.rs`) write real WebP.

#[path = "support/fixture.rs"]
mod fixture;

use std::path::Path;

use nwn_bake::area::scene::placements::{Turn, placements};
use nwn_bake::area::{Part, SceneOptions};
use nwn_bake::area3d::batch::{self, Action, Job};
use nwn_bake::area3d::gltf::{INSTANCING, read_floats, read_glb, read_indices};
use nwn_bake::area3d::{self, Inputs, Options, TextureFiles, content_name, y_up};
use nwn_bake::game::{self, Game, GameOptions, Haks, Tlk};
use nwn_bake::module::ModuleSource;
use serde_json::Value;

fn open(f: &fixture::Fixture) -> Game {
    let g = game::open(&GameOptions {
        root: Some(f.root.clone()),
        user: Some(f.user.clone()),
        haks: Haks::List(fixture::HAKS.iter().map(ToString::to_string).collect()),
        tlk: Tlk::None,
    })
    .unwrap();
    assert!(g.missing_haks().is_empty(), "{:?}", g.missing_haks());
    g
}

/// A stand-in encoder: `RAW`, the width and height, the texels.
fn raw(img: &mg_image::Rgba, quality: f32) -> Vec<u8> {
    let mut out = b"RAW".to_vec();
    out.extend(img.width.to_le_bytes());
    out.extend(img.height.to_le_bytes());
    out.push(quality as u8);
    out.extend(&img.data);
    out
}

fn inputs(source: &ModuleSource, area: &nwn_bake::module::AreaFiles) -> Inputs {
    Inputs::of(area, source.haks(), &[], None).unwrap()
}

/// Turns `v` by the unit quaternion `q` (x, y, z, w).
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let (u, w) = ([q[0], q[1], q[2]], q[3]);
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let ut = cross(u, t);
    [v[0] + w * t[0] + ut[0], v[1] + w * t[1] + ut[1], v[2] + w * t[2] + ut[2]]
}

/// A mesh's triangles (positions) from the file.
fn mesh_triangles(j: &Value, bin: &[u8], mesh: usize) -> Vec<[[f32; 3]; 3]> {
    let mut out = Vec::new();
    for prim in j["meshes"][mesh]["primitives"].as_array().unwrap() {
        let pos: Vec<[f32; 3]> = read_floats(j, bin, prim["attributes"]["POSITION"].as_u64().unwrap() as usize).unwrap();
        let idx = read_indices(j, bin, prim["indices"].as_u64().unwrap() as usize).unwrap();
        out.extend(idx.as_chunks::<3>().0.iter().map(|t| [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]]));
    }
    out
}

type Faces = Vec<[[f32; 3]; 3]>;

/// Every triangle the file draws but the water, in its space (each instance and node placed), and the water's.
fn drawn(glb: &[u8]) -> (Faces, Faces) {
    let (j, bin) = read_glb(glb).unwrap();
    let root = j["scenes"][0]["nodes"][0].as_u64().unwrap() as usize;
    let (mut solid, mut water) = (Vec::new(), Vec::new());
    for c in j["nodes"][root]["children"].as_array().unwrap() {
        let node = &j["nodes"][c.as_u64().unwrap() as usize];
        let tris = mesh_triangles(&j, &bin, node["mesh"].as_u64().unwrap() as usize);
        if node["extras"]["part"] == "water" {
            water.extend(tris);
            continue;
        }
        let places: Vec<([f32; 3], [f32; 4])> = match node.get("extensions").and_then(|e| e.get(INSTANCING)) {
            Some(inst) => {
                let t: Vec<[f32; 3]> = read_floats(&j, &bin, inst["attributes"]["TRANSLATION"].as_u64().unwrap() as usize).unwrap();
                let r: Vec<[f32; 4]> = read_floats(&j, &bin, inst["attributes"]["ROTATION"].as_u64().unwrap() as usize).unwrap();
                t.into_iter().zip(r).collect()
            }
            None => {
                let v = |k: &str, n: usize| -> Vec<f32> {
                    node[k].as_array().map_or(vec![0.0; n], |a| a.iter().map(|x| x.as_f64().unwrap() as f32).collect())
                };
                let (t, r) = (v("translation", 3), node["rotation"].as_array().map_or(vec![0.0, 0.0, 0.0, 1.0], |_| v("rotation", 4)));
                vec![([t[0], t[1], t[2]], [r[0], r[1], r[2], r[3]])]
            }
        };
        for (t, r) in places {
            solid.extend(tris.iter().map(|tri| {
                tri.map(|p| {
                    let q = rotate(r, p);
                    [q[0] + t[0], q[1] + t[1], q[2] + t[2]]
                })
            }));
        }
    }
    (solid, water)
}

/// Triangles as sorted lists of rounded corners (millimetres), for comparing as sets.
fn rounded(tris: &[[[f32; 3]; 3]]) -> Vec<[[i64; 3]; 3]> {
    let mut out: Vec<[[i64; 3]; 3]> = tris
        .iter()
        .map(|t| {
            let mut c = t.map(|p| p.map(|x| (x * 1000.0).round() as i64));
            c.sort_unstable();
            c
        })
        .collect();
    out.sort_unstable();
    out
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).map_or_default(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect());
    v.sort();
    v
}

#[test]
fn the_placements_are_the_scene_and_the_glb_draws_it() {
    let f = fixture::write(&fixture::dir("a3d-scene"));
    let g = open(&f);
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    for name in fixture::AREAS {
        let area = source.area(name).unwrap();
        let o = SceneOptions::default();
        let scene = nwn_bake::area::scene(&g, &area, &o).unwrap();
        let placed = placements(&g, &area, &o, &scene).unwrap();
        placed.check(&scene).unwrap_or_else(|e| panic!("{name}: {e:#}"));

        let tex = TextureFiles::new(&f.base.join("tex"), raw, "raw");
        let baked = area3d::bake(&g, &area, &Options::default(), &tex, inputs(&source, &area), None).unwrap();
        let (solid, water) = drawn(&baked.glb);
        let want: Vec<[[f32; 3]; 3]> = scene.triangles.iter().map(|t| t.p.map(y_up)).collect();
        assert_eq!(solid.len(), want.len(), "{name}");
        assert_eq!(rounded(&solid), rounded(&want), "{name}: the glb draws the scene's triangles");
        let want_water: Vec<[[f32; 3]; 3]> = scene.water.iter().map(|t| t.map(y_up)).collect();
        assert_eq!(rounded(&water), rounded(&want_water), "{name}: the water");
        let c = &baked.manifest.counts;
        assert_eq!(c.triangles_drawn, scene.triangles.len() + scene.water.len(), "{name}");
        assert_eq!(c.tiles, scene.counts.tiles);
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn tiles_are_instances_and_objects_are_named_nodes_with_their_git_values() {
    let f = fixture::write(&fixture::dir("a3d-nodes"));
    let g = open(&f);
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let area = source.area("fx_main").unwrap();
    let tex = TextureFiles::new(&f.base.join("tex"), raw, "raw");
    let baked = area3d::bake(&g, &area, &Options::default(), &tex, inputs(&source, &area), Some("k".into())).unwrap();
    let (j, bin) = read_glb(&baked.glb).unwrap();
    assert!(j["extensionsRequired"].as_array().unwrap().iter().any(|e| e == INSTANCING));
    let root = &j["nodes"][j["scenes"][0]["nodes"][0].as_u64().unwrap() as usize];
    assert_eq!(root["name"], "fx_main");
    assert_eq!(root["extras"]["tileset"], "fx01");
    let nodes: Vec<&Value> = root["children"].as_array().unwrap().iter().map(|c| &j["nodes"][c.as_u64().unwrap() as usize]).collect();

    // Six tiles from three models (fx01_a01 four times, turned 0..3 quarters; the void; plain grass): three instanced nodes.
    let tiles: Vec<&&Value> = nodes.iter().filter(|n| n["extras"]["part"] == "tiles").collect();
    assert_eq!(tiles.len(), 3);
    assert_eq!(baked.manifest.counts.tile_nodes, 3);
    let a01 = tiles.iter().find(|n| n["extras"]["model"] == "fx01_a01").unwrap();
    assert_eq!(a01["name"], "tiles_fx01_a01");
    assert_eq!(a01["extras"]["tiles"], serde_json::json!([0, 1, 2, 4]));
    let inst = &a01["extensions"][INSTANCING]["attributes"];
    let t: Vec<[f32; 3]> = read_floats(&j, &bin, inst["TRANSLATION"].as_u64().unwrap() as usize).unwrap();
    let r: Vec<[f32; 4]> = read_floats(&j, &bin, inst["ROTATION"].as_u64().unwrap() as usize).unwrap();
    // Tile 4 is (1, 1), a step (2 m) up, turned three quarters: (15, 15, 2) in the game, (15, 2, -15) in glTF.
    assert_eq!(t, [[5.0, 0.0, -5.0], [15.0, 0.0, -5.0], [25.0, 0.0, -5.0], [15.0, 2.0, -15.0]]);
    let h = std::f32::consts::FRAC_1_SQRT_2;
    assert_eq!(r, [[0.0, 0.0, 0.0, 1.0], [0.0, h, 0.0, h], [0.0, 1.0, 0.0, 0.0], [0.0, h, 0.0, -h]]);
    // The mesh is stored once for the four tiles.
    let meshes: Vec<&str> = j["meshes"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(meshes.iter().filter(|m| **m == "fx01_a01").count(), 1);

    // The door and the box: named nodes, sharing nothing but carrying the `.git`'s values.
    let door = nodes.iter().find(|n| n["extras"]["part"] == "door").unwrap();
    assert_eq!(door["name"], "door_0_fx_door");
    assert_eq!(door["extras"]["index"], 0);
    assert_eq!(door["extras"]["tag"], "fx_door");
    assert_eq!(
        (door["extras"]["x"].as_f64(), door["extras"]["y"].as_f64(), door["extras"]["z"].as_f64()),
        (Some(25.0), Some(15.0), Some(0.0))
    );
    assert_eq!(door["translation"], serde_json::json!([25.0, 0.0, -15.0]));
    assert_eq!(door["extras"]["front_deg"], 270.0);
    let boxed = nodes.iter().find(|n| n["extras"]["part"] == "placeable").unwrap();
    assert_eq!(boxed["name"], "placeable_0_fx_plc");
    // (The fixture's quarter turn, as nasher keeps it: four decimals.)
    #[expect(clippy::approx_constant, reason = "the fixture's quarter turn to four decimals, not the exact constant")]
    let quarter = 1.5708f32;
    assert_eq!(boxed["extras"]["bearing"].as_f64().unwrap() as f32, quarter);
    let q = boxed["rotation"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect::<Vec<_>>();
    assert!((q[1] - (quarter / 2.0).sin()).abs() < 1e-6 && (q[3] - (quarter / 2.0).cos()).abs() < 1e-6, "{q:?}");

    // The manifest's door list.
    let d = &baked.manifest.doors;
    assert_eq!(d.len(), 1);
    assert_eq!((d[0].index, d[0].tag.as_str(), d[0].node.as_deref()), (0, "fx_door", Some("door_0_fx_door")));
    assert_eq!((d[0].x, d[0].y, d[0].bearing, d[0].front_deg), (25.0, 15.0, 0.0, 270.0));
    // Street side south: -y in the game is +z in glTF.
    assert!((d[0].front[2] - 1.0).abs() < 1e-6, "{:?}", d[0].front);
    assert_eq!(baked.manifest.key.as_deref(), Some("k"));
    assert_eq!(baked.manifest.area.name.as_deref(), Some("Fixture fx_main"));
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn textures_are_files_named_by_their_content_shared_by_areas_and_the_bytes_repeat() {
    let f = fixture::write(&fixture::dir("a3d-tex"));
    let g = open(&f);
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let dir = f.base.join("tex");
    let tex = TextureFiles::new(&dir, raw, "raw");
    let bake = |name: &str| {
        let area = source.area(name).unwrap();
        area3d::bake(&g, &area, &Options::default(), &tex, inputs(&source, &area), None).unwrap()
    };
    let main = bake("fx_main");
    let after_main = files_in(&dir);
    assert_eq!(after_main.len(), main.manifest.textures.len());
    for t in &main.manifest.textures {
        let bytes = std::fs::read(dir.join(&t.file)).unwrap();
        assert_eq!(t.file, content_name(&bytes, "raw"), "named by its content");
        assert_eq!(t.file.len(), 24);
        assert_eq!(bytes.len() as u64, t.bytes);
    }
    // The glb refers to them through the texture folder's URI.
    let (j, _) = read_glb(&main.glb).unwrap();
    let uris: Vec<String> = j["images"].as_array().unwrap().iter().map(|i| i["uri"].as_str().unwrap().to_string()).collect();
    assert_eq!(uris, main.manifest.textures.iter().map(|t| format!("tex/{}", t.file)).collect::<Vec<_>>());
    // Each file once, however many texture names mean it.
    let mut unique = uris.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), uris.len());
    // fx_mark is plain grass (shared with fx_main) and the marker: one new file.
    let mark = bake("fx_mark");
    let after_mark = files_in(&dir);
    let shared = mark.manifest.textures.iter().filter(|t| after_main.contains(&t.file)).count();
    assert!(shared >= 1, "the grass is shared");
    assert_eq!(after_mark.len(), after_main.len() + mark.manifest.textures.len() - shared);
    assert_eq!(tex.written(), after_mark.len());
    // The same inputs, the same bytes (a fresh texture folder too).
    let tex2 = TextureFiles::new(&f.base.join("tex2"), raw, "raw");
    let area = source.area("fx_main").unwrap();
    let again = area3d::bake(&g, &area, &Options::default(), &tex2, inputs(&source, &area), None).unwrap();
    assert_eq!(again.glb, main.glb);
    assert_eq!(serde_json::to_string(&again.manifest).unwrap(), serde_json::to_string(&main.manifest).unwrap());
    assert_eq!(files_in(&f.base.join("tex2")), after_main);
    // Every module form gives the same glb.
    for (form, path) in f.forms() {
        let s = ModuleSource::open(&path).unwrap();
        let a = s.area("fx_main").unwrap();
        let b = area3d::bake(&g, &a, &Options::default(), &tex2, inputs(&s, &a), None).unwrap();
        assert_eq!(b.glb, main.glb, "{form}");
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn a_tile_turned_any_way_lands_where_the_scene_puts_it() {
    let f = fixture::write(&fixture::dir("a3d-turns"));
    let g = open(&f);
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let area = source.area("fx_main").unwrap();
    let o = SceneOptions::default();
    let scene = nwn_bake::area::scene(&g, &area, &o).unwrap();
    let placed = placements(&g, &area, &o, &scene).unwrap();
    let turns: Vec<(Part, Turn)> = placed.placements.iter().map(|p| (p.part, p.turn)).collect();
    assert_eq!(
        &turns[..6],
        &[
            (Part::Tile(0), Turn::Quarters(0)),
            (Part::Tile(1), Turn::Quarters(1)),
            (Part::Tile(2), Turn::Quarters(2)),
            (Part::Tile(3), Turn::Quarters(0)),
            (Part::Tile(4), Turn::Quarters(3)),
            (Part::Tile(5), Turn::Quarters(0))
        ]
    );
    assert!(placed.placements.iter().all(|p| p.dropped.is_empty()));
    // A placement that drops a triangle leaves exactly that one out.
    let mut less = placed.clone();
    less.placements[0].dropped = vec![0];
    assert_eq!(less.expand().len(), scene.triangles.len() - 1);
    assert!(less.check(&scene).is_err());
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_floor_file_is_the_walkmesh_where_the_game_walks() {
    use nwn_bake::area::FloorFile;
    use nwn_bake::area::floor::{FLOOR_WALKABLE, sample_metres};
    let f = fixture::write(&fixture::dir("a3d-floor"));
    let g = open(&f);
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let tex = TextureFiles::new(&f.base.join("tex"), raw, "raw");
    let bake = |name: &str| {
        let area = source.area(name).unwrap();
        let baked = area3d::bake(&g, &area, &Options::default(), &tex, inputs(&source, &area), None).unwrap();
        let floor = FloorFile::read(&baked.floor).unwrap();
        let m = baked.manifest.floor.clone().unwrap();
        assert_eq!(
            (m.file.as_str(), m.bytes, m.samples),
            (format!("{name}.floor").as_str(), baked.floor.len() as u64, [floor.nx, floor.ny])
        );
        assert_eq!(m.walkable, floor.flags.iter().filter(|&&b| b & FLOOR_WALKABLE != 0).count());
        (floor, baked.manifest)
    };

    // fx_chasm, 3x1: plain grass, then twice the pit (a 5 m hole in the tile's middle whose floor, 3 m down, does not walk).
    let (fl, m) = bake("fx_chasm");
    assert_eq!((fl.nx, fl.ny), (49, 17));
    assert_eq!(m.version, 2);
    for j in 0..fl.ny {
        for i in 0..fl.nx {
            let (x, y) = (sample_metres(i, fl.nx), sample_metres(j, fl.ny));
            let local = x % 10.0 - 5.0;
            let in_hole = x >= 10.0 && local.abs() < fixture::PIT_HALF && (y - 5.0).abs() < fixture::PIT_HALF;
            let k = fl.index(i, j);
            assert_eq!(fl.flags[k] & FLOOR_WALKABLE != 0, !in_hole, "({i}, {j}) at ({x}, {y})");
            if !in_hole {
                assert_eq!(fl.heights[k], 0, "({i}, {j})");
            }
        }
    }
    // The hole: 8 by 8 samples in each pit tile.
    assert_eq!(m.floor.as_ref().unwrap().walkable, 49 * 17 - 2 * 64);

    // fx_main: tile 0's grass walks over its pond's deep bed; the rock's foot in its north-east corner does not; the tile turned
    // three quarters (row 1, middle) stands 2 m up.
    let (fl, _) = bake("fx_main");
    assert_eq!((fl.nx, fl.ny), (49, 33));
    let walk = |i: usize, j: usize| fl.flags[fl.index(i, j)] & FLOOR_WALKABLE != 0;
    // (The pond: a triangle from (1, 1) and (3, 1) up to (3, 3); sample 4 is at 2.51 m, sample 2 at 1.26 m.)
    assert!(walk(4, 2) && fl.heights[fl.index(4, 2)] == 0);
    // (The rock's foot: a triangle from (8, 8) and (9.5, 8) up to (8.75, 9.5); sample 13 is 8.135 m.)
    assert!(!walk(13, 13) && !walk(14, 13) && walk(12, 13) && walk(12, 12));
    assert!(walk(24, 24) && fl.heights[fl.index(24, 24)] == 200);
    // Nothing here has a placeable walkmesh: nothing is blocked.
    assert!(fl.flags.iter().all(|&b| b & !FLOOR_WALKABLE == 0));
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_batch_reuses_the_output_folder_and_the_cache_by_key() {
    let f = fixture::write(&fixture::dir("a3d-batch"));
    let g = open(&f);
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let out = f.base.join("out");
    let job = Job {
        options: Options::default(),
        out: out.clone(),
        tex_dir: out.join("tex"),
        cache: Some(f.base.join("cache")),
        force: false,
        strict: false,
        jobs: 2,
        tlk: None,
    };
    let areas: Vec<String> = ["fx_main", "fx_mark", "nope"].map(String::from).to_vec();
    let run = |job: &Job| batch::run(&g, &source, &areas, job, raw, &|_| {});
    let actions = |r: &[batch::AreaResult]| r.iter().map(|r| r.action).collect::<Vec<_>>();

    let (first, written) = run(&job);
    assert_eq!(actions(&first), [Action::Baked, Action::Baked, Action::Failed]);
    assert!(first[2].error.as_deref().unwrap().contains("nope"), "{:?}", first[2].error);
    assert!(written > 0);
    let glb = std::fs::read(out.join("fx_main.glb")).unwrap();
    let floor = std::fs::read(out.join("fx_main.floor")).unwrap();
    let manifest: Value = serde_json::from_slice(&std::fs::read(out.join("fx_main.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], "nwn-bake/area-3d");
    assert_eq!((manifest["version"].as_u64(), manifest["floor"]["file"].as_str()), (Some(2), Some("fx_main.floor")));
    assert_eq!(manifest["floor"]["bytes"].as_u64(), Some(floor.len() as u64));
    assert_eq!(manifest["key"], first[0].key.clone().unwrap());

    // Again: nothing to do.
    let (second, written) = run(&job);
    assert_eq!(actions(&second), [Action::Current, Action::Current, Action::Failed]);
    assert_eq!(written, 0);
    assert_eq!(second[0].key, first[0].key);

    // The output folder gone: from the cache, the same bytes.
    std::fs::remove_dir_all(&out).unwrap();
    let (third, _) = run(&job);
    assert_eq!(actions(&third), [Action::Cached, Action::Cached, Action::Failed]);
    assert_eq!(std::fs::read(out.join("fx_main.glb")).unwrap(), glb);
    assert_eq!(std::fs::read(out.join("fx_main.floor")).unwrap(), floor);
    assert!(third[0].textures_copied > 0);

    // The floor file missing: not current any more (from the cache again).
    std::fs::remove_file(out.join("fx_main.floor")).unwrap();
    assert_eq!(run(&job).0[0].action, Action::Cached);
    assert_eq!(std::fs::read(out.join("fx_main.floor")).unwrap(), floor);

    // A manifest of version 1 (before the floor file) under the same key, here and in the cache: baked again, to the same .glb.
    // (Its key differs since GLB_VERSION 2; a manifest copied by hand under a new key must still not count.)
    let mut old = manifest.clone();
    old["version"] = 1.into();
    old.as_object_mut().unwrap().remove("floor");
    let key_dir = f.base.join("cache").join(first[0].key.as_deref().unwrap().trim_start_matches("sha256:"));
    for dir in [&out, &key_dir] {
        std::fs::write(dir.join("fx_main.json"), serde_json::to_vec(&old).unwrap()).unwrap();
    }
    let (again, _) = run(&job);
    assert_eq!(again[0].action, Action::Baked, "{:?}", again[0].error);
    assert_eq!(again[0].key, first[0].key);
    assert_eq!(std::fs::read(out.join("fx_main.glb")).unwrap(), glb);
    assert_eq!(std::fs::read(out.join("fx_main.floor")).unwrap(), floor);

    // A texture missing from the output folder: not current any more. (fx_main alone: the areas share textures, and fx_mark, on
    // the other thread, would copy the texture back from the cache before fx_main is checked, which then finds it current.)
    let t = &serde_json::from_value::<area3d::Manifest>(manifest).unwrap().textures[0];
    std::fs::remove_file(out.join("tex").join(&t.file)).unwrap();
    let (fourth, _) = batch::run(&g, &source, &areas[..1], &job, raw, &|_| {});
    assert_eq!(fourth[0].action, Action::Cached, "{:?}", fourth[0].error);
    assert!(out.join("tex").join(&t.file).is_file());

    // Another option is another key; --force bakes whatever the keys say.
    let smaller = Job { options: Options { max_tex: 4, ..Options::default() }, ..job.clone() };
    let (fifth, _) = run(&smaller);
    assert_eq!(actions(&fifth)[..2], [Action::Baked, Action::Baked]);
    assert_ne!(fifth[0].key, first[0].key);
    let (sixth, _) = run(&Job { force: true, ..smaller });
    assert_eq!(actions(&sixth)[..2], [Action::Baked, Action::Baked]);
    let _ = std::fs::remove_dir_all(&f.base);
}

/// The fixture's areas, with area `bad`'s work panicking.
struct Panics<'a> {
    source: &'a ModuleSource,
    bad: &'a str,
}

impl nwn_bake::iso::batch::AreaSource for Panics<'_> {
    fn area(&self, resref: &str) -> anyhow::Result<nwn_bake::module::AreaFiles> {
        assert_ne!(resref, self.bad, "a broken area");
        self.source.area(resref)
    }
}

#[test]
fn a_panic_fails_its_area_and_the_batch_ends() {
    let f = fixture::write(&fixture::dir("a3d-panic"));
    let g = open(&f);
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let areas: Vec<String> = ["fx_main", "fx_mark", "fx_flat"].map(String::from).to_vec();
    for jobs in [1, 2] {
        let out = f.base.join(format!("out-{jobs}"));
        let job = Job {
            options: Options::default(),
            out: out.clone(),
            tex_dir: out.join("tex"),
            cache: None,
            force: false,
            strict: false,
            jobs,
            tlk: None,
        };
        // An area whose work panics fails with the panic's message; the others are baked.
        let (r, _) = batch::run(&g, &Panics { source: &source, bad: "fx_mark" }, &areas, &job, raw, &|_| {});
        assert_eq!(r.iter().map(|r| r.action).collect::<Vec<_>>(), [Action::Baked, Action::Failed, Action::Baked]);
        assert!(r[1].error.as_deref().is_some_and(|e| e.contains("panicked") && e.contains("a broken area")), "{:?}", r[1].error);
        // A report that panics (a progress line to a closed stderr) loses nothing: every area has its result.
        let (r, _) = batch::run(&g, &source, &areas, &Job { force: true, ..job }, raw, &|r| panic!("cannot report {}", r.area));
        assert!(r.iter().all(|r| r.action == Action::Baked), "{:?}", r.iter().map(|r| &r.error).collect::<Vec<_>>());
    }
    let _ = std::fs::remove_dir_all(&f.base);
}
