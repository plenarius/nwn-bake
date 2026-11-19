// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The roof cut that follows the ground ([`super::Roof::Ground`], `docs/area-iso.md`, "Following the ground"): everything more than
//! a number of metres over the ground under it is cut away, so raised ground (a plateau, a hill) keeps its own ground and loses its
//! own trees and roofs, where one flat cut for the whole area would take the plateau's ground with them.
//!
//! The ground is the top of the tiles' walkmeshes, walked or not ([`FloorProbe::walkmesh_top`]): it follows a cliff's slope and a
//! plateau's or a rock's top as the game's walkmesh has them, and stays at a tree's foot, whose walkmesh is its footprint on the
//! ground. A tile's building can have a walkmesh block as high as its eaves: a raised patch of walkmesh that fits in
//! [`super::Look::roof_ground_width`] and does not reach the area's edge is taken for one, and the probe's ground counts there instead.
//! Where no walkmesh face is (black void, a hole in a tile), the probe's ground ([`FloorProbe::heights`]) stands in for it too: the
//! walkable faces around, filled in across the gap within the tile, else the tile's own height. Each sample then takes the highest
//! ground within [`super::Look::roof_ground_reach`] of it, so a cliff's face, drawn a little outside its walkmesh, is not cut where
//! it hangs over the ground below.
//!
//! The test is made where a triangle is drawn, at each sample of the picture and of the shadow map, against the cut's height there
//! (bilinear between the probe's samples): a triangle across the line is cut along it, not dropped whole. A triangle wholly below the
//! lowest the line goes over it is drawn without the test. Where nothing is left in a sample of the picture (raised ground whose drawn
//! mesh stands more than the cut over its walkmesh, a building with no floor under its roof), the lowest of what was cut away there
//! is drawn instead, within the area, so the cut leaves no hole where the picture without it has none. That is done in the top view
//! only: a corner view's empty samples are mostly the background past the area's edge, which must stay empty.

use crate::area::FloorProbe;
use crate::area::floor::SPACING;

/// The height, at each sample of the floor probe, above which everything is cut away.
pub(super) struct GroundCut {
    nx: usize,
    ny: usize,
    /// Rows from the south, as the probe's.
    at: Vec<f32>,
}

impl GroundCut {
    /// The cut `over` metres over the ground of `probe`: a raised patch of walkmesh that fits in a square `width` metres a side and
    /// does not reach the area's edge is no ground ([`raised_patches`]), and each sample takes the highest ground within `reach`
    /// metres (along x and y). None for a probe without samples.
    pub(super) fn new(probe: &FloorProbe, over: f32, width: f32, reach: f32) -> Option<GroundCut> {
        let (nx, ny) = (probe.nx, probe.ny);
        if nx == 0 || ny == 0 || probe.heights.len() < nx * ny {
            return None;
        }
        let top: Vec<f32> = (0..nx * ny).map(|k| probe.walkmesh_top.get(k).copied().flatten().unwrap_or(probe.heights[k])).collect();
        let small = raised_patches(&top, &probe.heights, nx, ny, (width.max(0.0) / SPACING).round() as usize);
        let ground: Vec<f32> = (0..nx * ny).map(|k| if small[k] { probe.heights[k] } else { top[k].max(probe.heights[k]) }).collect();
        let r = (reach.max(0.0) / SPACING).round() as usize;
        let at = square(&ground, nx, ny, r, f32::max).into_iter().map(|g| g + over).collect();
        Some(GroundCut { nx, ny, at })
    }

    /// The sample index along x (or y) of `v` metres, as a float clamped to the probe.
    fn sample(v: f32, n: usize) -> f32 {
        (v / SPACING).clamp(0.0, (n - 1) as f32)
    }

    /// The height above which everything is cut at (`x`, `y`) metres: bilinear between the samples around it (the nearest edge's
    /// outside the area).
    pub(super) fn at(&self, x: f32, y: f32) -> f32 {
        let (fx, fy) = (Self::sample(x, self.nx), Self::sample(y, self.ny));
        let (i, j) = ((fx as usize).min(self.nx.saturating_sub(2)), (fy as usize).min(self.ny.saturating_sub(2)));
        let (i1, j1) = ((i + 1).min(self.nx - 1), (j + 1).min(self.ny - 1));
        let (tx, ty) = ((fx - i as f32).clamp(0.0, 1.0), (fy - j as f32).clamp(0.0, 1.0));
        let z = |i: usize, j: usize| self.at[j * self.nx + i];
        let south = z(i, j) * (1.0 - tx) + z(i1, j) * tx;
        let north = z(i, j1) * (1.0 - tx) + z(i1, j1) * tx;
        south * (1.0 - ty) + north * ty
    }

    /// Whether the point `p` (metres) is kept: not above the cut there.
    pub(super) fn keeps(&self, p: [f32; 3]) -> bool {
        p[2] <= self.at(p[0], p[1])
    }

    /// The lowest and highest the cut goes over the rectangle about the triangle `p` (along x and y): every sample the bilinear
    /// [`GroundCut::at`] reads there.
    pub(super) fn range(&self, p: &[[f32; 3]; 3]) -> (f32, f32) {
        let (x0, x1) = (p[0][0].min(p[1][0]).min(p[2][0]), p[0][0].max(p[1][0]).max(p[2][0]));
        let (y0, y1) = (p[0][1].min(p[1][1]).min(p[2][1]), p[0][1].max(p[1][1]).max(p[2][1]));
        let (i0, i1) = (Self::sample(x0, self.nx).floor() as usize, (Self::sample(x1, self.nx).ceil() as usize).min(self.nx - 1));
        let (j0, j1) = (Self::sample(y0, self.ny).floor() as usize, (Self::sample(y1, self.ny).ceil() as usize).min(self.ny - 1));
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for j in j0..=j1 {
            for &v in &self.at[j * self.nx + i0..=j * self.nx + i1] {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        (lo, hi)
    }

    /// Whether the triangle `p` may have a part above the cut, so it is tested where it is drawn: its highest corner is over the
    /// lowest the cut goes under it.
    pub(super) fn crosses(&self, p: &[[f32; 3]; 3]) -> bool {
        p[0][2].max(p[1][2]).max(p[2][2]) > self.range(p).0
    }
}

/// A raised sample is one whose walkmesh top is more than this (metres) over the probe's ground (the walkable faces, filled in).
const RAISED: f32 = 1.0;

/// Which samples lie in a small raised patch of walkmesh: samples whose walkmesh top (`top`) is more than [`RAISED`] over the
/// probe's ground (`ground`), joined to their neighbours along x and y, in a patch whose samples fit in `side` samples each way and
/// none of which is on the grid's edge. A tile's building can have a walkmesh as high as its eaves, a block that does not walk, and
/// is such a patch; a plateau or a hill's top is wider, or runs on past the area's edge.
fn raised_patches(top: &[f32], ground: &[f32], nx: usize, ny: usize, side: usize) -> Vec<bool> {
    let raised: Vec<bool> = top.iter().zip(ground).map(|(&t, &g)| t > g + RAISED).collect();
    let mut seen = vec![false; nx * ny];
    let mut small = vec![false; nx * ny];
    let mut patch = Vec::new();
    for start in 0..nx * ny {
        if !raised[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        patch.clear();
        patch.push(start);
        let mut next = 0;
        while next < patch.len() {
            let k = patch[next];
            next += 1;
            let (i, j) = (k % nx, k / nx);
            for (x, y) in [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)] {
                if x < nx && y < ny && raised[y * nx + x] && !seen[y * nx + x] {
                    seen[y * nx + x] = true;
                    patch.push(y * nx + x);
                }
            }
        }
        let (mut i0, mut i1, mut j0, mut j1) = (usize::MAX, 0, usize::MAX, 0);
        for &k in &patch {
            let (i, j) = (k % nx, k / nx);
            (i0, i1, j0, j1) = (i0.min(i), i1.max(i), j0.min(j), j1.max(j));
        }
        let inside = i0 > 0 && j0 > 0 && i1 + 1 < nx && j1 + 1 < ny;
        if inside && i1 - i0 < side && j1 - j0 < side {
            for &k in &patch {
                small[k] = true;
            }
        }
    }
    small
}

/// Each sample of the grid `v` (`nx` by `nx`, rows of `nx`) made the `pick` (min or max) of the samples within `r` of it along x and
/// y (a square of 2r + 1 a side, cut by the grid's edges).
fn square(v: &[f32], nx: usize, ny: usize, r: usize, pick: fn(f32, f32) -> f32) -> Vec<f32> {
    let mut rows = vec![0.0; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            let (a, b) = (i.saturating_sub(r), (i + r).min(nx - 1));
            rows[j * nx + i] = v[j * nx + a..=j * nx + b].iter().copied().reduce(pick).unwrap_or(0.0);
        }
    }
    let mut out = vec![0.0; nx * ny];
    for j in 0..ny {
        let (a, b) = (j.saturating_sub(r), (j + r).min(ny - 1));
        for i in 0..nx {
            out[j * nx + i] = (a..=b).map(|jj| rows[jj * nx + i]).reduce(pick).unwrap_or(0.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1 by 1 tile probe (17 by 17 samples): ground at 0 m west of x = 5 m, at 10 m east of it (a cliff whose top does not walk,
    /// so the probe's walkable ground is 0 m there), with no walkmesh face in its north-east corner, where the probe's ground (4 m)
    /// stands in.
    fn two_levels() -> FloorProbe {
        let (nx, ny) = (17, 17);
        let heights = (0..nx * ny).map(|k| if k % nx >= 14 && k / nx >= 14 { 4.0 } else { 0.0 }).collect();
        let walkmesh_top = (0..nx * ny)
            .map(|k| {
                let (i, j) = (k % nx, k / nx);
                let x = i as f32 * SPACING;
                (!(i >= 14 && j >= 14)).then_some(if x < 5.0 { 0.0 } else { 10.0 })
            })
            .collect();
        FloorProbe { nx, ny, heights, colours: vec![[100; 3]; nx * ny], walkmesh_top, ..Default::default() }
    }

    #[test]
    fn the_cut_follows_the_ground_and_falls_back_on_the_probe() {
        let g = GroundCut::new(&two_levels(), 2.0, 0.0, 0.0).unwrap();
        // Each level is cut 2 m over its own ground.
        assert_eq!(g.at(1.0, 1.0), 2.0);
        assert_eq!(g.at(8.0, 1.0), 12.0);
        assert!(g.keeps([1.0, 1.0, 1.9]) && !g.keeps([1.0, 1.0, 2.1]));
        assert!(g.keeps([8.0, 1.0, 11.9]) && !g.keeps([8.0, 1.0, 12.1]));
        // Where no walkmesh face is, the probe's ground.
        assert_eq!(g.at(9.5, 9.5), 6.0);
        // Outside the area, the nearest edge's.
        assert_eq!(g.at(-3.0, 1.0), 2.0);
        assert_eq!(g.at(30.0, 1.0), 12.0);
    }

    #[test]
    fn the_reach_keeps_a_cliffs_face_over_the_ground_below() {
        let none = GroundCut::new(&two_levels(), 2.0, 0.0, 0.0).unwrap();
        let reach = GroundCut::new(&two_levels(), 2.0, 0.0, 1.25).unwrap();
        // Two samples (1.25 m) west of the cliff's first sample at 5 m, the cliff top's cut.
        let x = 5.0 - 2.0 * SPACING;
        assert_eq!((none.at(x, 1.0), reach.at(x, 1.0)), (2.0, 12.0));
        // Further west, the low ground's.
        assert_eq!(reach.at(1.0, 1.0), 2.0);
    }

    #[test]
    fn a_triangle_is_tested_only_where_the_cut_may_cross_it() {
        let g = GroundCut::new(&two_levels(), 2.0, 0.0, 0.0).unwrap();
        // On the low ground: a face at 1 m is wholly kept, one from 1 to 3 m crosses the cut.
        assert!(!g.crosses(&[[1.0, 1.0, 1.0], [2.0, 1.0, 1.0], [1.0, 2.0, 1.0]]));
        assert!(g.crosses(&[[1.0, 1.0, 1.0], [2.0, 1.0, 3.0], [1.0, 2.0, 1.0]]));
        // A face at 11 m over both levels: the cut there runs from 2 to 12 m.
        assert_eq!(g.range(&[[1.0, 1.0, 11.0], [8.0, 1.0, 11.0], [1.0, 2.0, 11.0]]), (2.0, 12.0));
    }

    #[test]
    fn a_small_raised_patch_inside_the_area_is_no_ground() {
        // 9 by 9 samples, the walkable ground at 0 m: a block 3 samples wide 4 m up in the middle, a ridge 1 m up beside it (not
        // raised: 1 m is not more than 1 m), and a block on the east edge.
        let (nx, ny) = (9, 9);
        let mut top = vec![0.0; nx * ny];
        for j in 3..6 {
            for i in 3..6 {
                top[j * nx + i] = 4.0;
            }
            top[j * nx + 1] = 1.0;
            top[j * nx + 8] = 4.0;
        }
        let small = raised_patches(&top, &vec![0.0; nx * ny], nx, ny, 3);
        let at = |i: usize, j: usize| small[j * nx + i];
        assert!(at(3, 3) && at(5, 5) && !at(1, 4) && !at(8, 4) && !at(0, 0));
        // A patch as wide as the square is not small.
        assert!(!raised_patches(&top, &vec![0.0; nx * ny], nx, ny, 2)[4 * nx + 4]);
    }
}
