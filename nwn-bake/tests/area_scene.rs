// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An area's scene on a made-up tileset (no game content): tiles turned and lifted as placed, water apart, a material resolved, a
//! placeable, the floor probe on the walkmeshes, and the same scene, bit for bit, every time it is built.

use std::path::{Path, PathBuf};

use mg_core::{ResRef, ResType};
use mg_gff::{Gff, Struct, Value};
use nwn_bake::area::{Kind, Part, RoofRule, SceneOptions};
use nwn_bake::game::{self, GameOptions, Haks, Tlk};
use nwn_bake::module::AreaFiles;

/// A 2x2 uncompressed 32-bit TGA of one colour.
fn tga(rgba: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 8];
    for _ in 0..4 {
        out.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
    }
    out
}

/// An ASCII trimesh node: `verts` (x y z), faces of three vertex numbers, the UVs the vertices' x and y over 10 m.
fn trimesh(name: &str, parent: &str, bitmap: &str, verts: &[[f32; 3]], faces: &[[usize; 3]]) -> String {
    let mut s = format!("node trimesh {name}\n  parent {parent}\n  bitmap {bitmap}\n  verts {}\n", verts.len());
    for v in verts {
        s += &format!("    {} {} {}\n", v[0], v[1], v[2]);
    }
    s += &format!("  tverts {}\n", verts.len());
    for v in verts {
        s += &format!("    {} {} 0\n", (v[0] + 5.0) / 10.0, (v[1] + 5.0) / 10.0);
    }
    s += &format!("  faces {}\n", faces.len());
    for f in faces {
        s += &format!("    {} {} {} 1 {} {} {} 1\n", f[0], f[1], f[2], f[0], f[1], f[2]);
    }
    s + "endnode\n"
}

fn model(name: &str, meshes: &[String]) -> Vec<u8> {
    let mut s = format!(
        "newmodel {name}\nsetsupermodel {name} NULL\nclassification TILE\nbeginmodelgeom {name}\nnode dummy {name}\n  parent NULL\nendnode\n"
    );
    for m in meshes {
        s += m;
    }
    s += &format!("endmodelgeom {name}\ndonemodel {name}\n");
    s.into_bytes()
}

/// A tile walkmesh: faces of (three corners, surface material).
fn walkmesh(name: &str, faces: &[([[f32; 3]; 3], u32)]) -> Vec<u8> {
    let mut s = format!(
        "#MAXWALKMESH  ASCII\nbeginwalkmeshgeom {name}\nnode aabb {name}_wg\n  parent {name}\n  position 0 0 0\n  orientation 0 0 0 0\n"
    );
    s += &format!("  verts {}\n", faces.len() * 3);
    for (f, _) in faces {
        for v in f {
            s += &format!("    {} {} {}\n", v[0], v[1], v[2]);
        }
    }
    s += &format!("  faces {}\n", faces.len());
    for (k, (_, m)) in faces.iter().enumerate() {
        s += &format!("    {} {} {} 1 0 0 0 {m}\n", k * 3, k * 3 + 1, k * 3 + 2);
    }
    s += &format!("endnode\nendwalkmeshgeom {name}\n");
    s.into_bytes()
}

const QUAD: [[usize; 3]; 2] = [[0, 1, 2], [0, 2, 3]];

fn square(z: f32) -> Vec<[f32; 3]> {
    vec![[-5.0, -5.0, z], [5.0, -5.0, z], [5.0, 5.0, z], [-5.0, 5.0, z]]
}

fn walk_square(z: f32, material: u32) -> Vec<([[f32; 3]; 3], u32)> {
    let s = square(z);
    vec![([s[0], s[1], s[2]], material), ([s[0], s[2], s[3]], material)]
}

/// The tileset tst01: tile 0 grass with a rock standing in its north-east corner (drawn through a material) and a pond, tile 1
/// black void.
fn tileset() -> Vec<u8> {
    let s = "[GENERAL]\nName=TST01\nType=SET\nVersion=V1.0\nInterior=0\nHasHeightTransition=1\nEnvMap=\nTransition=2.0\nDisplayName=-1\nBorder=\nDefault=\nFloor=\n\n\
[GRASS]\nGrass=0\n\n[TERRAIN TYPES]\nCount=0\n\n[CROSSER TYPES]\nCount=0\n\n[PRIMARY RULES]\nCount=0\n\n[SECONDARY RULES]\nCount=0\n\n\
[TILES]\nCount=2\n\n[TILE0]\nModel=tst01_a01\nWalkMesh=msb01\nTopLeft=\nTopLeftHeight=0\nTopRight=\nTopRightHeight=0\nBottomLeft=\nBottomLeftHeight=0\nBottomRight=\nBottomRightHeight=0\nTop=\nRight=\nBottom=\nLeft=\nMainLight1=0\nMainLight2=0\nSourceLight1=0\nSourceLight2=0\nAnimLoop1=0\nAnimLoop2=0\nAnimLoop3=0\nDoors=0\nSounds=0\nPathNode=A\nOrientation=0\nImageMap2D=\n\n\
[TILE1]\nModel=tst01_b01\nWalkMesh=msb01\nTopLeft=\nTopLeftHeight=0\nTopRight=\nTopRightHeight=0\nBottomLeft=\nBottomLeftHeight=0\nBottomRight=\nBottomRightHeight=0\nTop=\nRight=\nBottom=\nLeft=\nMainLight1=0\nMainLight2=0\nSourceLight1=0\nSourceLight2=0\nAnimLoop1=0\nAnimLoop2=0\nAnimLoop3=0\nDoors=0\nSounds=0\nPathNode=A\nOrientation=0\nImageMap2D=\n\n\
[GROUPS]\nCount=0\n";
    s.replace('\n', "\r\n").into_bytes()
}

fn install(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("nwn-bake-scene-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, user) = (base.join("game"), base.join("user"));
    std::fs::create_dir_all(root.join("lang/en/data")).unwrap();
    std::fs::create_dir_all(user.join("hak")).unwrap();
    std::fs::write(root.join("lang/en/data/dialog.tlk"), mg_tlk::Tlk::new(mg_core::Language::ENGLISH).to_bytes().unwrap()).unwrap();
    let rock = [[3.0, 3.0, 0.0], [4.5, 3.0, 0.0], [3.75, 4.5, 2.5]];
    let pond = [[-4.0, -4.0, -0.5], [-2.0, -4.0, -0.5], [-2.0, -2.0, -0.5]];
    let files: Vec<(&str, ResType, Vec<u8>)> = vec![
        ("tst01", ResType::SET, tileset()),
        (
            "tst01_a01",
            ResType::MDL,
            model(
                "tst01_a01",
                &[
                    trimesh("ground", "tst01_a01", "tst01_grass", &square(0.0), &QUAD),
                    trimesh("rock", "tst01_a01", "tst01_mat", &rock, &[[0, 1, 2]]),
                    trimesh("pond", "tst01_a01", "tst01_water", &pond, &[[0, 1, 2]]),
                ],
            ),
        ),
        // Grass walks; the pond's bed is deep water (row 17, below the grass); the rock's foot does not walk.
        (
            "tst01_a01",
            ResType::WOK,
            walkmesh(
                "tst01_a01",
                &[walk_square(0.0, 1), vec![(pond.map(|v| [v[0], v[1], -1.5]), 17), (rock.map(|v| [v[0], v[1], 0.1]), 2)]].concat(),
            ),
        ),
        ("tst01_b01", ResType::MDL, model("tst01_b01", &[trimesh("void", "tst01_b01", "tst01_void", &square(0.0), &QUAD)])),
        ("tst01_b01", ResType::WOK, walkmesh("tst01_b01", &walk_square(0.0, 1))),
        (
            "plc_box",
            ResType::MDL,
            model(
                "plc_box",
                &[trimesh("box", "plc_box", "tst01_grass", &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 1.0]], &[[0, 1, 2]])],
            ),
        ),
        ("tst01_grass", ResType::TGA, tga([40, 160, 40, 255])),
        ("tst01_void", ResType::TGA, tga([0, 0, 0, 255])),
        ("tst01_stone", ResType::TGA, tga([128, 128, 128, 255])),
        ("tst01_mat", ResType::MTR, b"texture0 tst01_stone\n".to_vec()),
        ("tst01_water", ResType::TGA, tga([0, 0, 255, 255])),
        ("surfacemat", ResType::TWODA, b"2DA V2.0\n\n   Label Walk\n0  NotDefined 0\n1  Grass 1\n2  Stone 0\n".to_vec()),
        ("placeables", ResType::TWODA, b"2DA V2.0\n\n   Label ModelName StrRef\n0  Box plc_box ****\n".to_vec()),
    ];
    let mut w = mg_erf::ErfWriter::new(*b"HAK ");
    for (name, t, data) in files {
        w.add(ResRef::from_str(name).unwrap(), t, data).unwrap();
    }
    std::fs::write(user.join("hak/tst.hak"), w.to_bytes().unwrap()).unwrap();
    (base, root, user)
}

fn open(root: &Path, user: &Path) -> game::Game {
    let g = game::open(&GameOptions {
        root: Some(root.to_path_buf()),
        user: Some(user.to_path_buf()),
        haks: Haks::List(vec!["tst".into()]),
        tlk: Tlk::None,
    })
    .unwrap();
    assert!(g.missing_haks().is_empty());
    g
}

fn item(id: u32, fields: &[(&str, Value)]) -> Struct {
    let mut s = Struct::new(id);
    for (label, v) in fields {
        s.set(label, v.clone());
    }
    s
}

/// A 3 by 2 area: row 0 is grass turned 0, 1 and 2 quarter turns, row 1 is void, grass a step up (2 m), and void turned 3 times.
/// One placeable, the box, at (12, 3, 0) turned a quarter.
fn area() -> AreaFiles {
    let tile = |id: i32, o: i32, h: i32| {
        item(1, &[("Tile_ID", Value::Int(id)), ("Tile_Orientation", Value::Int(o)), ("Tile_Height", Value::Int(h))])
    };
    let mut are = Gff::new(*b"ARE ");
    are.root = item(
        u32::MAX,
        &[
            ("Tileset", Value::ResRef(b"tst01".to_vec())),
            ("Width", Value::Int(3)),
            ("Height", Value::Int(2)),
            ("Flags", Value::Dword(2)),
            ("SunAmbientColor", Value::Dword(0x00332211)),
            ("SunDiffuseColor", Value::Dword(0x00ccbbaa)),
            ("Tile_List", Value::List(vec![tile(0, 0, 0), tile(0, 1, 0), tile(0, 2, 0), tile(1, 0, 0), tile(0, 0, 1), tile(1, 3, 0)])),
        ],
    );
    let mut git = Gff::new(*b"GIT ");
    let box_ = item(
        9,
        &[
            ("Appearance", Value::Dword(0)),
            ("X", Value::Float(12.0)),
            ("Y", Value::Float(3.0)),
            ("Z", Value::Float(0.0)),
            ("Bearing", Value::Float(std::f32::consts::FRAC_PI_2)),
        ],
    );
    git.root = item(u32::MAX, &[("Placeable List", Value::List(vec![box_]))]);
    AreaFiles { resref: "tst_area".into(), are, git: Some(git) }
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5)
}

#[test]
fn the_scene_has_the_tiles_as_placed_water_apart_and_the_placeables() {
    let (base, root, user) = install("parts");
    let g = open(&root, &user);
    let s = nwn_bake::area::scene(&g, &area(), &SceneOptions::default()).unwrap();
    assert!(s.notes.is_empty(), "{:?}", s.notes);
    assert_eq!(
        (s.tiles, s.size(), s.kind(), s.light.ambient, s.light.diffuse),
        ([3, 2], [30.0, 20.0], Kind::Underground, [0x11, 0x22, 0x33], [0xaa, 0xbb, 0xcc])
    );
    // Four grass tiles of a ground quad and a rock, two void tiles of a quad, the box's triangle; each grass tile's pond is water.
    assert_eq!(
        (s.counts.tile_triangles, s.counts.object_triangles, s.water.len(), s.counts.tile_models, s.counts.placeables),
        (16, 1, 4, 2, 1)
    );
    // Textures by first use; the rock's material means the stone image.
    let names: Vec<&str> = s.textures.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["tst01_grass", "tst01_mat", "tst01_void"]);
    assert_eq!(s.textures[1].image.as_ref().unwrap().image, "tst01_stone");
    // The rock's top, (3.75, 4.5) about the tile's middle: tile 0 as it is, tile 1 a quarter turn (to (-4.5, 3.75)), tile 2 a half
    // turn; tile 4 is a step (2 m) up.
    let top = |tile: u32| s.triangles.iter().find(|t| t.part == Part::Tile(tile) && t.texture == 1).unwrap().p[2];
    assert!(close(top(0), [8.75, 9.5, 2.5]));
    assert!(close(top(1), [15.0 - 4.5, 5.0 + 3.75, 2.5]));
    assert!(close(top(2), [25.0 - 3.75, 5.0 - 4.5, 2.5]));
    assert!(close(top(4), [15.0 + 3.75, 15.0 + 4.5, 4.5]));
    // The box, turned a quarter about its origin: its (1, 0, 0) corner goes to (0, 1).
    let b = s.triangles.iter().find(|t| matches!(t.part, Part::Placeable(0))).unwrap();
    assert!(close(b.p[1], [12.0, 4.0, 0.0]), "{:?}", b.p);
    // Without objects: no box.
    let bare = nwn_bake::area::scene(&g, &area(), &SceneOptions { objects: false, ..Default::default() }).unwrap();
    assert_eq!((bare.counts.object_triangles, bare.triangles.len()), (0, 16));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn the_floor_probe_reads_the_walkmeshes_and_sees_the_void() {
    let (base, root, user) = install("probe");
    let g = open(&root, &user);
    let s = nwn_bake::area::scene(&g, &area(), &SceneOptions::default()).unwrap();
    let p = &s.probe;
    assert_eq!((p.nx, p.ny, p.heights.len()), (49, 33, 49 * 33));
    let at = |x: f32, y: f32| {
        let k = (y / 0.625).round() as usize * p.nx + (x / 0.625).round() as usize;
        (p.heights[k], p.colours[k], p.is_void(k))
    };
    // Grass on tile 0, the raised grass on tile 4, void on tile 3.
    assert_eq!(at(1.25, 7.5), (0.0, [40, 160, 40], false));
    assert_eq!(at(16.25, 17.5), (2.0, [40, 160, 40], false));
    assert_eq!(at(1.25, 15.0), (0.0, [0, 0, 0], true));
    // Over the pond's bed (deep water, below the grass) the grass is the top; the rock's foot does not walk, the grass under it does.
    assert_eq!(at(2.5, 2.5).0, 0.0);
    assert_eq!(at(8.75, 8.75).0, 0.0);
    let stats = p.stats();
    assert_eq!((stats.roof, stats.drawn_share > 0.6, stats.median), (2.0, true, 0.0));
    let cut = s.roof_cut(&RoofRule::default()).unwrap();
    assert_eq!((cut.at, cut.slab), (3.5, false));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_scene_built_twice_is_the_same_bit_for_bit() {
    let (base, root, user) = install("twice");
    // Two games opened apart (nothing cached shared), and one game asked twice (its caches warm).
    let (g1, g2) = (open(&root, &user), open(&root, &user));
    let a = nwn_bake::area::scene(&g1, &area(), &SceneOptions::default()).unwrap();
    let b = nwn_bake::area::scene(&g2, &area(), &SceneOptions::default()).unwrap();
    let c = nwn_bake::area::scene(&g1, &area(), &SceneOptions::default()).unwrap();
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a.digest(), c.digest());
    assert_eq!(a.triangles, b.triangles);
    assert_eq!((&a.water, &a.probe, &a.notes), (&b.water, &b.probe, &b.notes));
    // And it changes when the area does: the box moved a centimetre.
    let mut moved = area();
    let Some(Value::List(list)) = moved.git.as_mut().unwrap().root.get_mut("Placeable List") else { unreachable!() };
    list[0].set("X", Value::Float(12.01));
    assert_ne!(nwn_bake::area::scene(&g1, &moved, &SceneOptions::default()).unwrap().digest(), a.digest());
    // The roof cut keeps what is below and cuts what crosses: the four rocks (up to 2.5 m, 4.5 m on the raised tile) cross 2 m, each cut into two.
    let mut cut = a.clone();
    cut.cut_roof(2.0);
    assert!(cut.triangles.iter().all(|t| t.p.iter().all(|p| p[2] <= 2.0 + 1e-5)));
    assert_eq!(cut.triangles.len(), a.triangles.len() + 4);
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn an_area_past_32_tiles_or_with_a_tile_list_of_another_length_fails() {
    let (base, root, user) = install("sizes");
    let g = open(&root, &user);
    let with = |width: i32, height: i32, tiles: Option<usize>| {
        let mut a = area();
        a.are.root.set("Width", Value::Int(width));
        a.are.root.set("Height", Value::Int(height));
        if let Some(n) = tiles {
            let one = a.are.root.list("Tile_List").unwrap()[0].clone();
            a.are.root.set("Tile_List", Value::List(vec![one; n]));
        }
        nwn_bake::area::scene(&g, &a, &SceneOptions::default()).map(|s| s.tiles)
    };
    assert_eq!(with(3, 2, None).unwrap(), [3, 2]);
    // (An .are claiming 1,024 by 1,024 tiles once built a million of them, minutes per area.)
    let e = format!("{:#}", with(1024, 1024, None).unwrap_err());
    assert!(e.contains("Width is 1024 tiles; an area has 0 to 32"), "{e}");
    assert!(format!("{:#}", with(3, 40, None).unwrap_err()).contains("Height is 40 tiles"));
    assert!(format!("{:#}", with(-1, 2, None).unwrap_err()).contains("Width is -1 tiles"));
    let e = format!("{:#}", with(3, 2, Some(5)).unwrap_err());
    assert!(e.contains("Tile_List has 5 tiles for a 3 by 2 area"), "{e}");
    assert!(with(3, 2, Some(7)).is_err());
    assert_eq!(with(32, 32, Some(1024)).unwrap(), [32, 32]);
    let _ = std::fs::remove_dir_all(&base);
}
