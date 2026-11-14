// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Drawing one view: the camera, the frame, the rasteriser, and the picture's pixels, depth and notes.

use std::time::Duration;

use super::shadows::Shadows;
use super::{Background, Depth, Look, Picture, Prepared, Transform, View};
use crate::area::Triangle;
use crate::texture::Mipped;

/// A triangle as one view draws it: turned into the frame the south-west camera draws, with its texture.
pub(super) struct DrawTri<'a> {
    pub(super) p: [[f32; 3]; 3],
    pub(super) uv: [[f32; 2]; 3],
    texture: u32,
    pub(super) tex: Option<&'a Mipped>,
    /// A chasm's floor: its corners where the scene has them (not turned), to look up its colour.
    chasm: Option<[[f32; 3]; 3]>,
}

/// A view's camera and light, in the turned frame.
pub(super) struct Camera {
    view: View,
    turns: u8,
    /// The corner views' 45-degree axes; `top` draws x and y as they are.
    diagonal: bool,
    elevation_deg: f32,
    sin_e: f32,
    cos_e: f32,
    /// Towards the camera.
    cam: [f32; 3],
    /// Towards the sun.
    light: [f32; 3],
}

const R2: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// A triangle's corners, in metres.
pub(super) type Face = [[f32; 3]; 3];

impl Camera {
    pub(super) fn of(view: View, look: &Look) -> Camera {
        let el = look.elevation_deg.to_radians();
        let (sin_c, cos_c) = (el.sin(), el.cos());
        let corner = [-cos_c * R2, -cos_c * R2, sin_c];
        // (From the camera's side and a little left; the top view is lit as the south-west one.)
        let light = norm([corner[0] - look.light_side, corner[1] + look.light_side, corner[2] + look.light_up]);
        match view {
            View::Top => {
                Camera { view, turns: 0, diagonal: false, elevation_deg: 90.0, sin_e: 1.0, cos_e: 0.0, cam: [0.0, 0.0, 1.0], light }
            }
            _ => Camera {
                view,
                turns: view.turns(),
                diagonal: true,
                elevation_deg: look.elevation_deg,
                sin_e: sin_c,
                cos_e: cos_c,
                cam: corner,
                light,
            },
        }
    }

    fn turn(&self, mut p: [f32; 3]) -> [f32; 3] {
        for _ in 0..self.turns {
            p = quarter(p);
        }
        p
    }

    /// A turned point on the picture, in pixels at `scale` with y up, and its depth (smaller is nearer).
    fn project(&self, p: [f32; 3], scale: f32) -> [f32; 3] {
        let (a, b) = if self.diagonal { ((p[0] - p[1]) * R2, (p[0] + p[1]) * R2) } else { (p[0], p[1]) };
        [a * scale, (b * self.sin_e + p[2] * self.cos_e) * scale, b * self.cos_e - p[2] * self.sin_e]
    }

    /// The turn as a matrix: x' = r[0][0] x + r[0][1] y, y' = r[1][0] x + r[1][1] y.
    fn rotation(&self) -> [[i32; 2]; 2] {
        let x = self.turn([1.0, 0.0, 0.0]);
        let y = self.turn([0.0, 1.0, 0.0]);
        [[x[0] as i32, y[0] as i32], [x[1] as i32, y[1] as i32]]
    }
}

/// A quarter turn clockwise seen from above: (x, y) to (y, -x). The next view's camera then stands where the last one's did.
fn quarter(p: [f32; 3]) -> [f32; 3] {
    [p[1], -p[0], p[2]]
}

/// The most samples (pixels before supersampling is averaged) a view may have: 2^28, about 4 GB of buffers while it is drawn. A
/// 32 by 32 tile area at the default scale and supersampling has about 30 million.
pub(super) const MAX_SAMPLES: usize = 1 << 28;

/// The longest side of a picture in pixels: WebP's limit, 16,383, for PNG too, so a picture is the same size in both formats.
pub(super) const MAX_SIDE_PX: usize = 16_383;

/// A view's frame: the scene's bounds on the picture (in samples, y up) and the picture's size in samples, padding included.
pub(super) struct Frame {
    lo: [f32; 2],
    hi: [f32; 2],
    pub(super) w: usize,
    pub(super) h: usize,
}

/// The frame of `camera`'s view of `prep`: everything in the scene, black void too (the area's corners when there is nothing).
pub(super) fn frame(prep: &Prepared, camera: &Camera) -> Frame {
    let ss = prep.o.quality as usize;
    let scale = prep.o.scale * ss as f32;
    let water = prep.water.iter().flatten();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in prep.triangles.iter().flat_map(|t| t.p.iter()).chain(water) {
        let q = camera.project(camera.turn(*p), scale);
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    if lo[0] > hi[0] {
        let [sx, sy] = prep.size;
        let corners = prep.region.map_or([[0.0, 0.0, 0.0], [sx, 0.0, 0.0], [0.0, sy, 0.0], [sx, sy, 0.0]], |r| r.ground(0.0));
        for p in corners {
            let q = camera.project(camera.turn(p), scale);
            lo = [lo[0].min(q[0]), lo[1].min(q[1])];
            hi = [hi[0].max(q[0]), hi[1].max(q[1])];
        }
    }
    let pad = prep.o.look.padding;
    // (A float too large for usize saturates: the size check then fails.)
    let side = |d: f32| ((d + 2.0 * pad).ceil() as usize).max(ss);
    Frame { lo, hi, w: side(hi[0] - lo[0]), h: side(hi[1] - lo[1]) }
}

/// One view.
pub(super) fn draw(prep: &Prepared, camera: Camera, notes: &mut Vec<String>) -> Picture {
    let (o, triangles, water, chasm, (amb, dif)) = (&prep.o, &prep.triangles, &prep.water, &prep.chasm, prep.lights);
    let look = &o.look;
    let ss = o.quality as usize;
    let scale = o.scale * ss as f32;
    // A tile's faces of black void are not drawn and cast no shadow, so the rooms stand on the background (the game draws them
    // black; on a transparent picture they would be black blocks). They still count for the picture's bounds below, so its size and
    // transform do not depend on them.
    let void: Vec<bool> = prep.textures.iter().map(|t| t.image.as_deref().is_some_and(Mipped::is_black)).collect();
    let is_void = |t: &Triangle| matches!(t.part, crate::area::Part::Tile(_)) && void.get(t.texture as usize).copied().unwrap_or(false);
    let tris: Vec<DrawTri<'_>> = triangles
        .iter()
        .filter(|t| !is_void(t))
        .map(|t| DrawTri {
            p: t.p.map(|p| camera.turn(p)),
            uv: t.uv,
            texture: t.texture,
            tex: prep.textures.get(t.texture as usize).and_then(|x| x.image.as_deref()),
            chasm: t.chasm.then_some(t.p),
        })
        .collect();
    let water: Vec<[[f32; 3]; 3]> = water.iter().map(|t| t.map(|p| camera.turn(p))).collect();
    let (cam, l) = (camera.cam, camera.light);
    // (A region's shadows are cast by what stands around it as well, so a tree 5 m outside it still shades its ground.)
    let casters: Option<Vec<DrawTri<'_>>> = prep.casters.as_ref().filter(|_| o.shadows).map(|c| {
        c.iter()
            .filter(|t| !is_void(t))
            .map(|t| DrawTri {
                p: t.p.map(|p| camera.turn(p)),
                uv: t.uv,
                texture: t.texture,
                tex: prep.textures.get(t.texture as usize).and_then(|x| x.image.as_deref()),
                chasm: None,
            })
            .collect()
    });
    let shadows = o.shadows.then(|| Shadows::new(casters.as_deref().unwrap_or(&tris), l, scale, look, look.alpha_cut));
    drop(casters);
    let bias = shadows.as_ref().map_or(look.shadow_bias, |s| look.shadow_bias.max(2.0 * s.cell));
    let project = |p: [f32; 3]| camera.project(p, scale);
    let Frame { lo, hi, w, h } = frame(prep, &camera);
    let pad = look.padding;
    let sx = |q: &[f32; 3]| q[0] - lo[0] + pad;
    let sy = |q: &[f32; 3]| hi[1] - q[1] + pad;

    let mut depth = vec![f32::MAX; w * h];
    let mut colour = vec![[0u8; 3]; w * h];
    // Coverage: 255 drawn, 0 empty, between for water over nothing.
    let mut alpha = vec![0u8; w * h];
    // Which texture each pixel shows (u32::MAX: none), for the note on textures drawn white.
    let mut owner = vec![u32::MAX; w * h];
    // What the exposure measures: each pixel as its texture draws it, a chasm's floor too (its black), so the chasm's own colour
    // changes nothing else in the picture.
    let mut measured = vec![[0u8; 3]; w * h];
    // With depth pictures: the game height of what each sample shows.
    let mut zs = if o.depth { vec![0f32; w * h] } else { Vec::new() };
    for t in &tris {
        let q = t.p.map(project);
        let (xs, ys) = (q.map(|q| sx(&q)), q.map(|q| sy(&q)));
        // The face's light.
        let mut n = norm(cross(sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0])));
        if dot(n, cam) < 0.0 {
            n = n.map(|c| -c);
        }
        let ndl = dot(n, l).max(0.0);
        let Some(raster) = Raster::new(xs, ys, w, h) else { continue };
        // Which mip level: how many texels one pixel of this triangle covers.
        let lod = match t.tex {
            Some(tx) => {
                let base = &tx.levels[0];
                let uv_area =
                    ((t.uv[1][0] - t.uv[0][0]) * (t.uv[2][1] - t.uv[0][1]) - (t.uv[2][0] - t.uv[0][0]) * (t.uv[1][1] - t.uv[0][1])).abs()
                        * (base.width * base.height) as f32;
                (0.5 * (uv_area / (raster.den.abs() * 0.5)).max(1e-6).log2()).max(0.0)
            }
            None => 0.0,
        };
        raster.each(|x, y, b| {
            let d = b[0] * q[0][2] + b[1] * q[1][2] + b[2] * q[2][2];
            let idx = y * w + x;
            if d >= depth[idx] {
                return;
            }
            let rgba = match t.tex {
                Some(tx) => tx.sample(
                    b[0] * t.uv[0][0] + b[1] * t.uv[1][0] + b[2] * t.uv[2][0],
                    b[0] * t.uv[0][1] + b[1] * t.uv[1][1] + b[2] * t.uv[2][1],
                    lod,
                    false,
                ),
                None => [128.0, 128.0, 128.0, 255.0],
            };
            if rgba[3] < look.alpha_cut {
                return;
            }
            depth[idx] = d;
            // The sun reaches it unless something is between: its position, looked up in the shadow map.
            let lit = match &shadows {
                Some(s) if ndl > 0.0 => s.lit(interpolate(&t.p, b), bias),
                _ => 1.0,
            };
            let light = [0, 1, 2].map(|k| (amb[k] + dif[k] * ndl * lit).min(look.light_max));
            measured[idx] = [0, 1, 2].map(|k| (rgba[k] * light[k]).min(255.0) as u8);
            colour[idx] = match t.chasm {
                Some(p) => {
                    let c = chasm.colour(interpolate(&p, b));
                    [0, 1, 2].map(|k| (c[k] * 255.0 * light[k]).min(255.0) as u8)
                }
                None => measured[idx],
            };
            alpha[idx] = 255;
            owner[idx] = t.texture;
            if !zs.is_empty() {
                zs[idx] = interpolate(&t.p, b)[2];
            }
        });
    }
    // The water over it, once per pixel: its nearest face in front of what was drawn, blended, no depth written.
    if !water.is_empty() {
        let mut wet = vec![f32::MAX; w * h];
        for t in &water {
            let q = t.map(project);
            let Some(raster) = Raster::new(q.map(|q| sx(&q)), q.map(|q| sy(&q)), w, h) else { continue };
            raster.each(|x, y, b| {
                let d = b[0] * q[0][2] + b[1] * q[1][2] + b[2] * q[2][2];
                let idx = y * w + x;
                if d < depth[idx] && d < wet[idx] {
                    wet[idx] = d;
                    if !zs.is_empty() {
                        zs[idx] = interpolate(t, b)[2];
                    }
                }
            });
        }
        if !zs.is_empty() {
            // (What the depth picture takes for the nearest: the water where it is in front.)
            for (d, &w) in depth.iter_mut().zip(&wet) {
                *d = d.min(w);
            }
        }
        let up = dot([0.0, 0.0, 1.0], l).max(0.0);
        let tint = [0, 1, 2].map(|k| look.water[k] * (amb[k] + dif[k] * up).min(1.0) * 255.0);
        let op = look.water_opacity.clamp(0.0, 1.0);
        for idx in (0..w * h).filter(|&i| wet[i] < f32::MAX) {
            if alpha[idx] == 0 {
                colour[idx] = tint.map(|c| c.min(255.0) as u8);
                measured[idx] = colour[idx];
                alpha[idx] = (op * 255.0).round() as u8;
            } else {
                colour[idx] = [0, 1, 2].map(|k| (f32::from(colour[idx][k]) * (1.0 - op) + tint[k] * op) as u8);
                measured[idx] = [0, 1, 2].map(|k| (f32::from(measured[idx][k]) * (1.0 - op) + tint[k] * op) as u8);
            }
        }
    }
    // Dark areas (caves, interiors) are lifted so they can be seen: the brightest 2% of what is drawn is brought up to about what a
    // daylight area has, never darkened and by at most `exposure_max` times.
    let mut gain = 1.0;
    if o.exposure {
        let mut lum: Vec<u16> = (0..w * h)
            .filter(|&i| alpha[i] > 0)
            .map(|i| measured[i])
            .filter(|c| u16::from(c[0]) + u16::from(c[1]) + u16::from(c[2]) > 12)
            .map(|c| (u16::from(c[0]) * 3 + u16::from(c[1]) * 6 + u16::from(c[2])) / 10)
            .collect();
        if lum.len() > 100 {
            lum.sort_unstable();
            let p98 = f32::from(lum[lum.len() * 98 / 100]);
            gain = (look.exposure_target / p98.max(1.0)).clamp(1.0, look.exposure_max.max(1.0));
            if gain > 1.01 {
                // Lifted more where it is dark than where it is bright, so a pale rock in the sun keeps its texture while the shade
                // opens up.
                for c in &mut colour {
                    for v in c.iter_mut() {
                        let f = f32::from(*v);
                        *v = (f * (1.0 + (gain - 1.0) * (1.0 - f / 255.0).powf(look.exposure_lift))).min(255.0) as u8;
                    }
                }
            } else {
                gain = 1.0;
            }
        }
    }
    note_white(&prep.textures, &colour, &alpha, &owner, camera.view, notes);
    // The marks, on top of everything, after the exposure so their colours are the ones asked for.
    for &(at, rgb) in &prep.marks {
        let to_samples = |p: [f32; 3]| {
            let q = project(camera.turn(p));
            [sx(&q), sy(&q)]
        };
        super::region::draw_ring(&mut colour, &mut alpha, w, h, ss, to_samples, at, o.scale, rgb);
    }

    // Averaged down from the supersampled picture: colour weighted by coverage, then over the background.
    let (ow, oh) = (w / ss, h / ss);
    let mut rgba = Vec::with_capacity(ow * oh * 4);
    for y in 0..oh {
        for x in 0..ow {
            let (mut sum, mut cover) = ([0u32; 3], 0u32);
            for dy in 0..ss {
                for dx in 0..ss {
                    let i = (y * ss + dy) * w + x * ss + dx;
                    let a = u32::from(alpha[i]);
                    for k in 0..3 {
                        sum[k] += u32::from(colour[i][k]) * a;
                    }
                    cover += a;
                }
            }
            let n = (ss * ss) as u32;
            match o.background {
                Background::Transparent => {
                    let c = if cover > 0 { sum.map(|s| ((s + cover / 2) / cover) as u8) } else { [0; 3] };
                    rgba.extend_from_slice(&[c[0], c[1], c[2], ((cover + n / 2) / n) as u8]);
                }
                Background::Colour(bg) => {
                    // (Each sample's colour over the background, then their mean.)
                    let c = [0, 1, 2].map(|k| ((sum[k] + u32::from(bg[k]) * (n * 255 - cover)) / (n * 255)) as u8);
                    rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
    }

    // The transform: the quarter turn, the axes, the scale and the picture's offsets.
    let rotation = camera.rotation();
    let (rx, ry) = (rotation[0].map(|v| v as f32), rotation[1].map(|v| v as f32));
    let (a, b) = if camera.diagonal {
        ([(rx[0] - ry[0]) * R2, (rx[1] - ry[1]) * R2], [(rx[0] + ry[0]) * R2, (rx[1] + ry[1]) * R2])
    } else {
        (rx, ry)
    };
    let (offx, offy) = ((pad - lo[0]) / ss as f32, (hi[1] + pad) / ss as f32);
    let s = o.scale;
    let transform = Transform {
        view: camera.view,
        elevation_deg: camera.elevation_deg,
        scale: s,
        width: ow as u32,
        height: oh as u32,
        rotation,
        offset: [offx, offy],
        // (+ 0.0: no -0 in the manifest.)
        matrix: [[a[0] * s, a[1] * s, 0.0, offx], [-b[0] * camera.sin_e * s, -b[1] * camera.sin_e * s, -camera.cos_e * s, offy]]
            .map(|r| r.map(|v| v + 0.0)),
    };
    let depth_picture = (!zs.is_empty()).then(|| depth_down(&zs, &depth, &alpha, w, ss, ow, oh));
    Picture {
        view: camera.view,
        width: ow as u32,
        height: oh as u32,
        rgba,
        transform,
        exposure: gain,
        depth: depth_picture,
        time: Duration::ZERO,
    }
}

/// The depth picture: for each output pixel, the height of its sample nearest the camera among those drawn (`depth` is the
/// distance from the camera, `alpha` what is drawn), scaled to 16 bits over the heights there are.
fn depth_down(zs: &[f32], depth: &[f32], alpha: &[u8], w: usize, ss: usize, ow: usize, oh: usize) -> Depth {
    let mut z = vec![f32::NAN; ow * oh];
    for y in 0..oh {
        for x in 0..ow {
            let mut best = f32::MAX;
            for dy in 0..ss {
                for dx in 0..ss {
                    let i = (y * ss + dy) * w + x * ss + dx;
                    if alpha[i] > 0 && depth[i] < best {
                        best = depth[i];
                        z[y * ow + x] = zs[i];
                    }
                }
            }
        }
    }
    let (lo, hi) = z.iter().filter(|v| v.is_finite()).fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let range = if lo <= hi { [lo, if hi > lo { hi } else { lo + 1.0 }] } else { [0.0, 1.0] };
    // (Drawn pixels are 1 or more, so 0 stays "nothing"; the lowest is a 65535th of the range off, well under a centimetre.)
    let values = z
        .iter()
        .map(
            |&v| if v.is_finite() { (((v - range[0]) / (range[1] - range[0]) * 65535.0).round() as u32).clamp(1, 65535) as u16 } else { 0 },
        )
        .collect();
    Depth { range: range.map(|v| v + 0.0), values }
}

/// A note for each texture that is pale on average and is drawn nearly white over more than 1% of a picture (what was once found by
/// hand: a texture the game tints or lights that shows as a white blot here).
fn note_white(
    textures: &[crate::area::SceneTexture],
    colour: &[[u8; 3]],
    alpha: &[u8],
    owner: &[u32],
    view: View,
    notes: &mut Vec<String>,
) {
    let drawn = alpha.iter().filter(|&&a| a == 255).count();
    if drawn == 0 {
        return;
    }
    let mut white = vec![0usize; textures.len()];
    for i in 0..colour.len() {
        if alpha[i] == 255 && owner[i] != u32::MAX && colour[i].iter().all(|&v| v > 200) {
            white[owner[i] as usize] += 1;
        }
    }
    for (t, &n) in white.iter().enumerate() {
        let Some(image) = textures[t].image.as_deref() else { continue };
        let mean = &image.levels[image.levels.len() - 1].rgba;
        let pale = (u32::from(mean[0]) + u32::from(mean[1]) + u32::from(mean[2])) / 3 > 200;
        let name = &textures[t].name;
        if pale && n * 100 > drawn && !notes.iter().any(|x| x.starts_with(&format!("texture {name} "))) {
            notes.push(format!(
                "texture {name} is drawn nearly white over {:.1}% of the {} picture",
                n as f32 * 100.0 / drawn as f32,
                view.name()
            ));
        }
    }
}

/// A triangle on the picture: the pixels whose centres it covers, each with its barycentric weights.
pub(super) struct Raster {
    xs: [f32; 3],
    ys: [f32; 3],
    den: f32,
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
}

impl Raster {
    pub(super) fn new(xs: [f32; 3], ys: [f32; 3], w: usize, h: usize) -> Option<Raster> {
        let den = (ys[1] - ys[2]) * (xs[0] - xs[2]) + (xs[2] - xs[1]) * (ys[0] - ys[2]);
        if den.abs() < 1e-9 || !den.is_finite() {
            return None;
        }
        let max_x = xs[0].max(xs[1]).max(xs[2]);
        let max_y = ys[0].max(ys[1]).max(ys[2]);
        if max_x < 0.0 || max_y < 0.0 {
            return None;
        }
        let x0 = xs[0].min(xs[1]).min(xs[2]).floor().max(0.0) as usize;
        let y0 = ys[0].min(ys[1]).min(ys[2]).floor().max(0.0) as usize;
        let (x1, y1) = ((max_x.ceil() as usize).min(w - 1), (max_y.ceil() as usize).min(h - 1));
        (x0 <= x1 && y0 <= y1).then_some(Raster { xs, ys, den, x0, x1, y0, y1 })
    }

    pub(super) fn each(&self, mut f: impl FnMut(usize, usize, [f32; 3])) {
        let (xs, ys, den) = (self.xs, self.ys, self.den);
        for y in self.y0..=self.y1 {
            for x in self.x0..=self.x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let l0 = ((ys[1] - ys[2]) * (px - xs[2]) + (xs[2] - xs[1]) * (py - ys[2])) / den;
                let l1 = ((ys[2] - ys[0]) * (px - xs[2]) + (xs[0] - xs[2]) * (py - ys[2])) / den;
                let l2 = 1.0 - l0 - l1;
                if l0 < -0.001 || l1 < -0.001 || l2 < -0.001 {
                    continue;
                }
                f(x, y, [l0, l1, l2]);
            }
        }
    }
}

fn interpolate(p: &[[f32; 3]; 3], b: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|k| b[0] * p[0][k] + b[1] * p[1][k] + b[2] * p[2][k])
}

pub(super) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(super) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(super) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub(super) fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-9);
    a.map(|c| c / l)
}
