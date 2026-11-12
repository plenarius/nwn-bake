// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A small made-up game world for nwn-bake's tests, with no file of the game in it: everything is generated here, from code, with
//! Moonglow's writers (ERF, GFF, talk table). It is safe to publish with the source.
//!
//! [`write`] lays it out under a folder:
//!
//! - `game/`: an "install" with nothing but an empty talk table (`lang/en/data/dialog.tlk`), so the game opens without NWN;
//! - `user/hak/fx_tiles.hak`: the tileset `fx01`, its tile models (ASCII `.mdl`) and walkmeshes (ASCII `.wok`), checkerboard TGA
//!   textures, a black void texture, a pit texture (black but for its border), a material (`fx_rock.mtr`, naming `fx_stone`), the placeable and door models, and the 2DAs
//!   the scene reads (`surfacemat`, `placeables`, `genericdoors`);
//! - `user/hak/fx_over.hak`: one texture also in `fx_tiles` (`fx_marker`), red here and blue there. The module lists `fx_over`
//!   first, so the marker is red (the first hak listed wins);
//! - the module, the same in every form `nwn_bake::module::ModuleSource` reads: `fx.mod`, `folder/` (binary files),
//!   `json/` (nwn_gff's JSON with no `nasher.cfg`, in subfolders) and `project/` (a nasher project, `src/areas/*.json`).
//!
//! The tileset `fx01` (`Transition=2.0`) counts four terrains and has three (`[TERRAIN2]` is absent, as in mst03): `Grass`, `Rock`
//! and `Chasm`. Every tile's corners are `Grass` but tile 5's, which are `Chasm`. Its tiles, each 10 m square about its middle:
//!
//! | Tile | Model | What it has |
//! |---|---|---|
//! | 0 | `fx01_a01` | grass at 0 m; a rock (through the material `fx_rock`) standing in its north-east corner, its top at (3.75, 4.5, 2.5); a pond (water, 0.1 m over the grass) in its south-west corner, its bed deep water |
//! | 1 | `fx01_b01` | black void at 0 m (walkable) |
//! | 2 | `fx01_c01` | plain grass at 0 m |
//! | 3 | `fx01_d01` | a rock top at 6 m (walkable) |
//! | 4 | `fx01_e01` | a room: grass floor at 0 m, a stone ceiling at 5 m |
//! | 5 | `fx01_f01` | a pit: grass at 0 m around a 5 m square hole in its middle, stone walls down to its floor at 3 m below ([`PIT_FLOOR`]), drawn with `fx_pit`, black where the floor is; the floor's walkmesh faces do not walk. Its corners are `Chasm`: a chasm |
//! | 6 | `fx01_f01` | the same model with `Grass` corners: a black floor that is no chasm |
//!
//! The areas (all in every module form; `.git` positions with four decimals at most, as nasher keeps them):
//!
//! | Area | Tiles | Kind | What it is for |
//! |---|---|---|---|
//! | `fx_main` | 3x2 | surface | row 0 grass turned 0, 1 and 2 quarter turns; row 1 void, grass turned 3 and a step (2 m) up, plain. A box at (12, 3) turned a quarter, a door at (25, 15) |
//! | `fx_mark` | 3x2 | surface | plain grass and one marker: a red 1 m square 1 m over its origin, at [`MARKER`] |
//! | `fx_flat` | 2x2 | interior | rooms: cut 3.5 m over the floor, which takes the ceiling away |
//! | `fx_cave` | 3x2 | underground | rock top (6 m) over two tiles of floor (0 m): a cave dug out of rock, cut 2 m over its highest floor |
//! | `fx_step` | 2x2 | interior | grass two steps (4 m) up over black void: no cave, cut 3.5 m over the grass |
//! | `fx_chasm` | 3x1 | surface | plain grass, the chasm (tile 5) and the pit without chasm terrain (tile 6), west to east |
//!
//! The ground's textures have no red in them, so a test can find the red marker in a picture by its red alone.

#![allow(dead_code, reason = "each test file that includes the fixture uses only part of it")]

use std::path::{Path, PathBuf};

use mg_core::{Gender, Language, LocString, ResRef, ResType};
use mg_gff::{Gff, Struct, Value};

/// The module's haks, first highest.
pub const HAKS: [&str; 2] = ["fx_over", "fx_tiles"];
/// The areas, in the module's area list's order.
pub const AREAS: [&str; 6] = ["fx_main", "fx_mark", "fx_flat", "fx_cave", "fx_step", "fx_chasm"];
/// Where `fx_mark`'s marker stands (its origin; the red square is 1 m over it).
pub const MARKER: [f32; 3] = [12.5, 7.5, 0.0];
/// The marker's red square: 1 m over its origin.
pub const MARKER_UP: f32 = 1.0;

/// Where the fixture was written.
pub struct Fixture {
    pub base: PathBuf,
    /// The made-up install (`--root`).
    pub root: PathBuf,
    /// The user folder with the haks (`--user`).
    pub user: PathBuf,
}

impl Fixture {
    pub fn module_mod(&self) -> PathBuf {
        self.base.join("fx.mod")
    }
    pub fn folder(&self) -> PathBuf {
        self.base.join("folder")
    }
    pub fn json_folder(&self) -> PathBuf {
        self.base.join("json")
    }
    pub fn project(&self) -> PathBuf {
        self.base.join("project")
    }
    /// Every form of the module, named: what `ModuleSource::open` is given.
    pub fn forms(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            (".mod", self.module_mod()),
            ("folder", self.folder()),
            ("JSON folder", self.json_folder()),
            ("nasher project", self.project()),
            ("nasher.cfg", self.project().join("nasher.cfg")),
        ]
    }
    /// One area's file in each form that opens a single area: binary in `folder/`, JSON in the nasher project.
    pub fn area_files(&self, area: &str) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("area file", self.folder().join(format!("{area}.are"))),
            ("area JSON file", self.project().join(format!("src/areas/{area}.are.json"))),
        ]
    }
}

/// Writes the fixture under `base` (emptied first).
pub fn write(base: &Path) -> Fixture {
    let _ = std::fs::remove_dir_all(base);
    let (root, user) = (base.join("game"), base.join("user"));
    mkdir(&root.join("lang/en/data"));
    mkdir(&user.join("hak"));
    std::fs::write(root.join("lang/en/data/dialog.tlk"), mg_tlk::Tlk::new(Language::ENGLISH).to_bytes().unwrap()).unwrap();
    write_hak(&user.join("hak/fx_tiles.hak"), &tiles_hak());
    write_hak(&user.join("hak/fx_over.hak"), &over_hak());
    write_module(base);
    Fixture { base: base.to_path_buf(), root, user }
}

/// A folder for a test's fixture: under Cargo's temporary folder for tests when there is one, else the system's.
pub fn dir(tag: &str) -> PathBuf {
    let tmp = option_env!("CARGO_TARGET_TMPDIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    tmp.join(format!("nwn-bake-fixture-{tag}-{}", std::process::id()))
}

fn mkdir(p: &Path) {
    std::fs::create_dir_all(p).unwrap();
}

/// Writes a hak of `files`.
pub fn write_hak(path: &Path, files: &[(String, ResType, Vec<u8>)]) {
    let mut w = mg_erf::ErfWriter::new(*b"HAK ");
    for (name, t, data) in files {
        w.add(ResRef::from_str(name).unwrap(), *t, data.clone()).unwrap();
    }
    std::fs::write(path, w.to_bytes().unwrap()).unwrap();
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Textures.

/// An uncompressed 32-bit TGA, `w` by `h`, rows from the bottom, each texel `f(x, y)` (RGBA).
pub fn tga(w: u16, h: u16, f: impl Fn(u16, u16) -> [u8; 4]) -> Vec<u8> {
    let mut out = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&[32, 8]);
    for y in 0..h {
        for x in 0..w {
            let c = f(x, y);
            out.extend_from_slice(&[c[2], c[1], c[0], c[3]]);
        }
    }
    out
}

/// An 8 by 8 checkerboard of two colours (squares of 2 texels).
pub fn checker(a: [u8; 3], b: [u8; 3]) -> Vec<u8> {
    tga(8, 8, |x, y| {
        let c = if (x / 2 + y / 2) % 2 == 0 { a } else { b };
        [c[0], c[1], c[2], 255]
    })
}

/// One colour, 4 by 4.
pub fn plain(c: [u8; 3]) -> Vec<u8> {
    tga(4, 4, |_, _| [c[0], c[1], c[2], 255])
}

pub const GRASS: ([u8; 3], [u8; 3]) = ([0, 150, 40], [0, 110, 60]);
pub const STONE: ([u8; 3], [u8; 3]) = ([0, 120, 140], [0, 90, 110]);
pub const CRATE: [u8; 3] = [0, 80, 160];
pub const WOOD: [u8; 3] = [0, 60, 90];

// ---------------------------------------------------------------------------------------------------------------------------------
// Models and walkmeshes.

/// An ASCII trimesh node: `verts` (x y z), faces of three vertex numbers; each vertex's UV is its x and y over 10 m.
pub fn trimesh(name: &str, parent: &str, bitmap: &str, verts: &[[f32; 3]], faces: &[[usize; 3]]) -> String {
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

/// An ASCII model of `meshes` under one dummy root.
pub fn model(name: &str, classification: &str, meshes: &[String]) -> Vec<u8> {
    let mut s = format!(
        "newmodel {name}\nsetsupermodel {name} NULL\nclassification {classification}\nbeginmodelgeom {name}\nnode dummy {name}\n  parent NULL\nendnode\n"
    );
    for m in meshes {
        s += m;
    }
    s += &format!("endmodelgeom {name}\ndonemodel {name}\n");
    s.into_bytes()
}

/// An ASCII tile walkmesh: faces of (three corners, surface material).
pub fn walkmesh(name: &str, faces: &[([[f32; 3]; 3], u32)]) -> Vec<u8> {
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

pub const QUAD: [[usize; 3]; 2] = [[0, 1, 2], [0, 2, 3]];

/// A tile's whole square at height `z`.
pub fn square(z: f32) -> Vec<[f32; 3]> {
    vec![[-5.0, -5.0, z], [5.0, -5.0, z], [5.0, 5.0, z], [-5.0, 5.0, z]]
}

/// A tile's whole square at height `z` as walkmesh faces of `material`.
pub fn walk_square(z: f32, material: u32) -> Vec<([[f32; 3]; 3], u32)> {
    let s = square(z);
    vec![([s[0], s[1], s[2]], material), ([s[0], s[2], s[3]], material)]
}

/// Surface materials: 1 walks, 2 (stone) does not, 17 is deep water.
pub const WALK: u32 = 1;
pub const NO_WALK: u32 = 2;
pub const DEEP_WATER: u32 = 17;

/// Tile 0's rock, a triangle standing in the north-east corner, and its pond's water in the south-west corner, 0.1 m over the grass
/// (so a picture shows it; its bed, in the walkmesh, is 1.5 m down).
pub const ROCK: [[f32; 3]; 3] = [[3.0, 3.0, 0.0], [4.5, 3.0, 0.0], [3.75, 4.5, 2.5]];
pub const POND: [[f32; 3]; 3] = [[-4.0, -4.0, 0.1], [-2.0, -4.0, 0.1], [-2.0, -2.0, 0.1]];

/// A tile's fields in the `.set` but its model, its four corners of terrain `corner`.
fn tile_fields(corner: &str) -> String {
    format!(
        "WalkMesh=msb01\nTopLeft={corner}\nTopLeftHeight=0\nTopRight={corner}\nTopRightHeight=0\nBottomLeft={corner}\nBottomLeftHeight=0\n\
BottomRight={corner}\nBottomRightHeight=0\nTop=\nRight=\nBottom=\nLeft=\nMainLight1=0\nMainLight2=0\nSourceLight1=0\nSourceLight2=0\n\
AnimLoop1=0\nAnimLoop2=0\nAnimLoop3=0\nDoors=0\nSounds=0\nPathNode=A\nOrientation=0\nImageMap2D=\n"
    )
}

/// The tile models, in the tileset's order (tiles 5 and 6 share the pit's model).
pub const TILE_MODELS: [&str; 7] = ["fx01_a01", "fx01_b01", "fx01_c01", "fx01_d01", "fx01_e01", "fx01_f01", "fx01_f01"];
/// The tile whose corners are `Chasm` (the rest are `Grass`).
pub const CHASM_TILE: usize = 5;
/// The tile with the chasm's model and `Grass` corners: its black floor is no chasm.
pub const PIT_TILE: usize = 6;
/// The pit's floor: this far below the tile's floor (metres), a square this far from its middle each way.
pub const PIT_FLOOR: f32 = -3.0;
pub const PIT_HALF: f32 = 2.5;

/// The tileset `fx01`: three terrains counted, two there.
pub fn tileset() -> Vec<u8> {
    let mut s = String::from(
        "[GENERAL]\nName=FX01\nType=SET\nVersion=V1.0\nInterior=0\nHasHeightTransition=1\nEnvMap=\nTransition=2.0\nDisplayName=-1\nBorder=Grass\nDefault=Grass\nFloor=Grass\n\n\
[GRASS]\nGrass=0\n\n[TERRAIN TYPES]\nCount=4\n\n[TERRAIN0]\nName=Grass\nStrRef=-1\n\n[TERRAIN1]\nName=Rock\nStrRef=-1\n\n[TERRAIN3]\nName=Chasm\nStrRef=-1\n\n\
[CROSSER TYPES]\nCount=0\n\n[PRIMARY RULES]\nCount=0\n\n[SECONDARY RULES]\nCount=0\n\n",
    );
    s += &format!("[TILES]\nCount={}\n\n", TILE_MODELS.len());
    for (i, m) in TILE_MODELS.iter().enumerate() {
        let corner = if i == CHASM_TILE { "Chasm" } else { "Grass" };
        s += &format!("[TILE{i}]\nModel={m}\n{}\n", tile_fields(corner));
    }
    s += "[GROUPS]\nCount=0\n";
    s.replace('\n', "\r\n").into_bytes()
}

/// Everything in `fx_tiles.hak`.
pub fn tiles_hak() -> Vec<(String, ResType, Vec<u8>)> {
    let wall_quad = |z0: f32, z1: f32| vec![[-1.0, 0.0, z0], [1.0, 0.0, z0], [1.0, 0.0, z1], [-1.0, 0.0, z1]];
    let cube: Vec<[f32; 3]> = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    let cube_faces = [[4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]];
    let marker: Vec<[f32; 3]> = vec![[-0.5, -0.5, MARKER_UP], [0.5, -0.5, MARKER_UP], [0.5, 0.5, MARKER_UP], [-0.5, 0.5, MARKER_UP]];
    let f = |n: &str, t: ResType, d: Vec<u8>| (n.to_string(), t, d);
    vec![
        f("fx01", ResType::SET, tileset()),
        f(
            "fx01_a01",
            ResType::MDL,
            model(
                "fx01_a01",
                "TILE",
                &[
                    trimesh("ground", "fx01_a01", "fx_grass", &square(0.0), &QUAD),
                    trimesh("rock", "fx01_a01", "fx_rock", &ROCK, &[[0, 1, 2]]),
                    trimesh("pond", "fx01_a01", "fx_water", &POND, &[[0, 1, 2]]),
                ],
            ),
        ),
        // Grass walks; the pond's bed is deep water (below the grass); the rock's foot does not walk.
        f(
            "fx01_a01",
            ResType::WOK,
            walkmesh(
                "fx01_a01",
                &[walk_square(0.0, WALK), vec![(POND.map(|v| [v[0], v[1], -1.5]), DEEP_WATER), (ROCK.map(|v| [v[0], v[1], 0.1]), NO_WALK)]]
                    .concat(),
            ),
        ),
        f("fx01_f01", ResType::MDL, pit_model()),
        // The grass around the pit walks; its floor does not (as a chasm's floor in the game).
        f("fx01_f01", ResType::WOK, walkmesh("fx01_f01", &pit_walk(NO_WALK))),
        f("fx01_b01", ResType::MDL, model("fx01_b01", "TILE", &[trimesh("void", "fx01_b01", "fx_black", &square(0.0), &QUAD)])),
        f("fx01_b01", ResType::WOK, walkmesh("fx01_b01", &walk_square(0.0, WALK))),
        f("fx01_c01", ResType::MDL, model("fx01_c01", "TILE", &[trimesh("ground", "fx01_c01", "fx_grass", &square(0.0), &QUAD)])),
        f("fx01_c01", ResType::WOK, walkmesh("fx01_c01", &walk_square(0.0, WALK))),
        f("fx01_d01", ResType::MDL, model("fx01_d01", "TILE", &[trimesh("top", "fx01_d01", "fx_stone", &square(6.0), &QUAD)])),
        f("fx01_d01", ResType::WOK, walkmesh("fx01_d01", &walk_square(6.0, WALK))),
        f(
            "fx01_e01",
            ResType::MDL,
            model(
                "fx01_e01",
                "TILE",
                &[
                    trimesh("floor", "fx01_e01", "fx_grass", &square(0.0), &QUAD),
                    trimesh("ceiling", "fx01_e01", "fx_stone", &square(5.0), &QUAD),
                ],
            ),
        ),
        f("fx01_e01", ResType::WOK, walkmesh("fx01_e01", &walk_square(0.0, WALK))),
        f("fx_box", ResType::MDL, model("fx_box", "CHARACTER", &[trimesh("box", "fx_box", "fx_crate", &cube, &cube_faces)])),
        f("fx_door", ResType::MDL, model("fx_door", "DOOR", &[trimesh("door", "fx_door", "fx_wood", &wall_quad(0.0, 3.0), &QUAD)])),
        f("fx_marker", ResType::MDL, model("fx_marker", "CHARACTER", &[trimesh("mark", "fx_marker", "fx_marker", &marker, &QUAD)])),
        f("fx_grass", ResType::TGA, checker(GRASS.0, GRASS.1)),
        f("fx_stone", ResType::TGA, checker(STONE.0, STONE.1)),
        f("fx_rock", ResType::MTR, b"// The rock is drawn with the stone image.\ntexture0 fx_stone\n".to_vec()),
        f("fx_water", ResType::TGA, plain([20, 60, 200])),
        f("fx_black", ResType::TGA, plain([0, 0, 0])),
        f("fx_pit", ResType::TGA, pit_texture()),
        f("fx_crate", ResType::TGA, plain(CRATE)),
        f("fx_wood", ResType::TGA, plain(WOOD)),
        // Blue here; red in fx_over, which the module lists first.
        f("fx_marker", ResType::TGA, plain([0, 0, 255])),
        f("surfacemat", ResType::TWODA, b"2DA V2.0\n\n   Label Walk\n0  NotDefined 0\n1  Grass 1\n2  Stone 0\n".to_vec()),
        f("placeables", ResType::TWODA, b"2DA V2.0\n\n   Label ModelName StrRef\n0  Box fx_box ****\n1  Marker fx_marker ****\n".to_vec()),
        f("genericdoors", ResType::TWODA, b"2DA V2.0\n\n   Label ModelName\n0  Door fx_door\n".to_vec()),
    ]
}

/// The pit's grass: four strips around the hole, as (corners of a quad).
fn pit_rim() -> Vec<[[f32; 3]; 4]> {
    let (o, h) = (5.0, PIT_HALF);
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32| [[x0, y0, 0.0], [x1, y0, 0.0], [x1, y1, 0.0], [x0, y1, 0.0]];
    vec![quad(-o, -o, o, -h), quad(-o, h, o, o), quad(-o, -h, -h, h), quad(h, -h, o, h)]
}

/// Tile 5's model (and 6's): grass around a pit, its stone walls, and its floor ([`PIT_FLOOR`]) drawn with `fx_pit`.
pub fn pit_model() -> Vec<u8> {
    let (h, z) = (PIT_HALF, PIT_FLOOR);
    let quads = |qs: &[[[f32; 3]; 4]]| -> (Vec<[f32; 3]>, Vec<[usize; 3]>) {
        let verts = qs.iter().flatten().copied().collect();
        let faces = (0..qs.len()).flat_map(|k| QUAD.map(|f| f.map(|v| v + k * 4))).collect();
        (verts, faces)
    };
    let (rim, rim_faces) = quads(&pit_rim());
    let c = [[-h, -h], [h, -h], [h, h], [-h, h]];
    let walls: Vec<[[f32; 3]; 4]> = (0..4)
        .map(|k| {
            let (a, b) = (c[k], c[(k + 1) % 4]);
            [[a[0], a[1], z], [b[0], b[1], z], [b[0], b[1], 0.0], [a[0], a[1], 0.0]]
        })
        .collect();
    let (wall, wall_faces) = quads(&walls);
    let floor: Vec<[f32; 3]> = c.iter().map(|p| [p[0], p[1], z]).collect();
    model(
        "fx01_f01",
        "TILE",
        &[
            trimesh("rim", "fx01_f01", "fx_grass", &rim, &rim_faces),
            trimesh("walls", "fx01_f01", "fx_stone", &wall, &wall_faces),
            trimesh("floor", "fx01_f01", "fx_pit", &floor, &QUAD),
        ],
    )
}

/// The pit's walkmesh: its grass walks, its floor is of `floor` (a surface material).
pub fn pit_walk(floor: u32) -> Vec<([[f32; 3]; 3], u32)> {
    let mut faces: Vec<([[f32; 3]; 3], u32)> = pit_rim().iter().flat_map(|q| QUAD.map(|f| (f.map(|v| q[v]), WALK))).collect();
    let (h, z) = (PIT_HALF, PIT_FLOOR);
    let s = [[-h, -h, z], [h, -h, z], [h, h, z], [-h, h, z]];
    faces.extend(QUAD.map(|f| (f.map(|v| s[v]), floor)));
    faces
}

/// The pit's texture, as the underdark's pit textures are: black where the floor is (its middle, UVs 0.25 to 0.75), grey at its
/// border, so it is not black void.
pub fn pit_texture() -> Vec<u8> {
    tga(16, 16, |x, y| if x == 0 || y == 0 || x == 15 || y == 15 { [110, 110, 110, 255] } else { [0, 0, 0, 255] })
}

/// Everything in `fx_over.hak`.
pub fn over_hak() -> Vec<(String, ResType, Vec<u8>)> {
    vec![("fx_marker".into(), ResType::TGA, plain([255, 0, 0]))]
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The module.

fn text(s: &str) -> Value {
    Value::String(s.as_bytes().to_vec())
}

fn resref(s: &str) -> Value {
    Value::ResRef(s.as_bytes().to_vec())
}

/// A struct with its fields in nasher's order (labels sorted, ignoring case), so the binary a nasher project is packed into and
/// the JSON read back agree byte for byte.
pub fn item(id: u32, fields: &[(&str, Value)]) -> Struct {
    let mut fields = fields.to_vec();
    fields.sort_by_key(|(label, _)| label.to_ascii_lowercase());
    let mut s = Struct::new(id);
    for (label, v) in fields {
        s.set(label, v);
    }
    s
}

fn gff(file_type: &[u8; 4], fields: &[(&str, Value)]) -> Gff {
    let mut g = Gff::new(*file_type);
    g.root = item(u32::MAX, fields);
    g
}

/// `module.ifo`: the haks, first highest, and the areas.
pub fn ifo() -> Gff {
    let hak = |n: &str| item(8, &[("Mod_Hak", text(n))]);
    let area = |n: &str| item(6, &[("Area_Name", resref(n))]);
    gff(
        b"IFO ",
        &[
            ("Mod_HakList", Value::List(HAKS.iter().map(|h| hak(h)).collect())),
            ("Mod_Area_list", Value::List(AREAS.iter().map(|a| area(a)).collect())),
            ("Mod_Name", Value::LocString(LocString::from_text(Language::ENGLISH, Gender::Male, "Fixture".to_string()))),
        ],
    )
}

/// A tile of an area: (tileset tile, quarter turns, height steps).
pub type TileAt = (i32, i32, i32);

/// The tiles of each area, row by row from the south-west, and its size and flags.
pub fn layout(area: &str) -> (i32, i32, u32, Vec<TileAt>) {
    match area {
        "fx_main" => (3, 2, 4, vec![(0, 0, 0), (0, 1, 0), (0, 2, 0), (1, 0, 0), (0, 3, 1), (2, 0, 0)]),
        "fx_mark" => (3, 2, 4, vec![(2, 0, 0); 6]),
        "fx_flat" => (2, 2, 1, vec![(4, 0, 0); 4]),
        "fx_cave" => (3, 2, 2, vec![(2, 0, 0), (3, 0, 0), (2, 0, 0), (3, 0, 0), (3, 0, 0), (3, 0, 0)]),
        "fx_step" => (2, 2, 1, vec![(1, 0, 0), (1, 0, 0), (2, 0, 2), (2, 0, 2)]),
        "fx_chasm" => (3, 1, 4, vec![(2, 0, 0), (CHASM_TILE as i32, 0, 0), (PIT_TILE as i32, 0, 0)]),
        _ => panic!("no fixture area {area}"),
    }
}

/// Area `name`'s `.are`.
pub fn are(name: &str) -> Gff {
    let (w, h, flags, tiles) = layout(name);
    let tile =
        |&(id, o, z): &TileAt| item(1, &[("Tile_ID", Value::Int(id)), ("Tile_Orientation", Value::Int(o)), ("Tile_Height", Value::Int(z))]);
    gff(
        b"ARE ",
        &[
            ("Tag", text(name)),
            ("Name", Value::LocString(LocString::from_text(Language::ENGLISH, Gender::Male, format!("Fixture {name}")))),
            ("Tileset", resref("fx01")),
            ("Width", Value::Int(w)),
            ("Height", Value::Int(h)),
            ("Flags", Value::Dword(flags)),
            ("SunAmbientColor", Value::Dword(0x00332211)),
            ("SunDiffuseColor", Value::Dword(0x00ccbbaa)),
            ("Tile_List", Value::List(tiles.iter().map(tile).collect())),
        ],
    )
}

/// A placeable of placeables.2da row `row` at `at`, turned `bearing` radians.
pub fn placeable(row: u32, at: [f32; 3], bearing: f32) -> Struct {
    item(
        9,
        &[
            ("Tag", text("fx_plc")),
            ("Appearance", Value::Dword(row)),
            ("X", Value::Float(at[0])),
            ("Y", Value::Float(at[1])),
            ("Z", Value::Float(at[2])),
            ("Bearing", Value::Float(bearing)),
        ],
    )
}

/// Area `name`'s `.git` (`fx_flat` has none, as an area may).
pub fn git(name: &str) -> Option<Gff> {
    let (placeables, doors) = match name {
        "fx_main" => {
            let door = item(
                8,
                &[
                    ("Tag", text("fx_door")),
                    ("Appearance", Value::Dword(0)),
                    ("GenericType", Value::Byte(0)),
                    ("X", Value::Float(25.0)),
                    ("Y", Value::Float(15.0)),
                    ("Z", Value::Float(0.0)),
                    ("Bearing", Value::Float(0.0)),
                ],
            );
            // (A quarter turn as nasher keeps it, to four decimals, so every module form has the same float.)
            #[expect(clippy::approx_constant, reason = "a quarter turn to four decimals, as nasher keeps it")]
            let quarter = 1.5708;
            (vec![placeable(0, [12.0, 3.0, 0.0], quarter)], vec![door])
        }
        "fx_mark" => (vec![placeable(1, MARKER, 0.0)], Vec::new()),
        "fx_flat" => return None,
        _ => (Vec::new(), Vec::new()),
    };
    Some(gff(b"GIT ", &[("Placeable List", Value::List(placeables)), ("Door List", Value::List(doors))]))
}

/// The module's files: `module.ifo`, then each area's `.are` and `.git`.
pub fn module_files() -> Vec<(String, ResType, Gff)> {
    let mut files = vec![("module".to_string(), ResType::IFO, ifo())];
    for a in AREAS {
        files.push((a.to_string(), ResType::ARE, are(a)));
        if let Some(g) = git(a) {
            files.push((a.to_string(), ResType::GIT, g));
        }
    }
    files
}

/// nwn_gff's JSON as nasher keeps it (keys sorted, floats rounded to four decimals).
pub fn json(g: &Gff) -> String {
    mg_gff::to_json_text(&mg_gff::to_json(g, mg_core::Codepage::WINDOWS_1252).unwrap(), mg_gff::TextStyle::NASHER)
}

fn write_module(base: &Path) {
    let (folder, json_dir, project) = (base.join("folder"), base.join("json"), base.join("project"));
    for d in [folder.clone(), json_dir.join("areas"), project.join("src/areas")] {
        mkdir(&d);
    }
    let mut erf = mg_erf::ErfWriter::new(*b"MOD ");
    for (name, t, g) in module_files() {
        let bytes = g.to_bytes().unwrap();
        let ext = t.extension().unwrap();
        erf.add(ResRef::from_str(&name).unwrap(), t, bytes.clone()).unwrap();
        std::fs::write(folder.join(format!("{name}.{ext}")), &bytes).unwrap();
        let (j, p) =
            if t == ResType::IFO { (json_dir.clone(), project.clone()) } else { (json_dir.join("areas"), project.join("src/areas")) };
        std::fs::write(j.join(format!("{name}.{ext}.json")), json(&g)).unwrap();
        std::fs::write(p.join(format!("{name}.{ext}.json")), json(&g)).unwrap();
    }
    std::fs::write(base.join("fx.mod"), erf.to_bytes().unwrap()).unwrap();
    std::fs::write(
        project.join("nasher.cfg"),
        "[package]\nname = \"fixture\"\n\n[sources]\ninclude = \"src/**/*.json\"\ninclude = \"module.ifo.json\"\n\n[target]\nname = \"default\"\nfile = \"fx.mod\"\n",
    )
    .unwrap();
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Reading pictures back.

/// The centroid of the red in an RGBA picture (rows from the top), weighted by how red each pixel is and how much of it is drawn;
/// None when there is no red. The fixture's ground has no red, so this is where the marker is drawn.
pub fn red_centroid(width: u32, height: u32, rgba: &[u8]) -> Option<[f32; 2]> {
    let (mut sx, mut sy, mut sw) = (0.0f64, 0.0f64, 0.0f64);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let px = &rgba[(y * width as usize + x) * 4..][..4];
            let w = f64::from(px[0]) * f64::from(px[3]) / 255.0;
            sx += (x as f64 + 0.5) * w;
            sy += (y as f64 + 0.5) * w;
            sw += w;
        }
    }
    (sw > 0.0).then(|| [(sx / sw) as f32, (sy / sw) as f32])
}

/// Where a 2 by 4 matrix (a manifest's `matrix`) puts game point `p`.
pub fn project(m: &[[f32; 4]; 2], p: [f32; 3]) -> [f32; 2] {
    [m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3], m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3]]
}

// ---------------------------------------------------------------------------------------------------------------------------------
// A creature and its items (not written by `write`: a test that wants them adds `kit_hak` to the user folder).

/// The creature kit's hak: appearance.2da row 0, a full-body (`F`) creature whose model is `fx_body`; baseitems.2da row 0, a
/// one-piece item kind `fxrod` drawn as `fxrod_<ModelPart1>` (`fxrod_001`, `fxrod_002`); and the game's own item blueprint
/// [`GAME_ROD`] (`fxrod_002`).
pub fn kit_hak() -> Vec<(String, ResType, Vec<u8>)> {
    let f = |n: &str, t: ResType, d: Vec<u8>| (n.to_string(), t, d);
    let mesh = |name: &str| trimesh(&format!("{name}_m"), name, "fx_marker", &square(0.0), &QUAD);
    vec![
        f("appearance", ResType::TWODA, b"2DA V2.0\n\n   LABEL RACE MODELTYPE WEAPONSCALE\n0  FxBody fx_body F 1\n".to_vec()),
        f("baseitems", ResType::TWODA, b"2DA V2.0\n\n   label ItemClass ModelType WeaponWield\n0  FxRod fxrod 0 ****\n".to_vec()),
        f("fx_body", ResType::MDL, model("fx_body", "character", &[mesh("fx_body")])),
        f("fxrod_001", ResType::MDL, model("fxrod_001", "item", &[mesh("fxrod_001")])),
        f("fxrod_002", ResType::MDL, model("fxrod_002", "item", &[mesh("fxrod_002")])),
        f(GAME_ROD, ResType::UTI, rod(GAME_ROD, 2).to_bytes().unwrap()),
    ]
}

/// The item blueprint the kit's hak has.
pub const GAME_ROD: &str = "fx_gamerod";

/// An item blueprint `resref` of the kit's item kind, drawn as `fxrod_<part>`.
pub fn rod(resref: &str, part: u8) -> Gff {
    gff(b"UTI ", &[("BaseItem", Value::Int(0)), ("ModelPart1", Value::Byte(part)), ("TemplateResRef", self::resref(resref))])
}

/// A creature blueprint of the kit's appearance wearing `equipped`: (`Equip_ItemList` slot id, item resref) each.
pub fn creature(equipped: &[(u32, &str)]) -> Gff {
    let worn = equipped.iter().map(|&(slot, r)| item(slot, &[("EquippedRes", resref(r))])).collect();
    gff(b"UTC ", &[("Appearance_Type", Value::Word(0)), ("Equip_ItemList", Value::List(worn))])
}
