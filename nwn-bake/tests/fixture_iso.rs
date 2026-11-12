// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The renderer on the generated fixture (`support/fixture.rs`; no game content): the transform in all five views (a red marker at
//! known game points is drawn where the matrix puts it), black void left out of the picture but not its bounds, the roof cut seen
//! in the picture, a chasm's floor drawn in its colours and a black floor elsewhere left black, the same pixels every time, and the manifest checked against its schema (`docs/area-iso.schema.json`).

#[path = "support/fixture.rs"]
mod fixture;

use std::sync::Arc;

use mg_gff::Value;
use nwn_bake::area::{Scene, SceneOptions, SceneTexture};
use nwn_bake::game::{self, Game, GameOptions, Haks, Tlk};
use nwn_bake::iso::{self, Background, Inputs, IsoOptions, Look, OutOptions, Picture, Roof, View};
use nwn_bake::module::ModuleSource;
use nwn_bake::texture::Mipped;

const ALL: [View; 5] = [View::Sw, View::Se, View::Ne, View::Nw, View::Top];

fn open(f: &fixture::Fixture) -> Game {
    game::open(&GameOptions {
        root: Some(f.root.clone()),
        user: Some(f.user.clone()),
        haks: Haks::List(fixture::HAKS.map(String::from).to_vec()),
        tlk: Tlk::None,
    })
    .unwrap()
}

fn scene_of(g: &Game, f: &fixture::Fixture, area: &str) -> Scene {
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    nwn_bake::area::scene(g, &source.area(area).unwrap(), &SceneOptions::default()).unwrap()
}

/// The pixel at game point (x, y, z) of a picture (RGBA).
fn pixel(p: &Picture, at: [f32; 3]) -> [u8; 4] {
    let q = p.transform.project(at);
    let (i, j) = (q[0].floor() as usize, q[1].floor() as usize);
    p.rgba[(j * p.width as usize + i) * 4..][..4].try_into().unwrap()
}

#[test]
fn the_matrix_puts_the_marker_where_it_is_drawn_in_all_five_views() {
    let f = fixture::write(&fixture::dir("transform"));
    let g = open(&f);
    let source = ModuleSource::open(&f.project()).unwrap();
    // The marker moved about the area and up, drawn with the default options (shadows, exposure) and at another scale and quality.
    // (Past the first place, with a smaller shadow map: the default's 4,096 cells a side are most of the time of a debug build.)
    let small = Look { shadow_size: 1024, ..Look::default() };
    let options = |first: bool| {
        let look = if first { Look::default() } else { small.clone() };
        [
            IsoOptions { views: ALL.to_vec(), look: look.clone(), ..Default::default() },
            IsoOptions { views: ALL.to_vec(), scale: 5.0, quality: 3, look, ..Default::default() },
        ]
    };
    for (n, at) in [fixture::MARKER, [3.0, 4.0, 0.0], [26.5, 2.5, 3.0], [4.0, 16.0, 6.0], [21.0, 17.5, 1.5]].into_iter().enumerate() {
        let mut area = source.area("fx_mark").unwrap();
        match area.git.as_mut().unwrap().root.get_mut("Placeable List") {
            Some(Value::List(l)) => {
                l[0].set("X", Value::Float(at[0]));
                l[0].set("Y", Value::Float(at[1]));
                l[0].set("Z", Value::Float(at[2]));
            }
            _ => unreachable!(),
        }
        let scene = nwn_bake::area::scene(&g, &area, &SceneOptions::default()).unwrap();
        let centre = [at[0], at[1], at[2] + fixture::MARKER_UP];
        for o in &options(n == 0) {
            for p in iso::render(&scene, o).unwrap().pictures {
                let want = p.transform.project(centre);
                let got = fixture::red_centroid(p.width, p.height, &p.rgba)
                    .unwrap_or_else(|| panic!("{:?}: the marker at {at:?} is not drawn", p.view));
                let off = ((got[0] - want[0]).powi(2) + (got[1] - want[1]).powi(2)).sqrt();
                assert!(
                    off < 0.5,
                    "{:?} at scale {}: the marker at {at:?} is drawn at {got:?}, the matrix says {want:?} ({off:.2} px off)",
                    p.view,
                    o.scale
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_main_area_s_pictures_show_its_parts() {
    let f = fixture::write(&fixture::dir("main"));
    let g = open(&f);
    let s = scene_of(&g, &f, "fx_main");
    let o = IsoOptions { views: vec![View::Top], shadows: false, exposure: false, ..Default::default() };
    let r = iso::render(&s, &o).unwrap();
    assert!(r.roof_cut.is_none() && r.notes.is_empty(), "{:?}", r.notes);
    let top = &r.pictures[0];
    // Grass is green with no red; the box is drawn with its own texture; the pond with the water's colour over it.
    let grass = pixel(top, [1.25, 7.5, 0.0]);
    assert!(grass[0] == 0 && grass[1] > 60 && grass[3] == 255, "{grass:?}");
    let crate_top = pixel(top, [11.5, 3.5, 1.0]);
    assert!(crate_top[0] == 0 && crate_top[2] > crate_top[1] && crate_top[3] == 255, "the box: {crate_top:?}");
    let pond = pixel(top, [2.8, 1.8, 0.1]);
    assert!(pond[0] > 0 && pond[3] == 255, "the pond is tinted by the water: {pond:?}");
    // The stone of the rock, through its material.
    let rock = pixel(top, [8.75, 8.6, 2.0]);
    assert!(rock[0] == 0 && rock[2] > 60, "the rock: {rock:?}");
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn black_void_is_left_out_of_the_picture_but_not_its_bounds() {
    let f = fixture::write(&fixture::dir("void"));
    let g = open(&f);
    let s = scene_of(&g, &f, "fx_main");
    let o = IsoOptions { views: ALL.to_vec(), shadows: false, ..Default::default() };
    let r = iso::render(&s, &o).unwrap();
    // The void tile (south-west of the second row) shows the transparent background; the grass beside it is drawn.
    let top = &r.pictures[4];
    assert_eq!(pixel(top, [5.0, 15.0, 0.0])[3], 0, "the void is not drawn");
    assert_eq!(pixel(top, [5.0, 5.0, 0.0])[3], 255);
    // Drawn grey instead, the void would change nothing of the pictures' sizes and transforms.
    let mut grey = s.clone();
    let image = mg_image::Rgba { width: 1, height: 1, data: vec![90, 90, 90, 255] };
    let k = grey.textures.iter().position(|t| t.name == "fx_black").unwrap();
    grey.textures[k] = SceneTexture { name: "fx_black".into(), image: Some(Arc::new(Mipped::new("fx_black".into(), &image))) };
    let d = iso::render(&grey, &o).unwrap();
    for (a, b) in r.pictures.iter().zip(&d.pictures) {
        assert_eq!((a.width, a.height, &a.transform), (b.width, b.height, &b.transform), "{:?}", a.view);
    }
    assert_eq!(pixel(&d.pictures[4], [5.0, 15.0, 0.0])[3], 255, "the grey is drawn");
    // On a coloured background the void shows that colour.
    let r = iso::render(&s, &IsoOptions { views: vec![View::Top], background: Background::Colour([255, 0, 255]), ..o.clone() }).unwrap();
    assert_eq!(pixel(&r.pictures[0], [5.0, 15.0, 0.0]), [255, 0, 255, 255]);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn a_chasm_s_floor_is_drawn_as_a_dark_gradient_and_a_black_floor_elsewhere_stays_black() {
    let f = fixture::write(&fixture::dir("chasm"));
    let g = open(&f);
    let s = scene_of(&g, &f, "fx_chasm");
    let o = IsoOptions { views: vec![View::Top], shadows: false, exposure: false, ..Default::default() };
    let r = iso::render(&s, &o).unwrap();
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    let top = &r.pictures[0];
    let z = fixture::PIT_FLOOR;
    // The chasm (the second tile, its middle at (15, 5)): dark blue-grey, not black, and darker in its middle than near its edge.
    let (middle, near_edge) = (pixel(top, [15.0, 5.0, z]), pixel(top, [13.25, 5.0, z]));
    for c in [middle, near_edge] {
        assert!(
            c[3] == 255 && c[2] > c[0] && c[0] > 0 && (u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2])) < 150,
            "the chasm's floor: {c:?}"
        );
    }
    let sum = |c: [u8; 4]| u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2]);
    assert!(sum(middle) < sum(near_edge), "darker in the middle: {middle:?}, near the edge {near_edge:?}");
    // The same floor on the third tile, whose terrain is no chasm: drawn as its texture is, black.
    let black = pixel(top, [25.0, 5.0, z]);
    assert!(black[3] == 255 && sum(black) <= 6, "the black floor: {black:?}");
    // Drawn as a texture would be: the rest of the picture is as it is with the chasm's floor drawn black.
    let mut plain = s.clone();
    plain.triangles.iter_mut().for_each(|t| t.chasm = false);
    let p = iso::render(&plain, &o).unwrap();
    assert!(sum(pixel(&p.pictures[0], [15.0, 5.0, z])) <= 6);
    assert_eq!(pixel(&p.pictures[0], [5.0, 5.0, 0.0]), pixel(top, [5.0, 5.0, 0.0]), "the grass");
    assert_eq!(black, pixel(&p.pictures[0], [25.0, 5.0, z]));
    // The look's chasm colours are used: green both at the edge and deep, and no seam, makes the chasm green.
    let mut look = Look { chasm_edge: [0.0, 0.6, 0.0], chasm_deep: [0.0, 0.6, 0.0], chasm_reach: 4.0, chasm_seam: 0.0, ..Look::default() };
    let green = pixel(&iso::render(&s, &IsoOptions { look: look.clone(), ..o.clone() }).unwrap().pictures[0], [15.0, 5.0, z]);
    assert!(green[1] > 60 && green[1] > 3 * green[0].max(green[2]).max(1), "a green chasm: {green:?}");
    // A wide seam fades the whole floor up from black.
    look.chasm_seam = 50.0;
    let seam = pixel(&iso::render(&s, &IsoOptions { look, ..o.clone() }).unwrap().pictures[0], [15.0, 5.0, z]);
    assert!(seam[1] < green[1], "the seam darkens it: {seam:?} against {green:?}");
    // The other tile's black floor is the same whatever the look.
    assert_eq!(pixel(&iso::render(&s, &o).unwrap().pictures[0], [25.0, 5.0, z]), black);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_roof_cut_opens_the_rooms_and_keep_keeps_their_ceilings() {
    let f = fixture::write(&fixture::dir("roof"));
    let g = open(&f);
    let s = scene_of(&g, &f, "fx_flat");
    let o = IsoOptions { views: vec![View::Top], shadows: false, exposure: false, ..Default::default() };
    // Cut at 3.5 m: the floor's grass shows (green, its colour's blue channel lower than its green).
    let r = iso::render(&s, &o).unwrap();
    assert_eq!(r.roof_cut.map(|c| (c.at, c.slab)), Some((3.5, false)));
    let floor = pixel(&r.pictures[0], [10.0, 10.0, 0.0]);
    assert!(floor[1] > floor[2], "the floor: {floor:?}");
    // Kept: the stone ceiling (more blue than green).
    let r = iso::render(&s, &IsoOptions { roof: Roof::Keep, ..o.clone() }).unwrap();
    assert!(r.roof_cut.is_none());
    let ceiling = pixel(&r.pictures[0], [10.0, 10.0, 5.0]);
    assert!(ceiling[2] > ceiling[1], "the ceiling: {ceiling:?}");
    // The cave is cut over its floor.
    let r = iso::render(&scene_of(&g, &f, "fx_cave"), &o).unwrap();
    assert_eq!(r.roof_cut.map(|c| (c.at, c.slab)), Some((2.0, true)));
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn every_area_draws_the_same_pixels_twice() {
    let f = fixture::write(&fixture::dir("twice"));
    let (g1, g2) = (open(&f), open(&f));
    let o = IsoOptions { views: ALL.to_vec(), ..Default::default() };
    for area in fixture::AREAS {
        let a = iso::render(&scene_of(&g1, &f, area), &o).unwrap();
        let b = iso::render(&scene_of(&g2, &f, area), &o).unwrap();
        for (a, b) in a.pictures.iter().zip(&b.pictures) {
            assert!(a.rgba == b.rgba && a.transform == b.transform, "{area} {:?}", a.view);
            assert!(a.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255), "{area} {:?}: nothing drawn", a.view);
        }
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

/// The manifest's schema, compiled.
fn schema() -> jsonschema::Validator {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/area-iso.schema.json")).unwrap();
    jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap()
}

fn errors(v: &jsonschema::Validator, m: &serde_json::Value) -> Vec<String> {
    v.iter_errors(m).map(|e| format!("{} at {}", e, e.instance_path())).collect()
}

#[test]
fn every_manifest_written_validates_against_the_schema() {
    let f = fixture::write(&fixture::dir("schema"));
    let g = open(&f);
    let v = schema();
    let source = ModuleSource::open(&f.json_folder()).unwrap();
    let out = OutOptions::new(f.base.join("out"));
    let variants = [
        IsoOptions { views: ALL.to_vec(), ..Default::default() },
        IsoOptions {
            views: vec![View::Top],
            roof: Roof::Over(1.5),
            background: Background::Colour([1, 2, 3]),
            look: Look { light_up: 0.5, ..Look::default() },
            ..Default::default()
        },
        IsoOptions {
            views: vec![View::Ne],
            roof: Roof::Keep,
            light: iso::Light::Area,
            shadows: false,
            exposure: false,
            quality: 1,
            scale: 3.0,
            ..Default::default()
        },
    ];
    let mut checked = 0;
    for (i, o) in variants.iter().enumerate() {
        for area in fixture::AREAS {
            let files = source.area(area).unwrap();
            let scene = nwn_bake::area::scene(&g, &files, &SceneOptions { objects: i != 2, ..Default::default() }).unwrap();
            let rendered = iso::render(&scene, o).unwrap();
            let missing = if i == 1 { vec!["fx_gone".to_string()] } else { Vec::new() };
            let inputs = Inputs::of(&files, &fixture::HAKS.map(String::from), &missing, Some("fx_tlk")).unwrap();
            let name = if i == 0 { Some(format!("Fixture {area}")) } else { None };
            let written = iso::write(&scene, name, inputs, &rendered, o, &out).unwrap();
            // What is on disk, read back.
            let m: serde_json::Value = serde_json::from_slice(&std::fs::read(out.dir.join(format!("{area}.json"))).unwrap()).unwrap();
            assert_eq!((m["area"]["resref"].as_str(), m["views"].as_array().map(Vec::len)), (Some(area), Some(written.views.len())));
            let errs = errors(&v, &m);
            assert!(errs.is_empty(), "{area}, variant {i}: {errs:#?}\n{m:#}");
            for view in m["views"].as_array().unwrap() {
                assert!(out.dir.join(view["file"].as_str().unwrap()).is_file());
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 3 * fixture::AREAS.len());

    // And the schema says no to what is not a manifest v1.
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(out.dir.join("fx_main.json")).unwrap()).unwrap();
    assert!(errors(&v, &m).is_empty());
    type Break = Box<dyn Fn(&mut serde_json::Value)>;
    let broken: [(&str, Break); 7] = [
        ("another version", Box::new(|m| m["version"] = 2.into())),
        ("another format", Box::new(|m| m["format"] = "nwn-bake/area-scene".into())),
        ("no matrix", Box::new(|m| _ = m["views"][0].as_object_mut().unwrap().remove("matrix"))),
        ("a matrix of three columns", Box::new(|m| m["views"][0]["matrix"] = serde_json::json!([[1, 0, 0], [0, 1, 0]]))),
        ("an unknown view", Box::new(|m| m["views"][0]["view"] = "up".into())),
        ("an unknown kind", Box::new(|m| m["area"]["kind"] = "cellar".into())),
        ("a digest that is no sha256", Box::new(|m| m["inputs"]["are"] = "md5:00".into())),
    ];
    for (what, breaks) in broken {
        let mut b = m.clone();
        breaks(&mut b);
        assert!(!errors(&v, &b).is_empty(), "the schema takes a manifest with {what}");
    }
    // Fields it does not know are allowed (readers ignore them).
    m["new_field"] = 1.into();
    m["views"][0]["new_field"] = "x".into();
    assert!(errors(&v, &m).is_empty());
    let _ = std::fs::remove_dir_all(&f.base);
}
