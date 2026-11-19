// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The renderer on hand-made scenes (no game content): the transform puts game points where they are drawn in every view, the top
//! view looks straight down with north up, drawing is deterministic, and the roof cut and background do what the options say.

use std::sync::Arc;

use super::*;
use crate::area::floor::FloorProbe;
use crate::area::scene::{Counts, Light as SunLight, Part, SceneTexture, Timings};
use crate::texture::Mipped;

fn texture(name: &str, rgba: [u8; 4]) -> SceneTexture {
    let image = mg_image::Rgba { width: 1, height: 1, data: rgba.to_vec() };
    SceneTexture { name: name.into(), image: Some(Arc::new(Mipped::new(name.into(), &image))) }
}

fn quad(x0: f32, y0: f32, x1: f32, y1: f32, z: f32, texture: u32) -> [Triangle; 2] {
    let t = |p: [[f32; 3]; 3]| Triangle { p, uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], texture, part: Part::Tile(0), chasm: false };
    [t([[x0, y0, z], [x1, y0, z], [x1, y1, z]]), t([[x0, y0, z], [x1, y1, z], [x0, y1, z]])]
}

/// A 2 by 2 tile area (20 m): blue ground at 0 m, and a red 1 m square centred on each of `markers`.
fn scene(markers: &[[f32; 3]], flags: u32) -> Scene {
    let mut triangles: Vec<Triangle> = quad(0.0, 0.0, 20.0, 20.0, 0.0, 0).to_vec();
    for m in markers {
        triangles.extend(quad(m[0] - 0.5, m[1] - 0.5, m[0] + 0.5, m[1] + 0.5, m[2], 1));
    }
    let n = 33 * 33;
    Scene {
        resref: "test".into(),
        tileset: "tst01".into(),
        tiles: [2, 2],
        flags,
        tileset_interior: false,
        light: SunLight::default(),
        objects: true,
        textures: vec![texture("blue", [0, 0, 200, 255]), texture("red", [255, 0, 0, 255])],
        triangles,
        water: Vec::new(),
        probe: FloorProbe { nx: 33, ny: 33, heights: vec![0.0; n], colours: vec![[100; 3]; n], ..Default::default() },
        counts: Counts::default(),
        notes: Vec::new(),
        reads: Vec::new(),
        timings: Timings::default(),
    }
}

fn plain(views: Vec<View>) -> IsoOptions {
    IsoOptions { views, shadows: false, exposure: false, ..Default::default() }
}

/// The centroid of the red in a picture, weighted by how red each pixel is (the ground has none); None when there is none.
fn red_centroid(p: &Picture) -> Option<[f32; 2]> {
    let (mut sx, mut sy, mut sw) = (0.0f64, 0.0f64, 0.0f64);
    for y in 0..p.height as usize {
        for x in 0..p.width as usize {
            let px = &p.rgba[(y * p.width as usize + x) * 4..][..4];
            let w = f64::from(px[0]) * f64::from(px[3]) / 255.0;
            sx += (x as f64 + 0.5) * w;
            sy += (y as f64 + 0.5) * w;
            sw += w;
        }
    }
    (sw > 0.0).then(|| [(sx / sw) as f32, (sy / sw) as f32])
}

const ALL: [View; 5] = [View::Sw, View::Se, View::Ne, View::Nw, View::Top];

#[test]
fn the_matrix_puts_each_game_point_where_it_is_drawn_in_every_view() {
    for m in [[5.0, 5.0, 0.5], [15.0, 4.0, 3.0], [4.0, 16.0, 6.0], [14.5, 15.0, 1.5], [10.0, 10.0, 12.0]] {
        let r = render(&scene(&[m], 4), &plain(ALL.to_vec())).unwrap();
        for p in &r.pictures {
            let want = p.transform.project(m);
            let got = red_centroid(p).unwrap_or_else(|| panic!("{:?}: the marker at {m:?} is not drawn", p.view));
            let off = ((got[0] - want[0]).powi(2) + (got[1] - want[1]).powi(2)).sqrt();
            assert!(off < 0.5, "{:?}: the marker at {m:?} is drawn at {got:?}, the matrix says {want:?} ({off:.2} px off)", p.view);
        }
    }
}

#[test]
fn the_top_view_looks_straight_down_with_north_up_and_east_right() {
    let r = render(&scene(&[], 4), &plain(vec![View::Top])).unwrap();
    let t = &r.pictures[0].transform;
    assert_eq!(t.elevation_deg, 90.0);
    assert_eq!(t.rotation, [[1, 0], [0, 1]]);
    // x to the right, y up the picture, height not at all, at the scale.
    assert_eq!(t.matrix[0][..3], [8.0, 0.0, 0.0]);
    assert_eq!(t.matrix[1][..3], [0.0, -8.0, 0.0]);
    // The area fills the picture but for the padding (8 supersampled pixels: 4).
    assert_eq!((t.width, t.height), (168, 168));
    let sw = t.project([0.0, 0.0, 0.0]);
    let ne = t.project([20.0, 20.0, 5.0]);
    assert!(sw[0] < ne[0] && sw[1] > ne[1], "the south-west corner is bottom left, the north-east top right: {sw:?} {ne:?}");
    assert_eq!(sw, [4.0, 164.0]);
    // A marker in the north-east quarter is drawn there.
    let r = render(&scene(&[[16.0, 16.0, 1.0]], 4), &plain(vec![View::Top])).unwrap();
    let c = red_centroid(&r.pictures[0]).unwrap();
    assert!((c[0] - 132.0).abs() < 0.5 && (c[1] - 36.0).abs() < 0.5, "{c:?}");
}

#[test]
fn the_corner_views_keep_their_documented_transform() {
    // The numbers `docs/area-iso.md` gives for a south-west view at 8 px a metre: 45-degree axes, 30 degrees up.
    let r = render(&scene(&[], 4), &plain(View::CORNERS.to_vec())).unwrap();
    let m = r.pictures[0].transform.matrix;
    for (got, want) in m.iter().flat_map(|r| r[..3].iter()).zip([5.656854, -5.656854, 0.0, -2.828427, -2.828427, -6.928203]) {
        assert!((got - want).abs() < 1e-4, "{m:?}");
    }
    let rotations: Vec<_> = r.pictures.iter().map(|p| p.transform.rotation).collect();
    assert_eq!(rotations, [[[1, 0], [0, 1]], [[0, 1], [-1, 0]], [[-1, 0], [0, -1]], [[0, -1], [1, 0]]]);
    // A square area is as wide and high in every corner view.
    assert!(r.pictures.windows(2).all(|w| (w[0].width, w[0].height) == (w[1].width, w[1].height)));
}

#[test]
fn drawing_is_deterministic() {
    let s = scene(&[[5.0, 5.0, 0.5], [12.0, 14.0, 4.0]], 4);
    let o = IsoOptions { views: ALL.to_vec(), ..Default::default() };
    let (a, b) = (render(&s, &o).unwrap(), render(&s, &o).unwrap());
    for (a, b) in a.pictures.iter().zip(&b.pictures) {
        assert_eq!(a.rgba, b.rgba);
        assert_eq!(a.transform, b.transform);
    }
}

#[test]
fn views_parse_and_all_is_the_four_corners() {
    assert_eq!(View::parse_list("all").unwrap(), View::CORNERS);
    assert_eq!(View::parse_list("top").unwrap(), [View::Top]);
    assert_eq!(View::parse_list("sw, top,sw").unwrap(), [View::Sw, View::Top]);
    assert_eq!(View::parse_list("all,top").unwrap(), ALL);
    assert!(View::parse_list("up").is_err());
    assert!(View::parse_list("").is_err());
}

#[test]
fn an_interiors_roof_is_cut_and_keep_keeps_it() {
    // The floor is at 0 m, so an interior is cut at 3.5 m: the marker at 6 m goes, the one at 2 m stays.
    let s = scene(&[[6.0, 6.0, 6.0]], 1);
    let r = render(&s, &plain(vec![View::Top])).unwrap();
    assert_eq!(r.roof_cut.map(|c| c.at), Some(3.5));
    assert!(red_centroid(&r.pictures[0]).is_none());
    let r = render(&s, &IsoOptions { roof: Roof::Keep, ..plain(vec![View::Top]) }).unwrap();
    assert!(r.roof_cut.is_none() && red_centroid(&r.pictures[0]).is_some());
    // A surface area keeps its roof unless told how high to cut.
    let s = scene(&[[6.0, 6.0, 6.0]], 4);
    assert!(render(&s, &plain(vec![View::Top])).unwrap().roof_cut.is_none());
    let r = render(&s, &IsoOptions { roof: Roof::Over(2.0), ..plain(vec![View::Top]) }).unwrap();
    assert_eq!(r.roof_cut.map(|c| c.at), Some(2.0));
    assert!(red_centroid(&r.pictures[0]).is_none());
}

#[test]
fn the_background_is_transparent_or_the_colour_given() {
    let s = scene(&[], 4);
    let r = render(&s, &plain(vec![View::Sw])).unwrap();
    assert_eq!(r.pictures[0].rgba[..4], [0, 0, 0, 0], "the top-left corner is empty");
    let r = render(&s, &IsoOptions { background: Background::Colour([10, 20, 30]), ..plain(vec![View::Sw]) }).unwrap();
    assert_eq!(r.pictures[0].rgba[..4], [10, 20, 30, 255]);
    assert_eq!(parse_background("#0a141e").unwrap(), Background::Colour([10, 20, 30]));
    assert!(parse_background("blue").is_err());
}

#[test]
fn the_manifest_has_every_view_and_its_transform() {
    let s = scene(&[], 4);
    let o = plain(vec![View::Sw, View::Top]);
    let r = render(&s, &o).unwrap();
    let out = OutOptions::new("unused");
    let m = serde_json::to_value(manifest(&s, Some("Test".into()), Inputs::default(), &r, &o, &out)).unwrap();
    assert_eq!(m["format"], "nwn-bake/area-iso");
    assert_eq!(m["version"], 1);
    assert_eq!(m["area"]["kind"], "surface");
    assert_eq!(m["options"]["look"], "default");
    assert_eq!(m["views"][0]["file"], "test.sw.png");
    assert_eq!(m["views"][1]["view"], "top");
    assert_eq!(m["views"][1]["elevation_deg"], 90.0);
    assert_eq!(m["views"][1]["matrix"][1][2], 0.0);
    // Another look is written whole.
    let o = IsoOptions { look: Look { light_up: 0.5, ..Look::default() }, ..o };
    let m = serde_json::to_value(manifest(&s, None, Inputs::default(), &r, &o, &out)).unwrap();
    assert_eq!(m["options"]["look"]["light_up"], 0.5);
}

#[test]
fn a_look_file_overrides_only_what_it_names() {
    let look: Look = serde_json::from_str(r#"{"exposure_target": 180}"#).unwrap();
    assert_eq!(look, Look { exposure_target: 180.0, ..Look::default() });
    assert!(serde_json::from_str::<Look>(r#"{"exposure": 1}"#).is_err(), "an unknown field is an error");
}

#[test]
fn the_roof_cut_takes_the_water_above_it_too() {
    // An interior cut at 3.5 m: water at 5 m (a cave's rock top) goes, water at 1 m stays.
    let mut s = scene(&[], 1);
    s.water = quad(2.0, 2.0, 8.0, 8.0, 5.0, 0).iter().map(|t| t.p).collect();
    let r = render(&s, &plain(vec![View::Top])).unwrap();
    let blue_water = |p: &Picture| p.rgba.as_chunks::<4>().0.iter().filter(|c| c[1] > 40).count();
    assert_eq!(blue_water(&r.pictures[0]), 0, "the water above the cut is drawn");
    s.water = quad(2.0, 2.0, 8.0, 8.0, 1.0, 0).iter().map(|t| t.p).collect();
    let r = render(&s, &plain(vec![View::Top])).unwrap();
    assert!(blue_water(&r.pictures[0]) > 1000, "the water below the cut is not drawn");
}

#[test]
fn black_void_tile_faces_are_not_drawn_but_keep_the_picture_s_bounds() {
    // The ground's east half is a tileset's black void; a placeable with the same black texture is still drawn.
    let mut s = scene(&[], 4);
    s.textures.push(texture("tst01_black", [3, 2, 1, 255]));
    s.triangles = quad(0.0, 0.0, 10.0, 20.0, 0.0, 0).to_vec();
    s.triangles.extend(quad(10.0, 0.0, 20.0, 20.0, 0.0, 2));
    let mut post = quad(14.0, 9.0, 16.0, 11.0, 0.5, 2);
    post.iter_mut().for_each(|t| t.part = Part::Placeable(0));
    s.triangles.extend(post);
    let all_void = |s: &Scene| s.textures.iter().map(|t| t.image.as_deref().is_some_and(Mipped::is_black)).collect::<Vec<_>>();
    assert_eq!(all_void(&s), [false, false, true]);
    let o = IsoOptions { background: Background::Colour([255, 255, 255]), ..plain(vec![View::Top, View::Sw]) };
    let r = render(&s, &o).unwrap();
    let top = &r.pictures[0];
    let px = |x: f32, y: f32| {
        let m = top.transform.matrix;
        let (u, v) = (m[0][0] * x + m[0][1] * y + m[0][3], m[1][0] * x + m[1][1] * y + m[1][3]);
        top.rgba[(v as usize * top.width as usize + u as usize) * 4..][..4].to_vec()
    };
    let floor = px(5.0, 5.0);
    assert!(floor[2] > 100 && floor[0] == 0 && floor[3] == 255, "the floor is drawn: {floor:?}");
    assert_eq!(px(18.0, 5.0), [255, 255, 255, 255], "the void shows the background");
    assert!(px(15.0, 10.0)[..3].iter().all(|&c| c < 10), "a placeable's black is drawn");
    // The same size and transform as with the void drawn, in every view.
    let mut drawn = s.clone();
    drawn.textures[2] = texture("tst01_grey", [90, 90, 90, 255]);
    let d = render(&drawn, &o).unwrap();
    for (a, b) in r.pictures.iter().zip(&d.pictures) {
        assert_eq!((a.width, a.height, a.transform.matrix), (b.width, b.height, b.transform.matrix));
    }
    // An area of nothing but void comes out empty, at its full size.
    let mut void = s.clone();
    void.triangles = quad(0.0, 0.0, 20.0, 20.0, 0.0, 2).to_vec();
    let v = render(&void, &plain(vec![View::Sw])).unwrap();
    assert!(v.pictures[0].rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    assert_eq!(v.pictures[0].width, d.pictures[1].width);
}

/// A chasm in the middle of the test area: ground (blue) around it 4 m wide, its floor (red, which must not show) 2.4 m down.
fn chasm_scene() -> Scene {
    let mut s = scene(&[], 4);
    s.triangles = [
        quad(0.0, 0.0, 20.0, 4.0, 0.0, 0),
        quad(0.0, 16.0, 20.0, 20.0, 0.0, 0),
        quad(0.0, 4.0, 4.0, 16.0, 0.0, 0),
        quad(16.0, 4.0, 20.0, 16.0, 0.0, 0),
    ]
    .concat();
    let mut floor = quad(4.0, 4.0, 16.0, 16.0, -2.4, 1);
    floor.iter_mut().for_each(|t| t.chasm = true);
    s.triangles.extend(floor);
    s
}

#[test]
fn a_chasm_floor_is_dark_blue_grey_and_darker_towards_its_middle() {
    let s = chasm_scene();
    let r = render(&s, &plain(vec![View::Top])).unwrap();
    let top = &r.pictures[0];
    let px = |x: f32, y: f32| {
        let m = top.transform.matrix;
        let (u, v) = (m[0][0] * x + m[0][1] * y + m[0][3], m[1][0] * x + m[1][1] * y + m[1][3]);
        top.rgba[(v as usize * top.width as usize + u as usize) * 4..][..4].to_vec()
    };
    let (seam, near, middle) = (px(4.05, 10.0), px(5.5, 10.0), px(10.0, 10.0));
    for p in [&seam, &near, &middle] {
        assert_eq!(p[3], 255, "drawn");
        assert!(p[2] >= p[1] && p[1] >= p[0], "blue-grey, not the texture's red: {p:?}");
        assert!(p[2] < 60, "dark: {p:?}");
    }
    let sum = |p: &[u8]| p[..3].iter().map(|&c| u32::from(c)).sum::<u32>();
    assert!(sum(&middle) < sum(&near), "darker towards the middle: {middle:?} {near:?}");
    assert!(sum(&seam) < sum(&near), "up from black at the foot of the walls: {seam:?} {near:?}");
    // Every corner view draws it alike (the colour is the scene's, not the view's).
    let corners = render(&s, &plain(View::CORNERS.to_vec())).unwrap();
    let darkest = |p: &Picture| {
        p.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|c| c[3] == 255)
            .map(|c| (u32::from(c[0]), u32::from(c[1]), u32::from(c[2])))
            .min_by_key(|c| c.0 + c.1 + c.2)
    };
    let d: Vec<_> = corners.pictures.iter().map(darkest).collect();
    assert!(d.windows(2).all(|w| w[0] == w[1]), "{d:?}");
    // Its colours come from the look.
    let mut o = plain(vec![View::Top]);
    o.look.chasm_edge = [0.0, 0.5, 0.0];
    o.look.chasm_deep = [0.0, 0.5, 0.0];
    let g = render(&s, &o).unwrap();
    let gp = &g.pictures[0];
    let m = gp.transform.matrix;
    let (u, v) = (m[0][0] * 10.0 + m[0][1] * 10.0 + m[0][3], m[1][0] * 10.0 + m[1][1] * 10.0 + m[1][3]);
    let c = &gp.rgba[(v as usize * gp.width as usize + u as usize) * 4..][..3];
    assert!(c[1] > 50 && c[0] == 0 && c[2] == 0, "{c:?}");
}

#[test]
fn a_chasm_floor_leaves_the_exposure_and_the_rest_of_the_picture_as_they_were() {
    // The same area with the floor drawn with its texture (red): only the red pixels change, and the exposure is the same.
    let s = chasm_scene();
    let mut textured = s.clone();
    textured.triangles.iter_mut().for_each(|t| t.chasm = false);
    let o = IsoOptions { exposure: true, shadows: true, ..plain(vec![View::Top, View::Sw]) };
    let (a, b) = (render(&s, &o).unwrap(), render(&textured, &o).unwrap());
    for (pa, pb) in a.pictures.iter().zip(&b.pictures) {
        assert!(pb.exposure > 1.0, "the test needs the exposure lifted");
        assert_eq!(pa.exposure, pb.exposure);
        assert_eq!((pa.width, pa.height), (pb.width, pb.height));
        let (mut differ, mut red) = (0, 0);
        for (x, y) in pa.rgba.as_chunks::<4>().0.iter().zip(pb.rgba.as_chunks::<4>().0.iter()) {
            // (The ground is pure blue: any red is the floor's, whole or at an edge.)
            red += usize::from(y[0] > 0);
            if x != y {
                assert!(y[0] > 0, "a pixel that is not the floor's changed: {x:?} {y:?}");
                differ += 1;
            }
        }
        assert!(differ > red / 2, "the floor is drawn in the chasm's colours: {differ} of {red}");
    }
}

#[test]
fn a_picture_too_large_fails_before_anything_is_allocated() {
    // (A scale of 100,000 once asked for about 100 TB and aborted the process; inside `serve`, the server.)
    for scale in [100_000.0, 1e30, f32::MAX] {
        let o = IsoOptions { scale, ..plain(ALL.to_vec()) };
        let Err(e) = render(&scene(&[], 4), &o) else { panic!("scale {scale}: drawn") };
        let e = format!("{e:#}");
        assert!(e.contains("lower the scale") && e.starts_with("view sw: the picture would be"), "{e}");
    }
    // A picture within the limits is drawn; one past 16,383 pixels a side is refused.
    assert!(render(&scene(&[], 4), &IsoOptions { scale: 64.0, quality: 1, ..plain(vec![View::Top]) }).is_ok());
    let Err(e) = render(&scene(&[], 4), &IsoOptions { scale: 1000.0, quality: 1, ..plain(vec![View::Top]) }) else { panic!("drawn") };
    let e = format!("{e:#}");
    assert!(e.contains("at most 16383 a side"), "{e}");
}

/// A 2 by 2 tile area on two levels: blue ground at 0 m west of x = 10 m, a green plateau at 6 m east of it whose walkmesh does not
/// walk (so the probe's walkable ground is 0 m everywhere, and only its walkmesh top is 6 m), with a red 1 m square on each of
/// `markers`.
fn two_levels(markers: &[[f32; 3]]) -> Scene {
    let mut s = scene(markers, 4);
    s.textures.push(texture("green", [0, 200, 0, 255]));
    s.triangles.splice(0..2, quad(0.0, 0.0, 10.0, 20.0, 0.0, 0).into_iter().chain(quad(10.0, 0.0, 20.0, 20.0, 6.0, 2)));
    let probe = &mut s.probe;
    probe.walkmesh_top = (0..probe.nx * probe.ny).map(|k| Some(if probe.sample_x(k % probe.nx) < 10.0 { 0.0 } else { 6.0 })).collect();
    s
}

/// The colour of the top view's pixel over game point (`x`, `y`).
fn top_pixel(p: &Picture, x: f32, y: f32) -> [u8; 4] {
    let [u, v] = p.transform.project([x, y, 0.0]);
    if u < 0.0 || v < 0.0 || u >= p.width as f32 || v >= p.height as f32 {
        return [0; 4];
    }
    let c = &p.rgba[(v as usize * p.width as usize + u as usize) * 4..][..4];
    [c[0], c[1], c[2], c[3]]
}

/// Which of the scene's colours a pixel shows: `r`, `g`, `b`, or `-` for nothing.
fn hue(c: [u8; 4]) -> char {
    match c {
        [.., 0] => '-',
        [r, g, b, _] if r > g && r > b => 'r',
        [_, g, b, _] if g > b => 'g',
        _ => 'b',
    }
}

#[test]
fn the_roof_cut_along_the_ground_keeps_each_levels_ground_and_cuts_its_roof() {
    // On the low ground, markers at 1 m (kept) and 3 m (cut); on the plateau, at 7 m (kept) and 9 m (cut).
    let s = two_levels(&[[5.0, 5.0, 1.0], [5.0, 15.0, 3.0], [15.0, 5.0, 7.0], [15.0, 15.0, 9.0]]);
    let at = |r: &Rendered| {
        [(5.0, 5.0), (5.0, 15.0), (15.0, 5.0), (15.0, 15.0), (5.0, 10.0), (15.0, 10.0)].map(|(x, y)| hue(top_pixel(&r.pictures[0], x, y)))
    };
    let r = render(&s, &IsoOptions { roof: Roof::Ground(2.0), ..plain(vec![View::Top]) }).unwrap();
    assert_eq!(at(&r), ['r', 'b', 'r', 'g', 'b', 'g']);
    assert_eq!((r.roof_cut, r.roof_over_ground), (None, Some(2.0)));
    // One flat cut 2 m over the floor (0 m) takes the plateau too.
    let r = render(&s, &IsoOptions { roof: Roof::Over(2.0), ..plain(vec![View::Top]) }).unwrap();
    assert_eq!(at(&r), ['r', 'b', '-', '-', 'b', '-']);
    // With shadows and in every view it draws, and 0 keeps the roof.
    assert!(render(&s, &IsoOptions { roof: Roof::Ground(2.0), shadows: true, ..plain(ALL.to_vec()) }).is_ok());
    let r = render(&s, &IsoOptions { roof: Roof::Ground(0.0), ..plain(vec![View::Top]) }).unwrap();
    assert_eq!(at(&r)[3], 'r');
    assert_eq!(r.roof_over_ground, None);
}

#[test]
fn a_triangle_across_the_cut_is_cut_along_it() {
    // A red ramp on the low ground from 1 m at x = 2 m to 3 m at x = 8 m: cut at 2 m, at x = 5 m.
    let mut s = two_levels(&[]);
    s.triangles.extend(quad(2.0, 4.0, 8.0, 6.0, 0.0, 1).map(|mut t| {
        t.p.iter_mut().for_each(|p| p[2] = 1.0 + (p[0] - 2.0) / 3.0);
        t
    }));
    let r = render(&s, &IsoOptions { roof: Roof::Ground(2.0), ..plain(vec![View::Top]) }).unwrap();
    let p = &r.pictures[0];
    assert_eq!([3.0, 4.8, 5.2, 7.0].map(|x| hue(top_pixel(p, x, 5.0))), ['r', 'r', 'b', 'b']);
}

#[test]
fn the_cut_along_the_ground_falls_back_on_the_probe_and_leaves_no_hole_in_the_top_view() {
    // No walkmesh face in the north-west quarter: the probe's ground (0 m) stands in, and a marker at 3 m there is cut.
    let mut s = two_levels(&[[5.0, 15.0, 3.0]]);
    let probe = &mut s.probe;
    for k in 0..probe.nx * probe.ny {
        if probe.sample_x(k % probe.nx) < 10.0 && probe.sample_y(k / probe.nx) > 10.0 {
            probe.walkmesh_top[k] = None;
        }
    }
    let r = render(&s, &IsoOptions { roof: Roof::Ground(2.0), ..plain(vec![View::Top]) }).unwrap();
    assert_eq!(hue(top_pixel(&r.pictures[0], 5.0, 15.0)), 'b');
    // A probe without walkmesh tops at all (made by hand) is cut over its ground, 0 m: the plateau is cut away, but nothing else is
    // under it, so the top view draws the lowest of what was cut there and has no hole; the corner views do not.
    s.probe.walkmesh_top.clear();
    let r = render(&s, &IsoOptions { roof: Roof::Ground(2.0), ..plain(vec![View::Top, View::Sw]) }).unwrap();
    assert_eq!(hue(top_pixel(&r.pictures[0], 15.0, 10.0)), 'g');
    let green = |p: &Picture| p.rgba.as_chunks::<4>().0.iter().filter(|c| c[3] > 0 && c[1] > c[2]).count();
    assert_eq!(green(&r.pictures[1]), 0, "the sw view draws what was cut");
}

#[test]
fn a_small_raised_patch_of_walkmesh_is_a_building_not_ground() {
    // A building's block in the middle of the low ground: walkmesh 4 m up over 5 by 5 m, its roof (red) at 4.5 m and its floor (the
    // blue ground) under it. The roof is cut; the plateau, as wide as the area, is still ground.
    let mut s = two_levels(&[[5.0, 10.0, 4.5]]);
    let probe = &mut s.probe;
    for k in 0..probe.nx * probe.ny {
        let (x, y) = (probe.sample_x(k % probe.nx), probe.sample_y(k / probe.nx));
        if (2.5..7.5).contains(&x) && (7.5..12.5).contains(&y) {
            probe.walkmesh_top[k] = Some(4.0);
        }
    }
    let r = render(&s, &IsoOptions { roof: Roof::Ground(2.0), ..plain(vec![View::Top]) }).unwrap();
    assert_eq!([(5.0, 10.0), (15.0, 10.0)].map(|(x, y)| hue(top_pixel(&r.pictures[0], x, y))), ['b', 'g']);
}

#[test]
fn the_roof_parses_and_is_written_as_given() {
    assert_eq!(Roof::parse("auto").unwrap(), Roof::Auto);
    assert_eq!(Roof::parse("keep").unwrap(), Roof::Keep);
    assert_eq!(Roof::parse("-2").unwrap(), Roof::Over(-2.0));
    assert_eq!(Roof::parse("ground:2.5").unwrap(), Roof::Ground(2.5));
    for bad in ["ground:", "ground:high", "ground:inf", "high", ""] {
        assert!(Roof::parse(bad).is_err(), "{bad:?}");
    }
    assert_eq!(Roof::Ground(2.0).to_value(), "ground:2");
    assert_eq!(Roof::Ground(1.5).to_value(), "ground:1.5");
    assert_eq!(Roof::Over(2.0).to_value(), serde_json::json!(2.0));
    // The manifest says how high over the ground it cut, and has no flat cut.
    let s = two_levels(&[]);
    let o = IsoOptions { roof: Roof::Ground(2.0), ..plain(vec![View::Top]) };
    let r = render(&s, &o).unwrap();
    let m = serde_json::to_value(manifest(&s, None, Inputs::default(), &r, &o, &OutOptions::new("unused"))).unwrap();
    assert_eq!(
        (&m["options"]["roof"], &m["roof_cut_m"], &m["roof_over_ground_m"]),
        (&"ground:2".into(), &serde_json::Value::Null, &2.0.into())
    );
    // Without it, the field is left out, so the manifests of the other roofs are as they were.
    let r = render(&s, &plain(vec![View::Top])).unwrap();
    let m = serde_json::to_value(manifest(&s, None, Inputs::default(), &r, &plain(vec![View::Top]), &OutOptions::new("unused"))).unwrap();
    assert!(m.get("roof_over_ground_m").is_none());
}
