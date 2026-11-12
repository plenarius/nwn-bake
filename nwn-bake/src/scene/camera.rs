// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A scene's camera: from a position, or round a target by a turn, a tilt and a distance; perspective or orthographic; framed to
//! what is drawn where the distance (or the scale) is left out.
//!
//! Moonglow's renderer draws in perspective only (`mg_render::Camera`). An orthographic picture is taken through a long lens instead:
//! the camera stands 2,000 m plus 200 times the scene's radius away, with a field of view narrow enough for the picture's height in
//! metres. A thing at the far side of the scene is then drawn under 1% smaller than at the near side, which a picture of a few
//! hundred pixels does not show.

use anyhow::{Result, ensure};
use glam::{Vec3, Vec4};
use serde::{Deserialize, Serialize};

/// Room left round a framed scene, as a share of the picture (5% in all: 2.5% a side).
const MARGIN: f32 = 1.05;

/// The tilt furthest from level: at 90 degrees the camera's up (north, at a turn of -90) and its view are one line, and the
/// renderer's view has no up. At 89.9 the picture is straight down for any purpose.
pub const MAX_PITCH: f32 = 89.9;

/// Where a picture is taken from. Angles in degrees, distances in metres, the game's coordinates.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CameraSpec {
    /// What the camera looks at: the middle of the picture. Left out, the middle of the box round what is drawn.
    pub target: Option<[f32; 3]>,
    /// Where the camera is. With `target`, the camera looks from here at it, and `yaw`, `pitch` and `distance` are not used.
    /// Without, it looks from here along `yaw` and `pitch`. In an orthographic picture only its direction from the target counts.
    pub position: Option<[f32; 3]>,
    /// The camera's turn round the target, counter-clockwise from east: the camera is on that side of it. -90 (the default) is from
    /// the south, looking north.
    pub yaw: f32,
    /// The camera's height above the horizon, -89.9 to 89.9; 30 (the default) is `area-iso`'s corner views. 89.9 looks straight
    /// down, with north up at a yaw of -90.
    pub pitch: f32,
    /// How far the camera is from the target. Left out, as near as shows everything drawn.
    pub distance: Option<f32>,
    /// The picture's field of view, top to bottom, more than 0 and less than 180; 40 (the default) is the other pictures'.
    pub fov: f32,
    /// Orthographic: a thing is drawn the same size however far away it is. `scale` sets the size.
    pub orthographic: bool,
    /// Orthographic only: pixels a metre. Left out, as large as shows everything drawn.
    pub scale: Option<f32>,
}

impl Default for CameraSpec {
    fn default() -> Self {
        CameraSpec { target: None, position: None, yaw: -90.0, pitch: 30.0, distance: None, fov: 40.0, orthographic: false, scale: None }
    }
}

impl CameraSpec {
    /// Checks the numbers.
    pub fn check(&self) -> Result<()> {
        let finite = |p: &Option<[f32; 3]>| p.is_none_or(|p| p.iter().all(|v| v.is_finite()));
        ensure!(finite(&self.target) && finite(&self.position), "camera: a position is three numbers");
        ensure!(self.yaw.is_finite(), "camera: yaw is degrees");
        ensure!(self.pitch.is_finite() && self.pitch.abs() <= MAX_PITCH, "camera: pitch is -{MAX_PITCH} to {MAX_PITCH} degrees");
        ensure!(self.fov.is_finite() && self.fov > 0.0 && self.fov < 180.0, "camera: fov is more than 0 and less than 180 degrees");
        ensure!(self.distance.is_none_or(|d| d.is_finite() && d > 0.0), "camera: distance is more than 0");
        ensure!(self.scale.is_none_or(|s| s.is_finite() && s > 0.0), "camera: scale is pixels a metre, more than 0");
        ensure!(self.scale.is_none() || self.orthographic, "camera: scale is for an orthographic picture");
        if let (Some(p), Some(t)) = (self.position, self.target) {
            ensure!(Vec3::from_array(p).distance(Vec3::from_array(t)) > 1e-3, "camera: the position is the target");
        }
        Ok(())
    }
}

/// The camera a picture was taken with, as the answer gives it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Framing {
    /// Where the camera was.
    pub eye: [f32; 3],
    /// What it looked at: the middle of the picture.
    pub target: [f32; 3],
    /// Its field of view, top to bottom, degrees (an orthographic picture's long lens's).
    pub fov: f32,
    /// Whether the picture is orthographic.
    pub orthographic: bool,
    /// Pixels a metre at the target (everywhere, in an orthographic picture).
    pub scale: f32,
}

/// A camera solved: the renderer's, and what the answer says of it.
#[derive(Clone, Debug)]
pub struct View {
    /// The renderer's camera.
    pub camera: mg_render::Camera,
    /// What the answer says of it.
    pub framing: Framing,
}

impl View {
    /// The pixel position of `p` in a picture `width` by `height`, from its top-left corner; None behind the camera.
    pub fn project(&self, p: Vec3, width: u32, height: u32) -> Option<[f32; 2]> {
        let (w, h) = (width as f32, height as f32);
        let clip = self.camera.projection(w / h) * self.camera.view() * Vec4::new(p.x, p.y, p.z, 1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip / clip.w;
        Some([(ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h])
    }
}

/// The direction from the target to a camera at `yaw` and `pitch` (degrees), as `mg_render::Camera::orbit` puts it.
fn direction(yaw: f32, pitch: f32) -> Vec3 {
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin())
}

/// The camera `spec` asks for, in a picture `width` by `height`, framed (where it is left to) to `points`: the corners of each
/// object's box and the lines' ends. Its target, left out, is the middle of the box round them.
pub fn solve(spec: &CameraSpec, points: &[Vec3], width: u32, height: u32) -> Result<View> {
    spec.check()?;
    ensure!(!points.is_empty(), "camera: nothing to frame");
    let (lo, hi) = points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let (target, back, given) = match (spec.position.map(Vec3::from_array), spec.target.map(Vec3::from_array)) {
        (Some(p), Some(t)) => (t, (p - t).normalize(), Some(p.distance(t))),
        (Some(p), None) => {
            let back = direction(spec.yaw, spec.pitch);
            let d = spec.distance.unwrap_or(10.0);
            (p - back * d, back, Some(d))
        }
        (None, t) => (t.unwrap_or((lo + hi) * 0.5), direction(spec.yaw, spec.pitch), spec.distance),
    };
    // The camera's axes, as `Mat4::look_at_rh` makes them with up along z.
    let right = (-back).cross(Vec3::Z).normalize();
    let up = right.cross(-back);
    let corners: Vec<Vec3> = points.iter().map(|p| *p - target).collect();
    let (w, h) = (width as f32, height as f32);
    if spec.orthographic {
        let across = corners.iter().map(|c| c.dot(right).abs()).fold(1e-3, f32::max);
        let upright = corners.iter().map(|c| c.dot(up).abs()).fold(1e-3, f32::max);
        let scale = spec.scale.unwrap_or_else(|| (w / (2.0 * across)).min(h / (2.0 * upright)) / MARGIN);
        let radius = corners.iter().map(|c| c.length()).fold(1.0, f32::max);
        let distance = 2000.0 + 200.0 * radius;
        let fov = 2.0 * ((h / scale) * 0.5 / distance).atan();
        let camera = mg_render::Camera {
            eye: target + back * distance,
            target,
            fov_y: fov,
            // (Moonglow draws a line 0.5% of its distance nearer the camera, so the ground it lies on does not hide it: 1% is
            // left before the scene.)
            near: ((distance - radius) * 0.99 - 1.0).max(1.0),
            far: distance + radius + 1.0,
        };
        let framing = Framing { eye: camera.eye.to_array(), target: target.to_array(), fov: fov.to_degrees(), orthographic: true, scale };
        return Ok(View { camera, framing });
    }
    let tan_v = (spec.fov.to_radians() * 0.5).tan();
    let tan_h = tan_v * w / h;
    // Each corner is in the picture when the camera is at least this far: its distance across (or up) over the tangent, plus
    // how far it is towards the camera.
    let fitted = corners
        .iter()
        .map(|c| c.dot(back) + (c.dot(right).abs() * MARGIN / tan_h).max(c.dot(up).abs() * MARGIN / tan_v))
        .fold(0.1, f32::max);
    let distance = given.unwrap_or(fitted);
    let depths: Vec<f32> = corners.iter().map(|c| distance - c.dot(back)).collect();
    let nearest = depths.iter().copied().fold(f32::MAX, f32::min);
    let farthest = depths.iter().copied().fold(distance, f32::max);
    let camera = mg_render::Camera {
        eye: target + back * distance,
        target,
        fov_y: spec.fov.to_radians(),
        near: if nearest > 0.02 { (nearest * 0.5).max(0.01) } else { (distance * 0.01).max(0.01) },
        far: farthest * MARGIN + 1.0,
    };
    let framing = Framing {
        eye: camera.eye.to_array(),
        target: target.to_array(),
        fov: spec.fov,
        orthographic: false,
        scale: h / (2.0 * distance * tan_v),
    };
    Ok(View { camera, framing })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: (Vec3, Vec3) = (Vec3::new(-4.0, -1.0, 0.0), Vec3::new(4.0, 1.0, 2.0));

    fn boxed() -> Vec<Vec3> {
        corners().collect()
    }

    fn corners() -> impl Iterator<Item = Vec3> {
        let (lo, hi) = BOX;
        (0..8).map(move |i| {
            Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z })
        })
    }

    /// Every corner of `BOX` lands in the picture, and the furthest out is within 5% of an edge (it is framed, not lost in it).
    fn framed(view: &View, w: u32, h: u32) {
        let mut reach: f32 = 0.0;
        for c in corners() {
            let [x, y] = view.project(c, w, h).unwrap();
            assert!((0.0..=w as f32).contains(&x) && (0.0..=h as f32).contains(&y), "{c} at {x}, {y}");
            reach = reach.max((x / w as f32 - 0.5).abs() * 2.0).max((y / h as f32 - 0.5).abs() * 2.0);
        }
        assert!(reach > 0.9, "the scene fills {reach} of the picture");
    }

    #[test]
    fn a_camera_left_out_frames_the_scene_from_the_south() {
        for (w, h) in [(800, 600), (300, 900), (1024, 256)] {
            let view = solve(&CameraSpec::default(), &boxed(), w, h).unwrap();
            framed(&view, w, h);
            let eye = Vec3::from_array(view.framing.eye);
            assert!(eye.y < -1.0 && eye.x.abs() < 1e-3 && eye.z > 1.0, "from the south and above: {eye}");
            let middle = view.project(Vec3::new(0.0, 0.0, 1.0), w, h).unwrap();
            assert!((middle[0] - w as f32 / 2.0).abs() < 0.01 && (middle[1] - h as f32 / 2.0).abs() < 0.01, "{middle:?}");
        }
    }

    #[test]
    fn an_orthographic_picture_keeps_its_scale_at_every_depth() {
        let spec = CameraSpec { orthographic: true, scale: Some(50.0), pitch: 20.0, ..Default::default() };
        let view = solve(&spec, &boxed(), 640, 480).unwrap();
        assert_eq!(view.framing.scale, 50.0);
        // One metre east, near the camera and far from it: 50 pixels at both, within 1%.
        for y in [-10.0, 0.0, 10.0] {
            let a = view.project(Vec3::new(0.0, y, 0.0), 640, 480).unwrap();
            let b = view.project(Vec3::new(1.0, y, 0.0), 640, 480).unwrap();
            assert!((b[0] - a[0] - 50.0).abs() < 0.5, "at y {y}: {}", b[0] - a[0]);
        }
        let fitted = solve(&CameraSpec { orthographic: true, ..Default::default() }, &boxed(), 640, 480).unwrap();
        framed(&fitted, 640, 480);
        // A line is drawn 0.5% of its distance nearer the camera (Moonglow's line shader): still past the near plane.
        let c = &fitted.camera;
        for p in corners() {
            let depth = (p - c.eye).dot((c.target - c.eye).normalize());
            assert!(depth * 0.995 > c.near && depth < c.far, "{p} at {depth}, near {} far {}", c.near, c.far);
        }
    }

    #[test]
    fn straight_down_has_north_up() {
        let view = solve(&CameraSpec { pitch: MAX_PITCH, orthographic: true, scale: Some(10.0), ..Default::default() }, &boxed(), 200, 200)
            .unwrap();
        let south = view.project(Vec3::new(0.0, -5.0, 0.0), 200, 200).unwrap();
        let north = view.project(Vec3::new(0.0, 5.0, 0.0), 200, 200).unwrap();
        let east = view.project(Vec3::new(5.0, 0.0, 0.0), 200, 200).unwrap();
        assert!(north[1] < south[1] && (north[0] - south[0]).abs() < 0.5, "north is up: {north:?} {south:?}");
        assert!(east[0] > 100.0 + 49.0, "east is right: {east:?}");
    }

    #[test]
    fn a_position_and_a_target_set_the_camera() {
        let spec = CameraSpec { position: Some([10.0, 0.0, 0.0]), target: Some([0.0, 0.0, 0.0]), ..Default::default() };
        let view = solve(&spec, &boxed(), 100, 100).unwrap();
        assert_eq!(view.framing.eye, [10.0, 0.0, 0.0]);
        assert!(view.project(Vec3::new(20.0, 0.0, 0.0), 100, 100).is_none(), "behind the camera");
        let spec = CameraSpec { position: Some([0.0, -10.0, 0.0]), yaw: -90.0, pitch: 0.0, distance: Some(4.0), ..Default::default() };
        let view = solve(&spec, &boxed(), 100, 100).unwrap();
        assert!(Vec3::from_array(view.framing.target).distance(Vec3::new(0.0, -6.0, 0.0)) < 1e-5, "four metres along the view");
        assert!(CameraSpec { position: Some([1.0; 3]), target: Some([1.0; 3]), ..Default::default() }.check().is_err());
        assert!(CameraSpec { scale: Some(2.0), ..Default::default() }.check().is_err(), "a scale needs orthographic");
    }
}
