// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Close-ups and marks: a picture of one rectangle of an area's ground ([`Region`]), and rings drawn over the pictures at game
//! points ([`Mark`]).
//!
//! A region is cut out of the scene as a column: every triangle is cut along the rectangle's four sides and only what stands over
//! the rectangle is drawn, so the picture is the rectangle's ground and what stands on it, and nothing in front of it or behind it
//! leaks in. The views' frames are then the column's, so a 30 m square of a 320 m area is drawn at the full scale without drawing
//! the rest. The shadows are cast by what stands within [`SHADOW_MARGIN`] of the rectangle too, so a tree 5 m outside still
//! shades the ground in it.
//!
//! The pull requests of a module change one placeable or door at a time; a mark rings where it stood and where it stands now, on
//! top of everything, so it can be found in the picture whatever covers it.

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use crate::area::Triangle;

/// How far outside a region (metres) the triangles that cast shadows into it are kept: a 20 m tower in the default light casts
/// a shadow about 25 m long.
pub const SHADOW_MARGIN: f32 = 30.0;

/// A rectangle of the ground, in game metres (x east, y north, as the `.git`'s `XPosition` and `YPosition`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Region {
    /// The south-west corner: the lowest x and y.
    pub min: [f32; 2],
    /// The north-east corner: the highest x and y.
    pub max: [f32; 2],
}

impl Region {
    /// The rectangle between two opposite corners, in any order. Fails on a corner that is not finite or a rectangle with no area.
    pub fn new(a: [f32; 2], b: [f32; 2]) -> Result<Region> {
        ensure!(a.iter().chain(&b).all(|v| v.is_finite()), "the region's corners must be numbers");
        let r = Region { min: [a[0].min(b[0]), a[1].min(b[1])], max: [a[0].max(b[0]), a[1].max(b[1])] };
        ensure!(r.max[0] > r.min[0] && r.max[1] > r.min[1], "the region must be wider and longer than 0 m");
        Ok(r)
    }

    /// `X0,Y0,X1,Y1`: two opposite corners, in metres.
    pub fn parse(s: &str) -> Result<Region> {
        let v = numbers(s).with_context(|| format!("region {s:?}: X0,Y0,X1,Y1 in metres"))?;
        ensure!(v.len() == 4, "region {s:?}: X0,Y0,X1,Y1 in metres (four numbers)");
        Region::new([v[0], v[1]], [v[2], v[3]])
    }

    /// The part of it inside an area of `size` metres; None when none of it is (touching an edge is not inside).
    pub fn clamp(self, size: [f32; 2]) -> Option<Region> {
        let r = Region { min: [self.min[0].max(0.0), self.min[1].max(0.0)], max: [self.max[0].min(size[0]), self.max[1].min(size[1])] };
        (r.max[0] > r.min[0] && r.max[1] > r.min[1]).then_some(r)
    }

    /// The rectangle `m` metres larger on every side.
    pub fn grow(self, m: f32) -> Region {
        Region { min: self.min.map(|v| v - m), max: self.max.map(|v| v + m) }
    }

    /// As `[x0, y0, x1, y1]`, the south-west corner first, for the manifest.
    pub fn corners(self) -> [f32; 4] {
        [self.min[0], self.min[1], self.max[0], self.max[1]]
    }

    /// Its four corners at height `z`.
    pub(super) fn ground(self, z: f32) -> [[f32; 3]; 4] {
        [[self.min[0], self.min[1], z], [self.max[0], self.min[1], z], [self.min[0], self.max[1], z], [self.max[0], self.max[1], z]]
    }
}

/// A mark's colour when none is given: magenta, which no ground or wall of the base game's tilesets is.
pub const MARK_COLOUR: [u8; 3] = [255, 0, 255];

/// A ring's radius in metres.
pub const MARK_RADIUS: f32 = 1.5;

/// A ring's smallest radius in pixels, so a mark can be seen at a small scale too.
const MARK_MIN_RADIUS_PX: f32 = 8.0;

/// A ring's coloured band, pixels wide.
const MARK_BAND_PX: f32 = 3.0;

/// The dark outline on each side of the band, pixels wide.
const MARK_OUTLINE_PX: f32 = 1.5;

/// The outline's colour.
const MARK_OUTLINE: [u8; 3] = [16, 16, 16];

/// A ring drawn over every picture, centred on a game point at the ground's height there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    /// Where, in game metres (x east, y north).
    pub at: [f32; 2],
    /// The ring's colour, RGB.
    pub colour: [u8; 3],
}

impl Mark {
    /// `X,Y` or `X,Y,#rrggbb`: a point in metres and the ring's colour ([`MARK_COLOUR`] when left out).
    pub fn parse(s: &str) -> Result<Mark> {
        let (point, colour) = match s.splitn(3, ',').nth(2) {
            Some(c) => (&s[..s.len() - c.len() - 1], Some(c.trim())),
            None => (s, None),
        };
        let v = numbers(point).with_context(|| format!("mark {s:?}: X,Y[,#rrggbb]"))?;
        ensure!(v.len() == 2, "mark {s:?}: X,Y[,#rrggbb] (two numbers, then a colour if any)");
        ensure!(v.iter().all(|v| v.is_finite()), "mark {s:?}: the point must be numbers");
        let colour = match colour {
            Some(c) => match super::parse_background(c) {
                Ok(super::Background::Colour(c)) => c,
                _ => bail!("mark {s:?}: the colour is #rrggbb"),
            },
            None => MARK_COLOUR,
        };
        Ok(Mark { at: [v[0], v[1]], colour })
    }
}

/// A mark as the manifest's `options` write it.
#[derive(Clone, Debug, Serialize)]
pub struct MarkInfo {
    /// [`Mark::at`].
    pub at: [f32; 2],
    /// [`Mark::colour`], `#rrggbb`.
    pub colour: String,
}

impl From<&Mark> for MarkInfo {
    fn from(m: &Mark) -> MarkInfo {
        let c = m.colour;
        MarkInfo { at: m.at, colour: format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]) }
    }
}

fn numbers(s: &str) -> Result<Vec<f32>> {
    s.split(',').map(|v| v.trim().parse::<f32>().map_err(|_| anyhow::anyhow!("{:?} is not a number", v.trim()))).collect()
}

/// `triangles` with everything outside the column over `r` cut away: a triangle across one of its sides is cut along it, the part
/// inside kept in the triangle's place (as a fan of triangles), so the order stays the scene's.
pub fn clip(triangles: &[Triangle], r: Region) -> Vec<Triangle> {
    let mut kept = Vec::new();
    for t in triangles {
        let inside = |p: &[f32; 3]| p[0] >= r.min[0] && p[0] <= r.max[0] && p[1] >= r.min[1] && p[1] <= r.max[1];
        if t.p.iter().all(inside) {
            kept.push(t.clone());
            continue;
        }
        let out = |k: usize, lo: bool| {
            let edge = if lo { r.min[k] } else { r.max[k] };
            t.p.iter().all(|p| if lo { p[k] < edge } else { p[k] > edge })
        };
        if out(0, true) || out(0, false) || out(1, true) || out(1, false) {
            continue;
        }
        // (Sutherland and Hodgman: the polygon cut by each side in turn, the UVs carried along.)
        let mut poly: Vec<([f32; 3], [f32; 2])> = (0..3).map(|i| (t.p[i], t.uv[i])).collect();
        for (k, lo) in [(0, true), (0, false), (1, true), (1, false)] {
            let edge = if lo { r.min[k] } else { r.max[k] };
            let keep = |p: &[f32; 3]| if lo { p[k] >= edge } else { p[k] <= edge };
            let mut next = Vec::with_capacity(poly.len() + 2);
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                if keep(&a.0) {
                    next.push(a);
                }
                if keep(&a.0) != keep(&b.0) {
                    let f = (edge - a.0[k]) / (b.0[k] - a.0[k]);
                    let p = [0, 1, 2].map(|c| a.0[c] + (b.0[c] - a.0[c]) * f);
                    let mut p = p;
                    p[k] = edge;
                    next.push((p, [a.1[0] + (b.1[0] - a.1[0]) * f, a.1[1] + (b.1[1] - a.1[1]) * f]));
                }
            }
            poly = next;
            if poly.len() < 3 {
                break;
            }
        }
        for i in 1..poly.len().saturating_sub(1) {
            let v = [poly[0], poly[i], poly[i + 1]];
            kept.push(Triangle { p: v.map(|x| x.0), uv: v.map(|x| x.1), texture: t.texture, part: t.part, chasm: t.chasm });
        }
    }
    kept
}

/// Water faces cut to the column over `r` ([`clip`]).
pub(super) fn clip_water(water: &[[[f32; 3]; 3]], r: Region) -> Vec<[[f32; 3]; 3]> {
    let as_triangles: Vec<Triangle> =
        water.iter().map(|p| Triangle { p: *p, uv: [[0.0; 2]; 3], texture: 0, part: crate::area::Part::Tile(0), chasm: false }).collect();
    clip(&as_triangles, r).into_iter().map(|t| t.p).collect()
}

/// Draws a ring on a supersampled picture (`w` by `h` samples, `ss` a pixel) on top of what is there: `centre` and `radius`
/// project the ring's points to samples (x right, y down), the coloured band with a dark outline each side, all of it opaque.
pub(super) fn draw_ring(
    colour: &mut [[u8; 3]],
    alpha: &mut [u8],
    w: usize,
    h: usize,
    ss: usize,
    project: impl Fn([f32; 3]) -> [f32; 2],
    centre: [f32; 3],
    scale_px: f32,
    rgb: [u8; 3],
) {
    let ssf = ss as f32;
    // (The radius in metres that is at least MARK_MIN_RADIUS_PX pixels across the picture.)
    let radius = MARK_RADIUS.max(MARK_MIN_RADIUS_PX / scale_px);
    const SEGMENTS: usize = 96;
    let points: Vec<[f32; 2]> = (0..=SEGMENTS)
        .map(|i| {
            let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            project([centre[0] + radius * a.cos(), centre[1] + radius * a.sin(), centre[2]])
        })
        .collect();
    let (half_band, reach) = (MARK_BAND_PX / 2.0 * ssf, (MARK_BAND_PX / 2.0 + MARK_OUTLINE_PX) * ssf);
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &points {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    if !(lo[0].is_finite() && hi[0].is_finite() && lo[1].is_finite() && hi[1].is_finite()) {
        return;
    }
    let x0 = (lo[0] - reach).floor().max(0.0) as usize;
    let y0 = (lo[1] - reach).floor().max(0.0) as usize;
    let x1 = ((hi[0] + reach).ceil().max(0.0) as usize).min(w);
    let y1 = ((hi[1] + reach).ceil().max(0.0) as usize).min(h);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let d = points.windows(2).map(|s| segment_distance(p, s[0], s[1])).fold(f32::MAX, f32::min);
            if d > reach {
                continue;
            }
            let i = y * w + x;
            colour[i] = if d <= half_band { rgb } else { MARK_OUTLINE };
            alpha[i] = 255;
        }
    }
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (ex, ey) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
    (ex * ex + ey * ey).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::area::Part;

    fn tri(p: [[f32; 3]; 3]) -> Triangle {
        Triangle { p, uv: p.map(|q| [q[0] / 10.0, q[1] / 10.0]), texture: 3, part: Part::Tile(7), chasm: false }
    }

    fn area(ts: &[Triangle]) -> f32 {
        ts.iter()
            .map(|t| {
                let (a, b, c) = (t.p[0], t.p[1], t.p[2]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }

    #[test]
    fn a_region_takes_its_corners_in_any_order() {
        let r = Region::parse("30, 5,10,25").unwrap();
        assert_eq!(r, Region { min: [10.0, 5.0], max: [30.0, 25.0] });
        assert_eq!(Region::parse("10,25,30,5").unwrap(), r);
        assert!(Region::parse("1,2,3").is_err());
        assert!(Region::parse("1,2,1,5").is_err());
        assert!(Region::parse("1,2,x,5").is_err());
        assert!(Region::parse("1,2,inf,5").is_err());
    }

    #[test]
    fn a_region_is_clamped_to_the_area_and_none_when_outside() {
        let r = Region::parse("-5,-5,15,8").unwrap();
        assert_eq!(r.clamp([10.0, 20.0]), Some(Region { min: [0.0, 0.0], max: [10.0, 8.0] }));
        assert_eq!(Region::parse("20,0,30,10").unwrap().clamp([20.0, 20.0]), None);
        assert_eq!(Region::parse("-9,-9,-1,-1").unwrap().clamp([20.0, 20.0]), None);
    }

    #[test]
    fn a_mark_takes_a_point_and_maybe_a_colour() {
        assert_eq!(Mark::parse("12.5,7").unwrap(), Mark { at: [12.5, 7.0], colour: MARK_COLOUR });
        assert_eq!(Mark::parse("12.5, 7, #00ff80").unwrap(), Mark { at: [12.5, 7.0], colour: [0, 255, 128] });
        assert!(Mark::parse("12.5").is_err());
        assert!(Mark::parse("1,2,3").is_err());
        assert!(Mark::parse("1,2,#ff00").is_err());
        assert_eq!(MarkInfo::from(&Mark::parse("1,2,#0A0b0C").unwrap()).colour, "#0a0b0c");
    }

    /// A 10 m square cut to a 4 by 3 m window keeps 12 m² of it, with its texture, part, corners inside and UVs as before.
    #[test]
    fn clipping_keeps_what_stands_over_the_region() {
        let square =
            [tri([[0.0, 0.0, 1.0], [10.0, 0.0, 1.0], [10.0, 10.0, 1.0]]), tri([[0.0, 0.0, 1.0], [10.0, 10.0, 1.0], [0.0, 10.0, 1.0]])];
        let r = Region::parse("3,2,7,5").unwrap();
        let cut = clip(&square, r);
        assert!((area(&cut) - 12.0).abs() < 1e-3, "{}", area(&cut));
        for t in &cut {
            assert_eq!((t.texture, t.part), (3, Part::Tile(7)));
            for (p, uv) in t.p.iter().zip(&t.uv) {
                assert!(p[0] >= 3.0 - 1e-4 && p[0] <= 7.0 + 1e-4 && p[1] >= 2.0 - 1e-4 && p[1] <= 5.0 + 1e-4, "{p:?}");
                assert!((uv[0] - p[0] / 10.0).abs() < 1e-5 && (uv[1] - p[1] / 10.0).abs() < 1e-5);
                assert_eq!(p[2], 1.0);
            }
        }
        // Wholly inside: as it was; wholly outside: gone.
        let inside = tri([[4.0, 3.0, 0.0], [5.0, 3.0, 0.0], [5.0, 4.0, 0.0]]);
        assert_eq!(clip(std::slice::from_ref(&inside), r), vec![inside]);
        assert!(clip(&[tri([[8.0, 0.0, 0.0], [9.0, 0.0, 0.0], [9.0, 9.0, 0.0]])], r).is_empty());
    }

    #[test]
    fn a_ring_is_a_band_with_a_dark_outline_and_an_empty_middle() {
        let (w, h) = (100, 100);
        let (mut colour, mut alpha) = (vec![[0u8; 3]; w * h], vec![0u8; w * h]);
        // Top-down at 20 px a metre: a ring 30 px across about (50, 50).
        let project = |p: [f32; 3]| [50.0 + (p[0] - 5.0) * 20.0, 50.0 - (p[1] - 5.0) * 20.0];
        draw_ring(&mut colour, &mut alpha, w, h, 1, project, [5.0, 5.0, 0.0], 20.0, [255, 0, 255]);
        let at = |x: usize, y: usize| (colour[y * w + x], alpha[y * w + x]);
        assert_eq!(at(80, 50), ([255, 0, 255], 255));
        assert_eq!(at(50, 20), ([255, 0, 255], 255));
        assert_eq!(at(82, 50), (MARK_OUTLINE, 255));
        assert_eq!(at(50, 50).1, 0);
        assert_eq!(at(90, 50).1, 0);
    }
}
