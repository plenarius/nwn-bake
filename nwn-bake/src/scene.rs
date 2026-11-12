// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A picture of several objects at one camera: placeables, bare models, creatures and items, each placed, turned and scaled, with
//! lines, axis markers and points projected to pixels. What a side-by-side comparison needs (two models at one camera and scale, a
//! chess board, a lineup of creatures), where the other pictures ([`crate::creature::thumbnail`]) are one object each, framed by
//! itself at a fixed pitch.
//!
//! The request is a [`Spec`], read from JSON (`docs/scene.md`, `docs/scene.schema.json`). Drawn with Moonglow's renderer, as the
//! other pictures are: each object as [`crate::creature`] composes it (a creature dressed by [`crate::items`]), its particles run
//! for a while first. Coordinates are the game's: metres, x east, y north, z up.

mod camera;
mod objects;
mod picture;

use std::path::Path;

use anyhow::{Context, Result, ensure};
use glam::{Mat4, Vec3};
use mg_rules::GameData;
use serde::{Deserialize, Serialize};

pub use camera::{CameraSpec, Framing};
pub use objects::{Kind, Object, Part, Parts, parts};
pub use picture::{Axes, Line};

use crate::creature::Gpu;
use crate::items::{Items, MissingItem};

/// The largest side of a picture, in pixels.
pub const MAX_SIDE_PX: u32 = 8192;

/// A picture to draw: the objects, the camera, what is drawn over them and the light.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// The picture's width in pixels, 1 to [`MAX_SIDE_PX`] (1024 when left out).
    pub width: Option<u32>,
    /// The picture's height in pixels, 1 to [`MAX_SIDE_PX`] (768 when left out).
    pub height: Option<u32>,
    /// What is drawn, each placed in the game's coordinates.
    #[serde(default)]
    pub objects: Vec<Object>,
    /// Where the picture is taken from; left out, from the south at 30 degrees up, framed by what is drawn.
    #[serde(default)]
    pub camera: CameraSpec,
    /// Straight lines, a pixel wide, hidden where an object is in front of them.
    #[serde(default)]
    pub lines: Vec<Line>,
    /// Axis markers: three lines from a point, east red, north green, up blue.
    #[serde(default)]
    pub axes: Vec<Axes>,
    /// Points whose pixel positions the answer gives ([`Drawn::points`]); nothing is drawn for them.
    #[serde(default)]
    pub points: Vec<[f32; 3]>,
    /// `"day"` (the light of the other pictures, the default) or `{"ambient": "#rrggbb", "diffuse": "#rrggbb", "toward": [x, y,
    /// z]}`: an area's sun, its colours as the `.are` gives them (`SunAmbientColor`, `SunDiffuseColor`).
    pub light: Option<serde_json::Value>,
    /// The sun's shadows, cast by every object onto the placeables and models (the ground of a scene: a board, a floor). Off when
    /// left out.
    #[serde(default)]
    pub shadows: bool,
    /// `"transparent"` or a colour, `"#rrggbb"`, as `area-iso`'s `--background` takes it; the other pictures' dark grey
    /// (`#1a1c24`) when left out.
    pub background: Option<String>,
}

/// What a drawn object left out, and where it was.
#[derive(Clone, Debug, Serialize)]
pub struct ObjectReport {
    /// What it is: `placeable`, `model`, `creature` or `item`.
    pub kind: &'static str,
    /// The box round it as drawn, in the game's coordinates: lowest corner, then highest.
    pub bounds: [[f32; 3]; 2],
    /// The models it names that the game does not have or cannot read (drawn without): a name, or `name: why`.
    pub missing_models: Vec<String>,
    /// The equipped items a creature blueprint names that were not found (drawn without).
    pub missing_items: Vec<MissingItem>,
}

/// What [`render`] drew.
#[derive(Clone, Debug, Serialize)]
pub struct Drawn {
    /// The picture's width in pixels.
    pub width: u32,
    /// The picture's height in pixels.
    pub height: u32,
    /// The camera it was taken with ([`Framing`]).
    pub camera: Framing,
    /// The box round everything drawn (objects, lines and axes), lowest corner then highest.
    pub bounds: [[f32; 3]; 2],
    /// The pixel position of each of [`Spec::points`], from the picture's top-left corner, in their order; None for a point behind
    /// the camera.
    pub points: Vec<Option<[f32; 2]>>,
    /// One for each of [`Spec::objects`], in their order.
    pub objects: Vec<ObjectReport>,
}

impl Spec {
    /// The picture's width and height, checked.
    pub fn size(&self) -> Result<(u32, u32)> {
        let (w, h) = (self.width.unwrap_or(1024), self.height.unwrap_or(768));
        ensure!((1..=MAX_SIDE_PX).contains(&w) && (1..=MAX_SIDE_PX).contains(&h), "the picture is 1 to {MAX_SIDE_PX} pixels a side");
        Ok((w, h))
    }

    /// The background: transparent, or a colour.
    pub fn background(&self) -> Result<crate::iso::Background> {
        match &self.background {
            None => Ok(crate::iso::Background::Colour([0x1a, 0x1c, 0x24])),
            Some(b) => crate::iso::parse_background(b).context("background"),
        }
    }

    /// The light: [`Spec::light`] read.
    pub fn area_light(&self) -> Result<mg_render::AreaLight> {
        picture::light(self.light.as_ref())
    }

    /// Checks everything that can be checked without the game: the size, the background, the light, each object's kind, the
    /// camera, the numbers. [`render`] does this first.
    pub fn check(&self) -> Result<()> {
        self.size()?;
        self.background()?;
        self.area_light()?;
        for (i, o) in self.objects.iter().enumerate() {
            o.check().with_context(|| format!("object {i}"))?;
        }
        self.camera.check()?;
        let finite = |p: &[f32; 3]| p.iter().all(|v| v.is_finite());
        picture::check(&self.lines)?;
        ensure!(self.lines.iter().all(|l| finite(&l.from) && finite(&l.to)), "a line's ends are numbers");
        ensure!(
            self.axes.iter().all(|a| finite(&a.at) && a.length.is_finite() && a.length > 0.0),
            "an axis marker's length is more than 0"
        );
        ensure!(self.points.iter().all(finite), "a point is three numbers");
        ensure!(!self.objects.is_empty() || !self.lines.is_empty() || !self.axes.is_empty(), "nothing to draw: no objects, lines or axes");
        Ok(())
    }
}

/// The eight corners of the box `lo`..`hi`, moved by `m`.
fn moved_corners(lo: Vec3, hi: Vec3, m: Mat4) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        m.transform_point3(Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        ))
    })
}

/// The box round `points`.
fn round(points: &[Vec3]) -> (Vec3, Vec3) {
    points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
}

/// Draws `spec` to the PNG `out`, a creature blueprint's equipped items read from `items` (a strict lookup fails on the first
/// creature missing one). Says what each object left out, where the camera was and where the points land.
pub fn render(game: &GameData, gpu: &Gpu, spec: &Spec, items: &Items, out: &Path) -> Result<Drawn> {
    spec.check()?;
    let (width, height) = spec.size()?;
    let loader = crate::creature::Loader::new(game);
    let load = |n: &str| loader.load(n);
    let mut placed = Vec::new();
    let mut reports = Vec::new();
    // (The camera is fitted to each object's own box, not to the box round them all: a board's far corners high up are no part
    // of a scene, and would leave the picture's top empty.)
    let mut frame = Vec::new();
    for (i, o) in spec.objects.iter().enumerate() {
        let made = objects::prepare(game, o, items).with_context(|| format!("object {i} ({})", o.describe()))?;
        let composed = mg_preview::compose::Composed::new(gpu, &made.preview, &load)
            .with_context(|| format!("object {i} ({}): no model to draw", o.describe()))?;
        let m = o.matrix();
        let (a, b) = if composed.has_meshes() { composed.bounds_in(Some(&made.anim), o.t) } else { (Vec3::ZERO, Vec3::ZERO) };
        let corners = moved_corners(a, b, m);
        frame.extend(corners);
        let (a, b) = round(&corners);
        reports.push(ObjectReport {
            kind: made.kind.name(),
            bounds: [a.to_array(), b.to_array()],
            missing_models: loader.missing(&composed.missing),
            missing_items: made.missing_items,
        });
        placed.push((composed, made.anim, o.t, m, made.kind));
    }
    let lines = picture::lines(&spec.lines, &spec.axes);
    frame.extend(lines.iter().flat_map(|l| [l.from, l.to]));
    let (lo, hi) = round(&frame);
    let view = camera::solve(&spec.camera, &frame, width, height)?;
    let mut instances = Vec::new();
    let mut particles = Vec::new();
    for (composed, anim, t, m, kind) in &placed {
        let ground = spec.shadows && matches!(kind, Kind::Placeable(_) | Kind::Model(_));
        instances.extend(composed.instances(Some(anim), *t, *m).into_iter().map(|mut inst| {
            inst.receives_shadows = ground;
            inst
        }));
        particles.extend(crate::creature::particles_now(composed, anim, *t, &view.camera, *m).unwrap_or_default());
    }
    let scene = mg_render::Scene {
        instances,
        area: spec.area_light()?,
        particles,
        lines,
        // (The shadows' fade up the screen ends at the area's fog distance; a scene has no fog, so none.)
        shadows: spec.shadows.then_some(mg_render::SunShadows { opacity: picture::SHADOW_OPACITY, fade_limit: 1.0e6 }),
        ..Default::default()
    };
    let rgba = picture::draw(gpu, &game.resman, scene, &view.camera, width, height, spec.background()?);
    crate::creature::write_rgba_png(out, width, height, &rgba)?;
    Ok(Drawn {
        width,
        height,
        points: spec.points.iter().map(|p| view.project(Vec3::from_array(*p), width, height)).collect(),
        camera: view.framing,
        bounds: [lo.to_array(), hi.to_array()],
        objects: reports,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: &str) -> Spec {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_spec_is_checked_before_the_game_is_read() {
        assert!(spec(r#"{"objects": [{"placeable": 3}]}"#).check().is_ok());
        assert!(spec(r#"{"lines": [{"from": [0, 0, 0], "to": [1, 0, 0]}]}"#).check().is_ok(), "lines alone are a picture");
        for bad in [
            r#"{}"#,
            r#"{"objects": [{"placeable": 3, "model": "box"}]}"#,
            r#"{"objects": [{}]}"#,
            r#"{"objects": [{"placeable": 3}], "width": 0}"#,
            r#"{"objects": [{"placeable": 3}], "height": 9000}"#,
            r##"{"objects": [{"placeable": 3}], "background": "#12345"}"##,
            r#"{"objects": [{"placeable": 3}], "light": "night"}"#,
            r#"{"objects": [{"placeable": 3, "scale": 0}]}"#,
            r#"{"objects": [{"placeable": 3}], "camera": {"fov": 180}}"#,
            r#"{"objects": [{"placeable": 3}], "axes": [{"at": [0, 0, 0], "length": -1}]}"#,
        ] {
            assert!(spec(bad).check().is_err(), "{bad} passed");
        }
        assert!(serde_json::from_str::<Spec>(r#"{"objects": [], "zoom": 2}"#).is_err(), "an unknown field is an error");
    }

    #[test]
    fn the_defaults_are_the_other_pictures() {
        let s = spec(r#"{"objects": [{"placeable": 3}]}"#);
        assert_eq!(s.size().unwrap(), (1024, 768));
        assert_eq!(s.background().unwrap(), crate::iso::Background::Colour([0x1a, 0x1c, 0x24]));
        assert_eq!(s.area_light().unwrap(), mg_render::AreaLight::default());
        assert_eq!(spec(r#"{"background": "transparent"}"#).background().unwrap(), crate::iso::Background::Transparent);
    }

    #[test]
    fn a_box_moved_stays_round_its_corners() {
        let m = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)) * Mat4::from_rotation_z(90f32.to_radians());
        let (a, b) = round(&moved_corners(Vec3::new(-1.0, -2.0, 0.0), Vec3::new(1.0, 2.0, 3.0), m));
        assert!((a - Vec3::new(8.0, -1.0, 0.0)).length() < 1e-5, "{a}");
        assert!((b - Vec3::new(12.0, 1.0, 3.0)).length() < 1e-5, "{b}");
    }
}
