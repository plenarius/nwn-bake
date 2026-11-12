// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! What a scene draws besides its objects (lines and axis markers), its light, and the picture itself: on a colour, or
//! transparent.
//!
//! Moonglow's renderer clears to an opaque colour. A transparent picture is drawn twice, on black and on white, and each pixel's
//! alpha is how much the two differ: where nothing is drawn, they differ by all of white; where something opaque is, not at all. The
//! colour is then the one on black divided by the alpha. Both pictures are drawn alike (the particles' random numbers start from
//! the same seed), so only the background differs.

use anyhow::{Result, bail};
use glam::Vec3;
use serde::Deserialize;

use crate::iso::Background;

/// How dark the sun's shadows are, 0 to 1: the Shadow Opacity the game's own areas mostly have (60 of 100).
pub const SHADOW_OPACITY: f32 = 0.6;

/// A straight line, a pixel wide.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    /// One end, in the game's coordinates.
    pub from: [f32; 3],
    /// The other end.
    pub to: [f32; 3],
    /// `#rrggbb` or `#rrggbbaa` (white when left out).
    pub colour: Option<String>,
}

/// An axis marker: lines from `at`, `length` metres east (red), north (green) and up (blue).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Axes {
    /// Where the three lines start.
    pub at: [f32; 3],
    /// How long each is, metres (1 when left out).
    #[serde(default = "one")]
    pub length: f32,
}

fn one() -> f32 {
    1.0
}

/// `#rrggbb` or `#rrggbbaa` as RGBA from 0 to 1.
pub fn colour(s: &str) -> Result<[f32; 4]> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("a colour is #rrggbb or #rrggbbaa, not {s:?}");
    }
    let v = |i: usize| hex.get(i..i + 2).map_or(1.0, |h| f32::from(u8::from_str_radix(h, 16).unwrap_or(255)) / 255.0);
    Ok([v(0), v(2), v(4), v(6)])
}

/// The lines and the axis markers' lines, as the renderer draws them. A colour that does not read is white ([`check`] says so
/// first).
pub fn lines(lines: &[Line], axes: &[Axes]) -> Vec<mg_render::Line> {
    let mut out: Vec<mg_render::Line> = lines
        .iter()
        .map(|l| mg_render::Line {
            from: Vec3::from_array(l.from),
            to: Vec3::from_array(l.to),
            color: l.colour.as_deref().and_then(|c| colour(c).ok()).unwrap_or([1.0; 4]),
        })
        .collect();
    for a in axes {
        let at = Vec3::from_array(a.at);
        for (dir, color) in [(Vec3::X, [1.0, 0.2, 0.2, 1.0]), (Vec3::Y, [0.2, 1.0, 0.2, 1.0]), (Vec3::Z, [0.3, 0.5, 1.0, 1.0])] {
            out.push(mg_render::Line { from: at, to: at + dir * a.length, color });
        }
    }
    out
}

/// Checks the lines' colours.
pub fn check(lines: &[Line]) -> Result<()> {
    for l in lines {
        if let Some(c) = &l.colour {
            colour(c)?;
        }
    }
    Ok(())
}

/// The light a scene asks for: `"day"` or nothing for the other pictures' daylight (Moonglow's preview light), or the sun's
/// colours (gamma, as an `.are` gives them) and where it is (`toward`, the direction to it; the game's default sun's when left
/// out).
pub fn light(spec: Option<&serde_json::Value>) -> Result<mg_render::AreaLight> {
    use serde_json::Value;
    let day = mg_render::AreaLight::default();
    let Some(v) = spec else { return Ok(day) };
    match v {
        Value::String(s) if s == "day" => Ok(day),
        Value::Object(o) => {
            let mut light = day;
            for (k, v) in o {
                match (k.as_str(), v) {
                    ("ambient", Value::String(c)) => light.ambient = linear(colour(c)?),
                    ("diffuse", Value::String(c)) => light.diffuse = linear(colour(c)?),
                    ("toward", Value::Array(a)) if a.len() == 3 => {
                        let d = Vec3::from_array([0, 1, 2].map(|i| a[i].as_f64().unwrap_or(f64::NAN) as f32));
                        if !d.is_finite() || d.length() < 1e-6 {
                            bail!("light: toward is a direction, three numbers not all 0");
                        }
                        light.direction = d.normalize();
                    }
                    _ => bail!("light: {k} is not ambient (#rrggbb), diffuse (#rrggbb) or toward ([x, y, z])"),
                }
            }
            Ok(light)
        }
        _ => bail!("light is \"day\" or {{\"ambient\", \"diffuse\", \"toward\"}}"),
    }
}

/// A gamma colour as the renderer's light takes it (linear), as `mg_render::AreaLight::from_are` reads an `.are`'s.
fn linear(c: [f32; 4]) -> Vec3 {
    Vec3::new(c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2))
}

/// The scene drawn by `camera`, `width` by `height`, RGBA rows top first, on `background`.
pub fn draw(
    gpu: &crate::creature::Gpu,
    assets: &dyn mg_render::Assets,
    mut scene: mg_render::Scene,
    camera: &mg_render::Camera,
    width: u32,
    height: u32,
    background: Background,
) -> Vec<u8> {
    let mut renderer = mg_render::Renderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm, 4);
    match background {
        Background::Colour(c) => {
            scene.background = c.map(|v| f32::from(v) / 255.0);
            renderer.render_image(gpu, assets, &scene, camera, width, height).data
        }
        Background::Transparent => {
            scene.background = [0.0; 3];
            let black = renderer.render_image(gpu, assets, &scene, camera, width, height).data;
            scene.background = [1.0; 3];
            let white = renderer.render_image(gpu, assets, &scene, camera, width, height).data;
            matte(&black, &white)
        }
    }
}

/// A transparent picture from the same picture drawn on black and on white: each pixel's alpha is 1 less how much white shows
/// through (the most of its three channels), its colour the one on black over the alpha.
pub fn matte(black: &[u8], white: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; black.len()];
    for ((o, b), w) in out.as_chunks_mut::<4>().0.iter_mut().zip(black.as_chunks::<4>().0).zip(white.as_chunks::<4>().0) {
        let through = (0..3).map(|c| i32::from(w[c]) - i32::from(b[c])).max().unwrap_or(0).clamp(0, 255);
        let alpha = 255 - through;
        if alpha == 0 {
            continue;
        }
        for c in 0..3 {
            o[c] = ((i32::from(b[c]) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
        o[3] = alpha as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_read_with_and_without_alpha() {
        assert_eq!(colour("#ff0000").unwrap(), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(colour("00ff0080").unwrap()[3], 128.0 / 255.0);
        assert!(colour("#ff00").is_err());
        assert!(colour("#gg0000").is_err());
    }

    #[test]
    fn axes_are_three_coloured_lines() {
        let l = lines(&[Line { from: [0.0; 3], to: [1.0; 3], colour: None }], &[Axes { at: [1.0, 2.0, 3.0], length: 2.0 }]);
        assert_eq!(l.len(), 4);
        assert_eq!(l[0].color, [1.0; 4], "white when left out");
        assert_eq!(l[1].to, Vec3::new(3.0, 2.0, 3.0), "east, red");
        assert!(l[1].color[0] > l[1].color[1]);
        assert_eq!(l[3].to, Vec3::new(1.0, 2.0, 5.0), "up, blue");
    }

    #[test]
    fn the_light_is_day_or_the_suns_colours() {
        assert_eq!(light(None).unwrap(), mg_render::AreaLight::default());
        let v = serde_json::json!({"ambient": "#000000", "diffuse": "#ffffff", "toward": [0, 0, 2]});
        let l = light(Some(&v)).unwrap();
        assert_eq!((l.ambient, l.diffuse, l.direction), (Vec3::ZERO, Vec3::ONE, Vec3::Z));
        for bad in [serde_json::json!("area"), serde_json::json!({"sun": "#ffffff"}), serde_json::json!({"toward": [0, 0, 0]})] {
            assert!(light(Some(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_matte_gives_back_the_colour_and_its_alpha() {
        // Nothing drawn, half a red pixel, and an opaque grey one.
        let black = [0, 0, 0, 255, 100, 0, 0, 255, 90, 90, 90, 255];
        let white = [255, 255, 255, 255, 227, 127, 127, 255, 90, 90, 90, 255];
        let m = matte(&black, &white);
        assert_eq!(m[3], 0, "nothing drawn is transparent");
        assert_eq!(&m[4..8], &[199, 0, 0, 128], "100 on black at alpha 128");
        assert_eq!(&m[8..12], &[90, 90, 90, 255]);
    }
}
