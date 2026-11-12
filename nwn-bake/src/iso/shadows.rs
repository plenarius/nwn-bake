// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The shadow map: what the sun sees.

use super::Look;
use super::draw::{DrawTri, Raster, cross, dot, norm, sub};

/// What the sun sees: for each cell of a flat picture taken along the light, how near the light the nearest surface is.
pub(super) struct Shadows {
    depth: Vec<f32>,
    size: usize,
    /// A cell's side, metres.
    pub(super) cell: f32,
    centre: [f32; 3],
    radius: f32,
    u: [f32; 3],
    v: [f32; 3],
    w: [f32; 3],
}

impl Shadows {
    /// The shadow map of `tris` for a picture drawn at `scale` pixels a metre (supersampled), sized as [`Look::shadow_size`] says.
    pub(super) fn new(tris: &[DrawTri<'_>], light: [f32; 3], scale: f32, look: &Look, alpha_cut: f32) -> Shadows {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in tris.iter().flat_map(|t| t.p.iter()) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if lo[0] > hi[0] {
            (lo, hi) = ([0.0; 3], [0.0; 3]);
        }
        let centre = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
        let radius = (dot(sub(hi, lo), sub(hi, lo)).sqrt() / 2.0).max(1.0);
        let size = shadow_cells(radius, scale, look);
        let w = norm(light);
        let u = norm(cross([0.0, 0.0, 1.0], w));
        let v = cross(w, u);
        let mut s = Shadows { depth: vec![f32::MIN; size * size], size, cell: 2.0 * radius / size as f32, centre, radius, u, v, w };
        for t in tris {
            // (A cut-out's clear parts let the light through: sampled at its own detail.)
            let cut = t.tex.filter(|x| x.cutout);
            let q = t.p.map(|p| s.to_light(p));
            let Some(raster) = Raster::new(q.map(|q| q[0]), q.map(|q| q[1]), size, size) else { continue };
            raster.each(|x, y, b| {
                if b.iter().any(|&c| c < 0.0) {
                    return;
                }
                let d = b[0] * q[0][2] + b[1] * q[1][2] + b[2] * q[2][2];
                let idx = y * size + x;
                if d <= s.depth[idx] {
                    return;
                }
                if let Some(tx) = cut {
                    let (u, v) = (
                        b[0] * t.uv[0][0] + b[1] * t.uv[1][0] + b[2] * t.uv[2][0],
                        b[0] * t.uv[0][1] + b[1] * t.uv[1][1] + b[2] * t.uv[2][1],
                    );
                    if tx.sample(u, v, 1.0, false)[3] < alpha_cut {
                        return;
                    }
                }
                s.depth[idx] = d;
            });
        }
        s
    }

    /// A point in the shadow map: (x, y in cells, depth towards the light).
    fn to_light(&self, p: [f32; 3]) -> [f32; 3] {
        let d = sub(p, self.centre);
        let half = self.size as f32 / 2.0;
        [half + dot(d, self.u) / self.radius * half, half + dot(d, self.v) / self.radius * half, dot(d, self.w)]
    }

    /// How much of the sun reaches the point (0 in shadow to 1 lit), averaged over the neighbouring cells so edges are soft.
    pub(super) fn lit(&self, p: [f32; 3], bias: f32) -> f32 {
        let q = self.to_light(p);
        let (cx, cy) = (q[0].round() as i64, q[1].round() as i64);
        let mut lit = 0.0;
        for dy in -1..=1i64 {
            for dx in -1..=1i64 {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0
                    || y < 0
                    || x >= self.size as i64
                    || y >= self.size as i64
                    || q[2] >= self.depth[y as usize * self.size + x as usize] - bias
                {
                    lit += 1.0;
                }
            }
        }
        lit / 9.0
    }
}

/// The shadow map's cells along a side for a scene `radius` metres about its middle drawn at `scale` pixels a metre: one cell a
/// pixel across the map's width (twice the radius), rounded up to a power of two, between [`Look::shadow_min_size`] and
/// [`Look::shadow_size`].
fn shadow_cells(radius: f32, scale: f32, look: &Look) -> usize {
    let max = look.shadow_size.clamp(16, 16384) as usize;
    let min = (look.shadow_min_size.clamp(16, 16384) as usize).min(max);
    let want = (2.0 * radius * scale).ceil().clamp(1.0, 1e6) as usize;
    want.next_power_of_two().clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shadow map is sized to the picture: one cell a drawn pixel across, a power of two, 1,024 to 4,096 cells by default.
    #[test]
    fn the_shadow_map_has_a_cell_for_each_pixel_drawn() {
        let look = Look::default();
        // An 8 by 8 area (about 57 m about its middle) at 8 px a metre, supersampled twice: 1,818 pixels across, 2,048 cells.
        assert_eq!(shadow_cells(56.8, 16.0, &look), 2048);
        // A small room: at least 1,024; a 32 by 32 area: at most 4,096.
        assert_eq!(shadow_cells(10.0, 16.0, &look), 1024);
        assert_eq!(shadow_cells(230.0, 16.0, &look), 4096);
        // A look may ask for fewer or more.
        assert_eq!(shadow_cells(230.0, 16.0, &Look { shadow_size: 1024, ..look.clone() }), 1024);
        assert_eq!(shadow_cells(10.0, 16.0, &Look { shadow_min_size: 256, ..look }), 512);
    }
}
