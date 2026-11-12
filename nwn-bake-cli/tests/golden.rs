// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden tests on a real NWN:EE install: a few areas of the base game's own campaign modules drawn in all five views, and a few of
//! its creature blueprints described part by part, compared with fingerprints kept in `tests/golden/` (no pictures, no game files).
//!
//! They run only when `NWN_BAKE_GAME` names the install (the folder with `data/` and `lang/`); otherwise each says it is skipped and
//! passes, as in public CI. `NWN_BAKE_HOME` may name a user folder; by default an empty one is used, so nothing in a user's
//! `override/` changes the result. The modules and blueprints are the base game's, so no hak is needed.
//!
//! A picture's fingerprint is its width and height and a 32 by 18 grid of the mean colour (RGBA) of each cell, compared with a
//! tolerance (float results may differ slightly between platforms and compilers): sizes within a pixel, each cell's channels within
//! [`CELL`], and all of them within [`MEAN`] on average. The scene's counts and the roof cut (to the centimetre) are compared
//! exactly.
//!
//! When a change to the pictures is intended (a new look, a fix), write the fingerprints again and commit them with the change:
//!
//! ```text
//! NWN_BAKE_GAME=/path/to/nwn NWN_BAKE_GOLDEN=write cargo test -p nwn-bake-cli --test golden
//! ```

use std::path::{Path, PathBuf};

use nwn_bake::area::SceneOptions;
use nwn_bake::game::{self, Game, GameOptions, Haks, Tlk};
use nwn_bake::iso::{self, IsoOptions, View};
use nwn_bake::module::ModuleSource;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A cell's channel may differ by this much (0..255).
const CELL: f32 = 8.0;
/// All cells' channels may differ by this much on average.
const MEAN: f32 = 0.5;
/// The grid.
const COLUMNS: usize = 32;
const ROWS: usize = 18;

const VIEWS: [View; 5] = [View::Sw, View::Se, View::Ne, View::Nw, View::Top];

/// The install, or None (and a note on stderr) when `NWN_BAKE_GAME` is not set.
fn install(test: &str) -> Option<PathBuf> {
    match std::env::var_os("NWN_BAKE_GAME").filter(|v| !v.is_empty()) {
        Some(root) => Some(PathBuf::from(root)),
        None => {
            eprintln!("{test}: skipped (set NWN_BAKE_GAME to an NWN:EE install to run the golden tests)");
            None
        }
    }
}

fn writing() -> bool {
    std::env::var("NWN_BAKE_GOLDEN").is_ok_and(|v| v == "write")
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// The game, with no haks and no custom talk table; the user folder `NWN_BAKE_HOME`, else an empty one.
fn open(root: &Path, tag: &str) -> Game {
    let user = match std::env::var_os("NWN_BAKE_HOME").filter(|v| !v.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => {
            let empty = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("nwn-bake-golden-home-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&empty).unwrap();
            empty
        }
    };
    game::open(&GameOptions { root: Some(root.to_path_buf()), user: Some(user), haks: Haks::None, tlk: Tlk::None }).unwrap()
}

/// The install's data build (the first line of `databuild.txt`), for the record.
fn build(root: &Path) -> String {
    std::fs::read_to_string(root.join("databuild.txt"))
        .ok()
        .and_then(|s| s.lines().next().map(str::trim).map(String::from))
        .unwrap_or_default()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct ViewPrint {
    view: View,
    width: u32,
    height: u32,
    /// `ROWS` rows from the top, each `COLUMNS` cells of RGBA as hex.
    grid: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct AreaGolden {
    /// The data build the fingerprints were recorded on.
    build: String,
    module: String,
    area: String,
    /// The scene's counts, its kind and where the roof is cut.
    scene: Value,
    views: Vec<ViewPrint>,
}

/// A picture's fingerprint.
fn fingerprint(view: View, width: u32, height: u32, rgba: &[u8]) -> ViewPrint {
    let (w, h) = (width as usize, height as usize);
    let mut grid = Vec::with_capacity(ROWS);
    for row in 0..ROWS {
        let (y0, y1) = (row * h / ROWS, ((row + 1) * h / ROWS).max(row * h / ROWS + 1).min(h));
        let mut line = String::with_capacity(COLUMNS * 8);
        for col in 0..COLUMNS {
            let (x0, x1) = (col * w / COLUMNS, ((col + 1) * w / COLUMNS).max(col * w / COLUMNS + 1).min(w));
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    for k in 0..4 {
                        sum[k] += u64::from(rgba[(y * w + x) * 4 + k]);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as u64;
            for s in sum {
                line += &format!("{:02x}", (s + n / 2) / n);
            }
        }
        grid.push(line);
    }
    ViewPrint { view, width, height, grid }
}

fn cells(p: &ViewPrint) -> Vec<f32> {
    p.grid.iter().flat_map(|l| (0..l.len() / 2).map(move |i| f32::from(u8::from_str_radix(&l[i * 2..i * 2 + 2], 16).unwrap()))).collect()
}

/// What differs between two fingerprints beyond the tolerance, if anything.
fn compare(want: &ViewPrint, got: &ViewPrint) -> Option<String> {
    if want.width.abs_diff(got.width) > 1 || want.height.abs_diff(got.height) > 1 {
        return Some(format!("{:?}: {}x{}, was {}x{}", got.view, got.width, got.height, want.width, want.height));
    }
    let (a, b) = (cells(want), cells(got));
    let diffs: Vec<f32> = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).collect();
    let (worst, at) = diffs.iter().enumerate().fold((0.0f32, 0), |m, (i, &d)| if d > m.0 { (d, i) } else { m });
    let mean = diffs.iter().sum::<f32>() / diffs.len().max(1) as f32;
    (worst > CELL || mean > MEAN).then(|| {
        let cell = at / 4;
        format!(
            "{:?}: the colours differ by {mean:.2} on average (at most {MEAN}), {worst} at most (at most {CELL}; cell {}, {} of row {}, channel {})",
            got.view, cell, cell % COLUMNS, cell / COLUMNS, at % 4
        )
    })
}

/// Draws `area` of the base module `module` (a path under `data/`) in every view, and compares or writes its fingerprints.
fn area_golden(test: &str, module: &str, area: &str) {
    let Some(root) = install(test) else { return };
    let game = open(&root, area);
    let source = ModuleSource::open(&root.join("data").join(module)).unwrap_or_else(|e| panic!("{module}: {e:#}"));
    let files = source.area(area).unwrap();
    let scene = nwn_bake::area::scene(&game, &files, &SceneOptions::default()).unwrap();
    let rendered = iso::render(&scene, &IsoOptions { views: VIEWS.to_vec(), ..Default::default() }).unwrap();
    let c = &scene.counts;
    let summary = serde_json::json!({
        "kind": scene.kind(), "tiles": scene.tiles, "tile_models": c.tile_models, "tile_triangles": c.tile_triangles,
        "placeables": c.placeables, "doors": c.doors, "object_triangles": c.object_triangles, "water_triangles": c.water_triangles,
        "stray": c.stray, "textures": c.textures, "textures_missing": c.textures_missing,
        // (To the centimetre: the cut over a cave's highest floor is a percentile of floats.)
        "roof_cut_m": rendered.roof_cut.map(|r| (r.at * 100.0).round() / 100.0), "cave_dug_out_of_rock": rendered.roof_cut.map(|r| r.slab),
    });
    let views: Vec<ViewPrint> = rendered.pictures.iter().map(|p| fingerprint(p.view, p.width, p.height, &p.rgba)).collect();
    let path = golden_dir().join(format!("area-{area}.json"));
    let got = AreaGolden { build: build(&root), module: module.to_string(), area: area.to_string(), scene: summary, views };
    if writing() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        eprintln!("{test}: wrote {}", path.display());
        return;
    }
    let want: AreaGolden = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (write it with NWN_BAKE_GOLDEN=write)", path.display())),
    )
    .unwrap();
    let mut problems = Vec::new();
    if want.scene != got.scene {
        problems.push(format!("the scene: {}, was {}", got.scene, want.scene));
    }
    for (w, g) in want.views.iter().zip(&got.views) {
        problems.extend(compare(w, g));
    }
    if want.views.len() != got.views.len() {
        problems.push(format!("{} views, was {}", got.views.len(), want.views.len()));
    }
    let builds = if want.build != got.build {
        format!(" (recorded on data build {}, this install is {})", want.build, got.build)
    } else {
        String::new()
    };
    assert!(
        problems.is_empty(),
        "{module} {area} is not drawn as recorded{builds}:\n  {}\nIf the change is intended, write the fingerprints again: NWN_BAKE_GAME=... NWN_BAKE_GOLDEN=write cargo test -p nwn-bake-cli --test golden",
        problems.join("\n  ")
    );
}

/// A small interior, mostly black void (left out of the picture), cut 3.5 m over its floor.
#[test]
fn golden_area_prelude_m0q1c() {
    area_golden("golden_area_prelude_m0q1c", "nwm/Prelude.nwm", "m0q1c");
}

/// An interior that is a cave dug out of rock: cut 2 m over its highest floor; water.
#[test]
fn golden_area_chapter1_map_m1q4e() {
    area_golden("golden_area_chapter1_map_m1q4e", "nwm/Chapter1.nwm", "map_m1q4e");
}

/// A large surface area (16 by 16 tiles): hills, water, many placeables and doors; the roof kept.
#[test]
fn golden_area_xp1_q1rural() {
    area_golden("golden_area_xp1_q1rural", "nwm/XP1-Chapter 1.nwm", "q1rural");
}

/// Caves (16 by 16 tiles) with water and many placeables, cut over the floor.
#[test]
fn golden_area_xp1_koboldcaves() {
    area_golden("golden_area_xp1_koboldcaves", "nwm/XP1-Chapter 1.nwm", "koboldcaves");
}

/// The creatures: base game blueprints, four made of parts (a half-fiend, a tiefling, a drow, a mounted dwarf) and a dragon of one
/// model.
const CREATURES: [&str; 5] = ["nw_halffnd001", "nw_tiefling02", "x2_mephdrow006", "x3_dwarfmount001", "nw_drgred001"];

/// `v` without the parts' ids (hashes of how the parts are made, which change with the code, not with what is drawn).
fn without_ids(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("id");
            m.values_mut().for_each(without_ids);
        }
        Value::Array(a) => a.iter_mut().for_each(without_ids),
        _ => {}
    }
}

/// Each creature blueprint described part by part, wearing its items from the game (`nwn_bake::look::describe`: models, where they
/// attach, their textures and colours; on the CPU, no GPU needed), compared exactly with what was recorded.
#[test]
fn golden_creatures_described() {
    let test = "golden_creatures_described";
    let Some(root) = install(test) else { return };
    let game = open(&root, "creatures");
    let mut got = serde_json::Map::new();
    for resref in CREATURES {
        let bytes = game.resman.get_named(resref, mg_core::ResType::UTC).unwrap_or_else(|_| panic!("{resref}.utc is not in the game"));
        let look = nwn_bake::look::describe(&game, &bytes, &nwn_bake::items::Items::game(), "pause1", 0.0).unwrap();
        let mut v = serde_json::to_value(&look).unwrap();
        without_ids(&mut v);
        got.insert(resref.to_string(), v);
    }
    let got = serde_json::json!({ "build": build(&root), "creatures": got });
    let path = golden_dir().join("creatures.json");
    if writing() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        eprintln!("{test}: wrote {}", path.display());
        return;
    }
    let want: Value = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (write it with NWN_BAKE_GOLDEN=write)", path.display())),
    )
    .unwrap();
    for resref in CREATURES {
        assert_eq!(
            got["creatures"][resref], want["creatures"][resref],
            "{resref} is not described as recorded (data build {} recorded, {} here). If the change is intended: NWN_BAKE_GAME=... NWN_BAKE_GOLDEN=write cargo test -p nwn-bake-cli --test golden",
            want["build"], got["build"]
        );
    }
}
