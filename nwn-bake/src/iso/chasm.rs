// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The chasms' colours: how far each point of a chasm's floor is from the nearest ground.

use super::Look;
use crate::area::Triangle;

/// How far each point of the area's chasm floors is from the nearest ground, on a grid of [`ChasmField::CELL`] metres over the area,
/// and the colour that gives ([`Look::chasm_edge`]). Built once from the scene, so every view draws a chasm alike.
pub(super) struct ChasmField {
    nx: usize,
    ny: usize,
    /// Metres to the nearest cell that is not a chasm's floor (0 where there is no chasm floor). Outside the area counts as chasm,
    /// so a chasm running off the area's edge stays deep there.
    distance: Vec<f32>,
    edge: [f32; 3],
    deep: [f32; 3],
    reach: f32,
    seam: f32,
}

impl ChasmField {
    const CELL: f32 = 0.25;

    pub(super) fn new(size: [f32; 2], triangles: &[Triangle], look: &Look) -> ChasmField {
        let chasms: Vec<&Triangle> = triangles.iter().filter(|t| t.chasm).collect();
        let (nx, ny) = if chasms.is_empty() {
            (0, 0)
        } else {
            ((size[0] / Self::CELL).ceil().max(1.0) as usize, (size[1] / Self::CELL).ceil().max(1.0) as usize)
        };
        let mut inside = vec![false; nx * ny];
        for t in &chasms {
            // The cells whose middles the face covers, seen from above.
            let (xs, ys) = (t.p.map(|p| p[0] / Self::CELL), t.p.map(|p| p[1] / Self::CELL));
            let (x0, x1) = (
                (xs.iter().copied().fold(f32::MAX, f32::min) - 0.5).floor().max(0.0) as usize,
                ((xs.iter().copied().fold(f32::MIN, f32::max) - 0.5).ceil().max(0.0) as usize).min(nx.saturating_sub(1)),
            );
            let (y0, y1) = (
                (ys.iter().copied().fold(f32::MAX, f32::min) - 0.5).floor().max(0.0) as usize,
                ((ys.iter().copied().fold(f32::MIN, f32::max) - 0.5).ceil().max(0.0) as usize).min(ny.saturating_sub(1)),
            );
            let den = (xs[1] - xs[0]) * (ys[2] - ys[0]) - (xs[2] - xs[0]) * (ys[1] - ys[0]);
            if den.abs() < 1e-9 {
                continue;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let a = ((xs[1] - px) * (ys[2] - py) - (xs[2] - px) * (ys[1] - py)) / den;
                    let b = ((xs[2] - px) * (ys[0] - py) - (xs[0] - px) * (ys[2] - py)) / den;
                    if a >= -1e-4 && b >= -1e-4 && 1.0 - a - b >= -1e-4 {
                        inside[y * nx + x] = true;
                    }
                }
            }
        }
        let distance = distance_to_outside(&inside, nx, ny).into_iter().map(|d| d * Self::CELL).collect();
        ChasmField {
            nx,
            ny,
            distance,
            edge: look.chasm_edge,
            deep: look.chasm_deep,
            reach: look.chasm_reach.max(1e-3),
            seam: look.chasm_seam.max(0.0),
        }
    }

    /// Metres from game point `p` (its x and y) to the nearest ground, blended between the four nearest cells.
    fn distance(&self, p: [f32; 3]) -> f32 {
        if self.nx == 0 {
            return 0.0;
        }
        let (x, y) =
            ((p[0] / Self::CELL - 0.5).clamp(0.0, (self.nx - 1) as f32), (p[1] / Self::CELL - 0.5).clamp(0.0, (self.ny - 1) as f32));
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.nx - 1), (y0 + 1).min(self.ny - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let d = |x: usize, y: usize| self.distance[y * self.nx + x];
        (d(x0, y0) * (1.0 - fx) + d(x1, y0) * fx) * (1.0 - fy) + (d(x0, y1) * (1.0 - fx) + d(x1, y1) * fx) * fy
    }

    /// The colour (0..1) of a chasm's floor at game point `p`.
    pub(super) fn colour(&self, p: [f32; 3]) -> [f32; 3] {
        let d = self.distance(p);
        let t = (d / self.reach).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let up = if self.seam > 0.0 { (d / self.seam).clamp(0.0, 1.0) } else { 1.0 };
        [0, 1, 2].map(|k| (self.edge[k] + (self.deep[k] - self.edge[k]) * t) * up)
    }
}

/// For each cell of an `nx` by `ny` grid, how many cells (Euclidean, between middles) it is from the nearest cell that is not
/// `inside`; 0 for those. Cells beyond the grid count as inside. Felzenszwalb and Huttenlocher's exact transform, by rows then
/// columns.
fn distance_to_outside(inside: &[bool], nx: usize, ny: usize) -> Vec<f32> {
    // (Further than any cell can be, and exact in an f64 with room to add to it.)
    let far = (2.0 * (nx + ny + 2) as f64).powi(2);
    let mut d: Vec<f64> = inside.iter().map(|&i| if i { far } else { 0.0 }).collect();
    let mut line = Vec::new();
    for y in 0..ny {
        line.clear();
        line.extend_from_slice(&d[y * nx..(y + 1) * nx]);
        d[y * nx..(y + 1) * nx].copy_from_slice(&squared_1d(&line));
    }
    for x in 0..nx {
        line.clear();
        line.extend((0..ny).map(|y| d[y * nx + x]));
        for (y, v) in squared_1d(&line).into_iter().enumerate() {
            d[y * nx + x] = v;
        }
    }
    d.into_iter().map(|v| v.min(far).sqrt() as f32).collect()
}

/// The 1D squared distance transform of `f`: for each q, the least of (q - p)^2 + f[p] (the lower envelope of parabolas).
fn squared_1d(f: &[f64]) -> Vec<f64> {
    let n = f.len();
    let mut out = vec![0.0; n];
    if n == 0 {
        return out;
    }
    let mut v = vec![0usize; n];
    let mut z = vec![0f64; n + 1];
    let mut k = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let sect = |q: usize, p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * q as f64 - 2.0 * p as f64);
    for q in 1..n {
        let mut s = sect(q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = sect(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let dq = q as f64 - v[k] as f64;
        *o = dq * dq + f[v[k]];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distance_to_the_nearest_ground_is_exact() {
        // A 7 by 5 grid, all chasm but one cell at (1, 1); the grid's edges are not ground.
        let (nx, ny) = (7, 5);
        let mut inside = vec![true; nx * ny];
        inside[nx + 1] = false;
        let d = distance_to_outside(&inside, nx, ny);
        for y in 0..ny {
            for x in 0..nx {
                let want = ((x as f32 - 1.0).powi(2) + (y as f32 - 1.0).powi(2)).sqrt();
                assert!((d[y * nx + x] - want).abs() < 1e-4, "({x}, {y}): {} for {want}", d[y * nx + x]);
            }
        }
    }
}
