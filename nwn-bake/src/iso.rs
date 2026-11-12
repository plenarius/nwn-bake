// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Area pictures: an area's [`Scene`] drawn on the CPU, from a corner 30 degrees above the horizon (`sw`, `se`, `ne`, `nw`) or straight
//! down (`top`), with each picture's transform from game coordinates to its pixels.
//!
//! A rasteriser with mip-mapped textures draws the scene's triangles: the tile models' own textures and UVs, each face lit by its
//! own normal, with a shadow map, water where the tiles' water faces are, an interior's roof cut open over its floor, dark areas
//! lifted, and a transparent or plain background. Each corner view is the south-west one with the scene turned a quarter turn per
//! view (coordinates swapped and negated, nothing rounded), so the light and shadows, which are set relative to the camera, fall
//! the same way in every corner view. The top view looks straight down, north up and east right, at the same scale; it is lit as
//! the south-west view is, so its shadows fall as in that picture.
//!
//! How the pictures look (the camera's angle, the light, the water, the shadows, the exposure, the chasms) is set by the constants
//! in [`Look`]. To propose a change to the look: try the new values with `--look <file>` (no build needed), change [`Look`]'s
//! defaults, raise [`PICTURES_VERSION`], and show the same areas before and after in the pull request (`docs/testing.md`).
//!
//! Drawing is deterministic: each view's triangles are drawn in the scene's order on one thread, so the same scene and options give
//! the same pixels. The views of an area are drawn at once, one thread each ([`render`]), or one at a time by whoever calls
//! [`Prepared::draw`] (a batch, [`batch`]); either way each view's picture is the same, and the notes are put together in the
//! views' order.

pub mod batch;
mod chasm;
mod draw;
mod output;
mod shadows;

pub use output::{
    AreaInfo, FORMAT, Format, Inputs, Manifest, OptionsInfo, OutOptions, PICTURES_VERSION, Renderer, VERSION, ViewInfo, WEBP_QUALITY,
    encode, encode_depth, file_name, manifest, parse_background, write, write_file, write_png,
};

use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::area::{RoofCut, RoofRule, Scene, Triangle};
use chasm::ChasmField;
use draw::{Camera, Face, Frame, MAX_SAMPLES, MAX_SIDE_PX, draw, frame};

/// Where the camera stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    /// From the south-west corner, looking north-east (each corner view is a quarter turn more than the one before).
    Sw,
    /// From the south-east corner, looking north-west.
    Se,
    /// From the north-east corner, looking south-west.
    Ne,
    /// From the north-west corner, looking south-east.
    Nw,
    /// Straight down, north up.
    Top,
}

impl View {
    /// The four corners, in the order `all` means.
    pub const CORNERS: [View; 4] = [View::Sw, View::Se, View::Ne, View::Nw];

    /// The view's name in options, file names and manifests: `sw`, `se`, `ne`, `nw` or `top`.
    pub fn name(self) -> &'static str {
        match self {
            View::Sw => "sw",
            View::Se => "se",
            View::Ne => "ne",
            View::Nw => "nw",
            View::Top => "top",
        }
    }

    /// `sw`, `se`, `ne`, `nw`, `top`, `all` (the four corners) or a list of them separated by commas (`sw,top`). Each view once, in
    /// the order given.
    pub fn parse_list(s: &str) -> Result<Vec<View>> {
        let mut views = Vec::new();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let more: Vec<View> = match part {
                "all" => View::CORNERS.to_vec(),
                _ => vec![
                    [View::Sw, View::Se, View::Ne, View::Nw, View::Top]
                        .into_iter()
                        .find(|v| v.name() == part)
                        .with_context(|| format!("view {part:?}: one of sw, se, ne, nw, top or all"))?,
                ],
            };
            for v in more {
                if !views.contains(&v) {
                    views.push(v);
                }
            }
        }
        ensure!(!views.is_empty(), "no view");
        Ok(views)
    }

    /// Quarter turns of the scene that put this view's camera where the south-west one stands.
    fn turns(self) -> u8 {
        match self {
            View::Sw | View::Top => 0,
            View::Se => 1,
            View::Ne => 2,
            View::Nw => 3,
        }
    }
}

/// The constants that set how the pictures look. `Default` is what nwn-bake draws with; a picture drawn with another `Look`
/// (`--look`) says so in its manifest. Changing a default changes every picture: raise [`PICTURES_VERSION`] with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Look {
    /// The corner views' camera: degrees above the horizon.
    pub elevation_deg: f32,
    /// How far to the camera's left the light comes from: part of a unit vector, added to the direction towards the camera.
    pub light_side: f32,
    /// How far above the camera the light comes from, as `light_side`.
    pub light_up: f32,
    /// Plain daylight (`--light day`): the ambient colour, 0..1.
    pub day_ambient: [f32; 3],
    /// Plain daylight: the sun's colour, 0..1.
    pub day_diffuse: [f32; 3],
    /// The ambient light (day or the area's) is scaled by this.
    pub ambient: f32,
    /// The sun's light (day or the area's) is scaled by this.
    pub diffuse: f32,
    /// The most a surface is lit (a little over its own colour, so sunlit ground is brighter than its texture).
    pub light_max: f32,
    /// The water's colour, 0..1.
    pub water: [f32; 3],
    /// How much the water's colour covers what is under it, 0..1.
    pub water_opacity: f32,
    /// The shadow map's cells along a side: one cell for each pixel drawn (the supersampled picture's), rounded up to a power of
    /// two, and at most this.
    pub shadow_size: u32,
    /// The shadow map's fewest cells along a side.
    pub shadow_min_size: u32,
    /// How far (metres) a surface may be behind the nearest one the sun sees and still be lit; two cells of the shadow map where
    /// that is more.
    pub shadow_bias: f32,
    /// Exposure: the brightest 2% of what is drawn is lifted to about this (0..255 luminance), by at most `exposure_max` times, more
    /// where it is dark: a pixel at v is multiplied by 1 + (gain - 1) * (1 - v/255)^`exposure_lift`.
    pub exposure_target: f32,
    /// The power in `exposure_target`'s formula: the higher, the more the lift is kept to the dark.
    pub exposure_lift: f32,
    /// The exposure's highest gain.
    pub exposure_max: f32,
    /// A chasm's floor ([`Triangle::chasm`]: the tile's black bottom of a drop) is drawn in these colours (0..1, lit as a texture's
    /// colour is) instead of its texture: this one at its edge, darkening to `chasm_deep` `chasm_reach` metres in from the nearest
    /// ground, so a wide chasm reads as a deep drop.
    pub chasm_edge: [f32; 3],
    /// A chasm floor's colour at `chasm_reach` metres or more from the nearest ground, 0..1.
    pub chasm_deep: [f32; 3],
    /// How far in from the nearest ground (metres) a chasm's floor takes `chasm_deep`.
    pub chasm_reach: f32,
    /// Within this many metres of the edge, a chasm floor's colour comes up from black, as the chasm's walls fade to black at their
    /// foot.
    pub chasm_seam: f32,
    /// The roof cut for interiors and underground areas.
    pub roof: RoofRule,
    /// A texel less opaque than this (0..255) is a hole (a cut-out's clear part).
    pub alpha_cut: f32,
    /// Empty pixels around the drawing, in pixels of the supersampled picture.
    pub padding: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            elevation_deg: 30.0,
            light_side: 0.35,
            light_up: 0.3,
            day_ambient: [0.24; 3],
            day_diffuse: [0.67; 3],
            ambient: 0.8,
            diffuse: 1.4,
            light_max: 1.6,
            water: [0.173, 0.416, 0.525],
            water_opacity: 0.62,
            shadow_size: 4096,
            shadow_min_size: 1024,
            shadow_bias: 0.12,
            exposure_target: 200.0,
            exposure_lift: 1.0,
            exposure_max: 6.0,
            chasm_edge: [0.10, 0.115, 0.145],
            chasm_deep: [0.035, 0.042, 0.058],
            chasm_reach: 10.0,
            chasm_seam: 0.5,
            roof: RoofRule::default(),
            alpha_cut: 90.0,
            padding: 8.0,
        }
    }
}

/// Which light the area is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Light {
    /// Plain daylight, the same for every area, so every area can be read and compared.
    Day,
    /// The area's own sun (`SunAmbientColor`, `SunDiffuseColor`): a crypt lit by nothing but its torches goes dark, a swamp green.
    Area,
}

/// Whether and where the roof is cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Roof {
    /// Interiors and underground areas are cut by [`Look::roof`]; surface areas keep their roofs.
    Auto,
    /// Never cut.
    Keep,
    /// Cut every area this many metres over its floor (the rule of [`Look::roof`] with this height); 0 or less keeps the roof.
    Over(f32),
}

/// What is behind the drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    /// Transparent: a pixel's alpha is how much of it is drawn.
    Transparent,
    /// This colour, RGB: the picture is opaque everywhere.
    Colour([u8; 3]),
}

/// How to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoOptions {
    /// The views to draw, each once, in this order.
    pub views: Vec<View>,
    /// Pixels a metre.
    pub scale: f32,
    /// Supersampling: each pixel is drawn as `quality` by `quality` and averaged (1..4).
    pub quality: u8,
    /// Which light the area is drawn in.
    pub light: Light,
    /// Shadows from the sun, from a shadow map ([`Look::shadow_size`]).
    pub shadows: bool,
    /// Whether and where the roof is cut.
    pub roof: Roof,
    /// Dark areas lifted ([`Look::exposure_target`]).
    pub exposure: bool,
    /// What is behind the drawing.
    pub background: Background,
    /// Also a picture of each pixel's height ([`Picture::depth`]).
    pub depth: bool,
    /// The constants that set the look.
    pub look: Look,
}

impl Default for IsoOptions {
    fn default() -> Self {
        IsoOptions {
            views: vec![View::Sw],
            scale: 8.0,
            quality: 2,
            light: Light::Day,
            shadows: true,
            roof: Roof::Auto,
            exposure: true,
            background: Background::Transparent,
            depth: false,
            look: Look::default(),
        }
    }
}

/// Where a game point lands in a picture.
///
/// Game coordinates are metres as in the `.git`: x east, y north, z up, the area's south-west corner at (0, 0). [`Transform::matrix`]
/// maps (x, y, z, 1) to a pixel position (px, py) from the picture's top-left corner; pixel (i, j) covers [i, i+1) by [j, j+1), so its
/// centre is at (i + 0.5, j + 0.5). The other fields describe how the matrix was made: the view's quarter turn (`rotation`: `x' =
/// r[0][0] x + r[0][1] y`, `y' = r[1][0] x + r[1][1] y`), then the 45-degree axes of a corner view (none for `top`), the camera's
/// elevation, the scale, and the offsets.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Transform {
    /// The view.
    pub view: View,
    /// The camera's height above the horizon, degrees: [`Look::elevation_deg`] for a corner view, 90 for `top`.
    pub elevation_deg: f32,
    /// Pixels a metre ([`IsoOptions::scale`]).
    pub scale: f32,
    /// The picture's width in pixels.
    pub width: u32,
    /// The picture's height in pixels.
    pub height: u32,
    /// The view's quarter turn of the scene, as a 2 by 2 matrix (see above).
    pub rotation: [[i32; 2]; 2],
    /// The pixel position of the game's origin: `matrix`'s last column.
    pub offset: [f32; 2],
    /// Maps a game point (x, y, z, 1) to its pixel position (px, py). The normative part: the other fields only say how it was made.
    pub matrix: [[f32; 4]; 2],
}

impl Transform {
    /// The pixel position of game point `p`.
    pub fn project(&self, p: [f32; 3]) -> [f32; 2] {
        let m = &self.matrix;
        [m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3], m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3]]
    }
}

/// One view of the area.
#[derive(Clone, Debug)]
pub struct Picture {
    /// The view.
    pub view: View,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA, rows from the top. Opaque everywhere unless the background is transparent.
    pub rgba: Vec<u8>,
    /// Where a game point lands in it.
    pub transform: Transform,
    /// The exposure's gain (1 when it was not lifted).
    pub exposure: f32,
    /// With [`IsoOptions::depth`]: the game height (z, metres) of what each pixel shows.
    pub depth: Option<Depth>,
    /// How long drawing it took.
    pub time: Duration,
}

/// The height of what each pixel of a picture shows, as 16-bit values: 0 where nothing is drawn, else z = `range[0]` + v / 65535 *
/// (`range[1]` - `range[0]`). Where the pixel's samples (`quality` by `quality`) show different things, the nearest to the camera
/// counts; water counts where it is over what is under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Depth {
    /// The heights (metres) that the values 0 and 65535 stand for: lowest and highest. A drawn pixel is 1 or more, since 0 means
    /// nothing is drawn.
    pub range: [f32; 2],
    /// Rows from the top, as the picture's.
    pub values: Vec<u16>,
}

/// What [`render`] made.
#[derive(Clone, Debug)]
pub struct Rendered {
    /// One picture a view, in [`IsoOptions::views`]' order.
    pub pictures: Vec<Picture>,
    /// Where the roof was cut, if it was.
    pub roof_cut: Option<RoofCut>,
    /// What looked wrong in the pictures (a texture drawn nearly white over much of a picture).
    pub notes: Vec<String>,
}

/// Draws `scene` as `o` says: one picture per view, each view on a thread of its own.
pub fn render(scene: &Scene, o: &IsoOptions) -> Result<Rendered> {
    let prepared = prepare(scene, o)?;
    let drawn: Vec<Drawn> = std::thread::scope(|scope| {
        let prepared = &prepared;
        let handles: Vec<_> = o.views.iter().map(|&v| scope.spawn(move || prepared.draw(v))).collect();
        handles.into_iter().map(|h| h.join().expect("drawing a view panicked")).collect()
    });
    Ok(prepared.assemble(drawn))
}

/// What every view of an area is drawn from: the scene with its roof cut, its lights and its chasms' colours, made once
/// ([`prepare`]). Owns what it needs, so it can be shared by threads drawing its views ([`Prepared::draw`]).
pub struct Prepared {
    o: IsoOptions,
    textures: Vec<crate::area::SceneTexture>,
    triangles: Vec<Triangle>,
    water: Vec<Face>,
    size: [f32; 2],
    roof_cut: Option<RoofCut>,
    lights: ([f32; 3], [f32; 3]),
    chasm: ChasmField,
}

/// One view drawn, and what looked wrong in it.
pub struct Drawn {
    /// The view's picture.
    pub picture: Picture,
    /// What looked wrong in it (a texture drawn nearly white over much of the picture).
    pub notes: Vec<String>,
}

/// Gets `scene` ready to be drawn as `o` says (nothing drawn yet).
pub fn prepare(scene: &Scene, o: &IsoOptions) -> Result<Prepared> {
    ensure!(o.scale.is_finite() && o.scale > 0.0, "the scale must be more than 0");
    ensure!((1..=4).contains(&o.quality), "the quality is 1 to 4");
    ensure!(!o.views.is_empty(), "no view");
    let look = &o.look;
    let roof_cut = match o.roof {
        Roof::Auto => scene.roof_cut(&look.roof),
        Roof::Keep => None,
        Roof::Over(m) if m > 0.0 => Some(scene.probe.stats().roof_cut(&RoofRule { over: m, ..look.roof })),
        Roof::Over(_) => None,
    };
    // (The water too: a cave's water at the rock's top would otherwise cover the floors cut open under it.)
    let (triangles, water) = match roof_cut {
        Some(c) => (crate::area::cut(scene.triangles.clone(), c.at), cut_water(&scene.water, c.at)),
        None => (scene.triangles.clone(), scene.water.clone()),
    };
    let (amb, dif) = match o.light {
        Light::Day => (look.day_ambient, look.day_diffuse),
        Light::Area => (scene.light.ambient.map(|c| f32::from(c) / 255.0), scene.light.diffuse.map(|c| f32::from(c) / 255.0)),
    };
    let lights = (amb.map(|c| c * look.ambient), dif.map(|c| c * look.diffuse));
    let chasm = ChasmField::new(scene.size(), &triangles, look);
    let prepared =
        Prepared { o: o.clone(), textures: scene.textures.clone(), triangles, water, size: scene.size(), roof_cut, lights, chasm };
    // (Before anything is allocated: a scale of 100,000 would ask for terabytes and abort the process, a server's too.)
    for &view in &o.views {
        let Frame { w, h, .. } = frame(&prepared, &Camera::of(view, look));
        let ss = usize::from(o.quality);
        ensure!(
            w / ss <= MAX_SIDE_PX && h / ss <= MAX_SIDE_PX && w.saturating_mul(h) <= MAX_SAMPLES,
            "view {}: the picture would be {} by {} pixels at scale {} and supersampling {}; at most {MAX_SIDE_PX} a side and \
             {MAX_SAMPLES} samples: lower the scale",
            view.name(),
            w / ss,
            h / ss,
            o.scale,
            o.quality,
        );
    }
    Ok(prepared)
}

impl Prepared {
    /// Where the roof is cut, if it is.
    pub fn roof_cut(&self) -> Option<RoofCut> {
        self.roof_cut
    }

    /// The views asked for, in order.
    pub fn views(&self) -> &[View] {
        &self.o.views
    }

    /// Draws one view.
    pub fn draw(&self, view: View) -> Drawn {
        let started = Instant::now();
        let mut notes = Vec::new();
        let mut picture = draw(self, Camera::of(view, &self.o.look), &mut notes);
        picture.time = started.elapsed();
        Drawn { picture, notes }
    }

    /// The views drawn (in the order asked for) put together: the pictures, the roof cut, and the notes in the views' order, each
    /// texture's note once.
    pub fn assemble(&self, drawn: Vec<Drawn>) -> Rendered {
        let mut notes: Vec<String> = Vec::new();
        let mut pictures = Vec::with_capacity(drawn.len());
        for d in drawn {
            for n in d.notes {
                let texture = n.split_once(" is drawn").map(|(t, _)| format!("{t} "));
                if !texture.as_ref().is_some_and(|t| notes.iter().any(|x| x.starts_with(t.as_str()))) {
                    notes.push(n);
                }
            }
            pictures.push(d.picture);
        }
        Rendered { pictures, roof_cut: self.roof_cut, notes }
    }
}

/// Water faces with everything above `at` metres cut away ([`crate::area::cut`]).
fn cut_water(water: &[[[f32; 3]; 3]], at: f32) -> Vec<[[f32; 3]; 3]> {
    let as_triangles =
        water.iter().map(|p| Triangle { p: *p, uv: [[0.0; 2]; 3], texture: 0, part: crate::area::Part::Tile(0), chasm: false }).collect();
    crate::area::cut(as_triangles, at).into_iter().map(|t| t.p).collect()
}

#[cfg(test)]
mod tests;
