// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The floor probe: the ground's height and colour on a 0.625 m grid, from the tiles' walkmeshes and ground faces, and the
//! statistics the roof cut is decided by.
//!
//! The roof cut was calibrated on a height grid made another way (the reference grid), by an earlier importer that is not part of
//! nwn-bake: each tile's walkmesh rasterised at 512 pixels a tile, its gaps filled, one pixel read a sample. The probe measures the
//! same thing at the same points: at each sample, the top of the walkmesh the game walks (or deep water's bed), and where no such face
//! is, a value filled in from the walkmesh around it in the same tile, else the tile's own height. It tests the same pixel's centre
//! against the same faces, so where a face covers it the height is the reference grid's (to rounding: that grid turned tiles with a
//! rotation matrix). A sample no face covers takes the value the reference grid's gap filling gives there, worked out on a raster of
//! 128 cells a tile instead of 512 pixels, so it can differ near a gap's edges. Like the reference grid's, the filling reaches 10 m (in
//! steps along x and y) from the walkable faces; further, the sample keeps the tile's height.
//!
//! Each sample also keeps the colour of the topmost ground face there (from a small mip level of its texture), which is what tells
//! a tileset's black void from a floor.
//!
//! And it keeps the walkable floor ([`FloorProbe::floor`]): at the same points, whether the topmost face of the tiles' walkmeshes
//! there is one the game walks (`surfacemat.2da`'s `Walk`), and its height; and whether a `.git` placeable's walkmesh blocks the
//! spot ([`FloorProbe::blocked`]). `area-3d` writes them as the floor file ([`FloorProbe::floor_file`], `docs/area-3d.md`).

use glam::Vec3;

use crate::texture::Mipped;

/// Metres a tile's side.
pub const TILE: f32 = 10.0;
/// Metres between samples (the reference grid's).
pub const SPACING: f32 = 0.625;
/// Samples along a tile's side.
pub const PER_TILE: usize = 16;
/// A face counts as ground within this of the walkmesh's top under it (metres).
const ON_GROUND: f32 = 0.35;
/// Faces whose normal's up part is less than this are steep (cliffs, walls).
const STEEP_NZ: f32 = 0.64;
/// The reference grid's raster: pixels a tile.
const RASTER: usize = 512;
/// The raster a tile's gaps are filled on: cells a tile (eight a sample).
const FILL: usize = 128;
/// A ground texture's colour is read from its first mip level this small or smaller.
const COLOUR_LEVEL: usize = 32;
/// A floor of a cave dug out of rock lies at least this far below the rock's top (about a wall's height, metres).
pub const FLOOR_BELOW: f32 = 2.0;

/// The ground on a 0.625 m grid: `nx` by `ny` samples (a tile's side is 16 of them, and one more closes the last tile), rows from
/// the south, sample (i, j) at (i, j) * [`SPACING`] metres.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FloorProbe {
    /// Samples from west to east.
    pub nx: usize,
    /// Samples from south to north.
    pub ny: usize,
    /// The ground's height at each sample (see the module).
    pub heights: Vec<f32>,
    /// The colour of the topmost ground face at each sample; from the samples around it where none is, black in a tile with no
    /// ground face at all.
    pub colours: Vec<[u8; 3]>,
    /// The walkable floor at each sample: the height of the topmost face of the tiles' walkmeshes there when the game walks it; None
    /// where that face does not walk (a wall's foot, a chasm's floor, deep water) or no face is. Not filled in, unlike `heights`.
    pub floor: Vec<Option<f32>>,
    /// Whether a placeable of the `.git` blocks the sample: one of its walkmesh's (`.pwk`) unwalkable faces overlaps the sample's
    /// cell, the [`SPACING`] square about it. All false for a scene without objects.
    pub blocked: Vec<bool>,
}

/// The floor file's first bytes ([`FloorProbe::floor_file`]).
pub const FLOOR_MAGIC: &[u8; 4] = b"NBFL";
/// The floor file's version: raised when its layout changes.
pub const FLOOR_VERSION: u16 = 1;
/// The floor file's header, in bytes.
pub const FLOOR_HEADER: usize = 16;
/// A height in the floor file is in this many metres (centimetres).
pub const FLOOR_UNIT: f32 = 0.01;
/// Flag bit: the sample's top walkmesh face walks.
pub const FLOOR_WALKABLE: u8 = 1;
/// Flag bit: a `.git` placeable's walkmesh blocks the sample.
pub const FLOOR_BLOCKED: u8 = 2;

/// A floor file read back ([`FloorFile::read`]): what [`FloorProbe::floor_file`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorFile {
    /// Samples a tile's side (16).
    pub per_tile: usize,
    /// Samples from west to east.
    pub nx: usize,
    /// Samples from south to north.
    pub ny: usize,
    /// Each sample's height, in centimetres.
    pub heights: Vec<i16>,
    /// Each sample's flags ([`FLOOR_WALKABLE`], [`FLOOR_BLOCKED`]).
    pub flags: Vec<u8>,
}

impl FloorFile {
    /// Reads a floor file. Fails on a file that is not one, of another version, or of the wrong length.
    pub fn read(bytes: &[u8]) -> anyhow::Result<FloorFile> {
        anyhow::ensure!(bytes.len() >= FLOOR_HEADER && &bytes[..4] == FLOOR_MAGIC, "not a floor file");
        let u16_at = |k: usize| usize::from(u16::from_le_bytes([bytes[k], bytes[k + 1]]));
        anyhow::ensure!(u16_at(4) == usize::from(FLOOR_VERSION), "floor file version {}, not {FLOOR_VERSION}", u16_at(4));
        let (per_tile, nx, ny) = (u16_at(6), u16_at(8), u16_at(10));
        let n = nx * ny;
        anyhow::ensure!(bytes.len() == FLOOR_HEADER + 3 * n, "a floor file of {nx} by {ny} samples is not {} bytes long", bytes.len());
        let heights = bytes[FLOOR_HEADER..FLOOR_HEADER + 2 * n].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        Ok(FloorFile { per_tile, nx, ny, heights, flags: bytes[FLOOR_HEADER + 2 * n..].to_vec() })
    }

    /// Sample (`i`, `j`)'s index in `heights` and `flags`.
    pub fn index(&self, i: usize, j: usize) -> usize {
        j * self.nx + i
    }

    /// Whether sample (`i`, `j`) is walkable floor and not blocked.
    pub fn open(&self, i: usize, j: usize) -> bool {
        self.flags[self.index(i, j)] & (FLOOR_WALKABLE | FLOOR_BLOCKED) == FLOOR_WALKABLE
    }
}

/// A tile as the probe sees it: its height, its walkmesh and its drawn faces, in the tile's own square (x and y -5..5 m about its
/// middle, turned as placed; z above the area's 0).
pub(crate) struct ProbeTile<'a> {
    pub z: f32,
    /// The faces the game walks, and deep water's bed.
    pub walk: Vec<[Vec3; 3]>,
    /// Every face of the walkmesh, and whether the game walks it (the walkable floor).
    pub wok: Vec<([Vec3; 3], bool)>,
    /// Faces that are drawn (water left out), each with its UVs and texture (None: the texture is missing).
    pub faces: Vec<([Vec3; 3], [[f32; 2]; 3], Option<&'a Mipped>)>,
}

impl FloorProbe {
    /// Whether sample `k` lies on black: a tileset's void filler (`tdm01_black`, `tbw01_black1`), left out of the floor's median.
    pub fn is_void(&self, k: usize) -> bool {
        let c = self.colours[k];
        u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2]) <= 15
    }

    /// The probe of an area `width` by `height` tiles, its tiles row by row from the south-west.
    pub(crate) fn measure(width: usize, height: usize, tiles: &[ProbeTile<'_>]) -> FloorProbe {
        let (nx, ny) = (width * PER_TILE + 1, height * PER_TILE + 1);
        let mut probe = FloorProbe {
            nx,
            ny,
            heights: vec![0.0; nx * ny],
            colours: vec![[0; 3]; nx * ny],
            floor: vec![None; nx * ny],
            blocked: vec![false; nx * ny],
        };
        for (t, tile) in tiles.iter().enumerate().take(width * height) {
            let (tx, ty) = (t % width, t / width);
            // (The last tile of a row or column also has the samples on the area's far edge.)
            let (ni, nj) = (PER_TILE + usize::from(tx + 1 == width), PER_TILE + usize::from(ty + 1 == height));
            let (heights, colours) = probe_tile(tile, ni, nj);
            let floor = floor_tile(&tile.wok, ni, nj);
            for cj in 0..nj {
                for ci in 0..ni {
                    let k = (ty * PER_TILE + cj) * nx + tx * PER_TILE + ci;
                    probe.heights[k] = heights[cj * ni + ci];
                    probe.colours[k] = colours[cj * ni + ci];
                    probe.floor[k] = floor[cj * ni + ci];
                }
            }
        }
        probe
    }

    /// Where sample `i` lies along x, in metres from the area's west edge ([`sample_metres`]).
    pub fn sample_x(&self, i: usize) -> f32 {
        sample_metres(i, self.nx)
    }

    /// Where sample `j` lies along y, in metres from the area's south edge ([`sample_metres`]).
    pub fn sample_y(&self, j: usize) -> f32 {
        sample_metres(j, self.ny)
    }

    /// Marks as blocked the samples whose cell (the [`SPACING`] square about the sample) the triangles `blocks` overlap (x, y in
    /// metres from the area's south-west corner): the unwalkable faces of the `.git` placeables' walkmeshes, placed. The cell, not the
    /// point: a fence 0.5 m thick lies between two rows of samples, and a flood fill must not pass through it.
    pub(crate) fn block(&mut self, blocks: &[[[f32; 2]; 3]]) {
        let xs: Vec<f32> = (0..self.nx).map(|i| self.sample_x(i)).collect();
        let ys: Vec<f32> = (0..self.ny).map(|j| self.sample_y(j)).collect();
        for t in blocks {
            overlapped(t, &xs, &ys, SPACING / 2.0, |k| self.blocked[k] = true);
        }
    }

    /// The floor file (`docs/area-3d.md`, "The floor file"): a 16-byte header, then each sample's height in centimetres (`i16`, rows
    /// from the south, west to east: the walkable floor's height where it walks, the probe's ground elsewhere), then each sample's
    /// flags (one byte: [`FLOOR_WALKABLE`], [`FLOOR_BLOCKED`]). Little-endian.
    pub fn floor_file(&self) -> Vec<u8> {
        let n = self.nx * self.ny;
        let mut out = Vec::with_capacity(FLOOR_HEADER + 3 * n);
        out.extend_from_slice(FLOOR_MAGIC);
        out.extend_from_slice(&FLOOR_VERSION.to_le_bytes());
        for v in [PER_TILE, self.nx, self.ny] {
            out.extend_from_slice(&u16::try_from(v).unwrap_or(u16::MAX).to_le_bytes());
        }
        out.extend_from_slice(&[0; 4]);
        for k in 0..n {
            let h = self.floor[k].unwrap_or(self.heights[k]);
            out.extend_from_slice(&((h / FLOOR_UNIT).round().clamp(-32767.0, 32767.0) as i16).to_le_bytes());
        }
        for k in 0..n {
            out.push(if self.floor[k].is_some() { FLOOR_WALKABLE } else { 0 } | if self.blocked[k] { FLOOR_BLOCKED } else { 0 });
        }
        out
    }

    /// How many samples are walkable floor, and how many of those a placeable blocks.
    pub fn floor_counts(&self) -> (usize, usize) {
        let walkable = self.floor.iter().filter(|f| f.is_some()).count();
        (walkable, self.floor.iter().zip(&self.blocked).filter(|(f, b)| f.is_some() && **b).count())
    }

    /// The statistics the roof cut is decided by.
    pub fn stats(&self) -> FloorStats {
        let mut sorted = self.heights.clone();
        sorted.sort_by(f32::total_cmp);
        let n = sorted.len().max(1) as f32;
        let (lowest, roof) = (sorted.first().copied().unwrap_or(0.0), sorted.last().copied().unwrap_or(0.0));
        let top = sorted.iter().filter(|&&h| h > roof - 0.3).count();
        let mut drawn: Vec<f32> = (0..self.heights.len()).filter(|&k| !self.is_void(k)).map(|k| self.heights[k]).collect();
        drawn.sort_by(f32::total_cmp);
        // (Black void is no floor: a cave's void can lie 6 m under its walkable ground.)
        let floors: Vec<f32> = drawn.iter().copied().filter(|&h| h < roof - FLOOR_BELOW).collect();
        let drawn_share = drawn.len() as f32 / n;
        // (Where nearly everything is black, the median of all of it.)
        if drawn.len() * 10 < sorted.len() {
            drawn = sorted.clone();
        }
        FloorStats {
            samples: sorted.len(),
            lowest,
            roof,
            top_share: top as f32 / n,
            floor_share: floors.len() as f32 / n,
            floors: floors.len(),
            floor_p95: floors.get(floors.len() * 95 / 100).copied(),
            drawn_share,
            median: drawn.get(drawn.len() / 2).copied().unwrap_or(0.0),
        }
    }
}

/// What the roof cut is decided by: the ground's statistics over every sample of the probe.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct FloorStats {
    /// How many samples the probe has.
    pub samples: usize,
    /// The lowest sample's height (metres).
    pub lowest: f32,
    /// The highest sample.
    pub roof: f32,
    /// Share of samples within 0.3 m of the roof (black void too: a rock top may be drawn as void).
    pub top_share: f32,
    /// Share of samples that are drawn ground (not black void) at least [`FLOOR_BELOW`] under the roof (the floors), and how many.
    pub floor_share: f32,
    /// How many samples are floors (see `floor_share`).
    pub floors: usize,
    /// The 95th percentile of the floors.
    pub floor_p95: Option<f32>,
    /// Share of samples not on black void.
    pub drawn_share: f32,
    /// The median height of the samples not on black void (of all of them when fewer than a tenth are).
    pub median: f32,
}

/// Where the roof is cut, and why.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct RoofCut {
    /// Everything above this height (metres) is cut away.
    pub at: f32,
    /// A cave dug out of rock: the cut is measured from the highest floor rather than the median.
    pub slab: bool,
}

/// The calibrated rule for the roof cut.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoofRule {
    /// Metres over the floor.
    pub over: f32,
    /// A cave dug out of rock needs at least this share of the ground to be its floors (drawn, [`FLOOR_BELOW`] under its top).
    pub slab_floors: f32,
}

impl Default for RoofRule {
    fn default() -> Self {
        RoofRule { over: 3.5, slab_floors: 0.08 }
    }
}

impl FloorStats {
    /// Where the roof is cut by `rule`.
    ///
    /// A cave dug out of rock is cut a little over its highest floor (the 95th percentile), clear of the rock top, so the passages
    /// show and the rock mass is cut away. It is one when more than a quarter of the ground is the rock's top and a real share of it
    /// (`rule.slab_floors`) is floors: drawn ground, not black void, at least [`FLOOR_BELOW`] (a wall's height) under the top. A
    /// floor with one sunken row along its edge (1% of it) is not one; nor is a cave whose walkable ground is the top, with void or
    /// a lake bed under it, or barrows whose cells stand 0.5 m over the rest. Anything else is cut `rule.over` over its floor, the
    /// median of the ground that is not black void (a barrows area can be more than half void at 0 m around floors 5 m up).
    pub fn roof_cut(&self, rule: &RoofRule) -> RoofCut {
        let slab = self.top_share > 0.25 && self.floors > 50 && self.floor_share >= rule.slab_floors;
        match self.floor_p95.filter(|_| slab) {
            Some(highest) => RoofCut { at: (highest + rule.over.clamp(1.0, 2.0)).min(self.roof - 0.4), slab: true },
            None => RoofCut { at: self.median + rule.over, slab: false },
        }
    }
}

/// Where sample `c` of `n` along one axis lies, in metres from the area's edge: the reference grid's pixel centre the probe tests,
/// 0.0098 m past `c` times [`SPACING`]; the last sample (the area's far edge) 0.0098 m short of it.
pub fn sample_metres(c: usize, n: usize) -> f32 {
    // (The last tile of a row also has the sample on the far edge: its 17th, whose pixel is the tile's last.)
    let (tile, within) =
        if c > 0 && c + 1 == n { ((c - 1) / PER_TILE, c - (c - 1) / PER_TILE * PER_TILE) } else { (c / PER_TILE, c % PER_TILE) };
    tile as f32 * TILE + pixel(within) / RASTER as f32 * TILE
}

/// Sample `c` of a tile's reference grid pixel centre, in pixels (the far edge's, 16, its last pixel).
fn pixel(c: usize) -> f32 {
    ((c * RASTER / PER_TILE).min(RASTER - 1)) as f32 + 0.5
}

/// Calls `f(sample)` for each sample (at `xs[i]`, `ys[j]`, index `j * xs.len() + i`) whose cell, the square `half` metres each way
/// about it, the triangle `t` overlaps by more than a millimetre (separating axes: x, y and the triangle's three edge normals).
fn overlapped(t: &[[f32; 2]; 3], xs: &[f32], ys: &[f32], half: f32, mut f: impl FnMut(usize)) {
    const SLACK: f32 = 1e-3;
    let [a, b, c] = *t;
    if ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() < 1e-9 {
        return;
    }
    let (x0, x1) = (a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0]));
    let (y0, y1) = (a[1].min(b[1]).min(c[1]), a[1].max(b[1]).max(c[1]));
    let normals: Vec<[f32; 2]> = [(a, b), (b, c), (c, a)].iter().map(|(p, q)| [q[1] - p[1], p[0] - q[0]]).collect();
    for (j, &y) in ys.iter().enumerate().filter(|(_, y)| **y + half > y0 + SLACK && **y - half < y1 - SLACK) {
        for (i, &x) in xs.iter().enumerate().filter(|(_, x)| **x + half > x0 + SLACK && **x - half < x1 - SLACK) {
            // (The square and the triangle overlap unless one edge normal separates them.)
            let apart = normals.iter().any(|n| {
                let tri = [a, b, c].map(|p| n[0] * p[0] + n[1] * p[1]);
                let (lo, hi) = (tri[0].min(tri[1]).min(tri[2]), tri[0].max(tri[1]).max(tri[2]));
                let (centre, reach) = (n[0] * x + n[1] * y, half * (n[0].abs() + n[1].abs()));
                let slack = SLACK * n[0].hypot(n[1]);
                centre + reach <= lo + slack || centre - reach >= hi - slack
            });
            if !apart {
                f(j * xs.len() + i);
            }
        }
    }
}

/// One tile's walkable floor (`ni` by `nj` samples, rows from the south, as [`probe_tile`]'s): at each sample, the topmost face of
/// the tile's walkmesh (`wok`, in the tile's own square) that covers it; its height when the game walks it, None when it does not or
/// no face covers the sample. Of two faces at one height (a shared edge), a walkable one wins.
fn floor_tile(wok: &[([Vec3; 3], bool)], ni: usize, nj: usize) -> Vec<Option<f32>> {
    let to_pixels = |v: Vec3| Vec3::new((v.x + TILE / 2.0) / TILE * RASTER as f32, (v.y + TILE / 2.0) / TILE * RASTER as f32, v.z);
    let mut top: Vec<Option<(f32, bool)>> = vec![None; ni * nj];
    for (f, walks) in wok {
        covered(f.map(to_pixels), ni, nj, pixel, |k, z| {
            let higher = match top[k] {
                None => true,
                Some((h, w)) => z > h + 1e-3 || (z > h - 1e-3 && *walks && !w),
            };
            if higher {
                top[k] = Some((z, *walks));
            }
        });
    }
    top.into_iter().map(|t| t.filter(|t| t.1).map(|t| t.0)).collect()
}

/// One tile's samples (`ni` by `nj`, rows from the south): heights and colours.
fn probe_tile(tile: &ProbeTile<'_>, ni: usize, nj: usize) -> (Vec<f32>, Vec<[u8; 3]>) {
    // (Sample c of the tile is the reference grid's pixel c * 32, tested at the pixel's centre: `pixel`.)
    let to_pixels = |v: Vec3| Vec3::new((v.x + TILE / 2.0) / TILE * RASTER as f32, (v.y + TILE / 2.0) / TILE * RASTER as f32, v.z);
    let mut heights = vec![f32::NAN; ni * nj];
    for f in &tile.walk {
        covered(f.map(to_pixels), ni, nj, pixel, |k, z| {
            if heights[k].is_nan() || z > heights[k] {
                heights[k] = z;
            }
        });
    }
    // A sample no face covers: what the reference grid fills its pixel with, from a finer raster than the samples (a walkable patch smaller
    // than a sample's spacing, between cliffs, still decides where the fill from each side meets).
    if heights.iter().any(|h| h.is_nan()) {
        let gaps = filled_walkmesh(&tile.walk);
        let cell = |c: usize| ((pixel(c) / RASTER as f32 * FILL as f32) as usize).min(FILL - 1);
        for cj in 0..nj {
            for ci in 0..ni {
                let k = cj * ni + ci;
                if heights[k].is_nan() {
                    heights[k] = gaps[cell(cj) * FILL + cell(ci)];
                }
            }
        }
    }
    for h in heights.iter_mut().filter(|h| h.is_nan()) {
        *h = tile.z;
    }

    // Each drawn face is ground (on it, or a cliff between its levels) or stands on it: above the highest ground within 0.75 m of
    // its middle, or steep where the ground there does not step (a trunk's foot, a wall).
    let local = |c: usize| pixel(c) / RASTER as f32 * TILE - TILE / 2.0;
    let on_ground = |f: &[Vec3; 3]| {
        let c = (f[0] + f[1] + f[2]) / 3.0;
        let (mut low, mut top) = (f32::MAX, f32::MIN);
        for cj in 0..nj {
            for ci in 0..ni {
                if (local(ci) - c.x).abs() <= 0.75 && (local(cj) - c.y).abs() <= 0.75 {
                    low = low.min(heights[cj * ni + ci]);
                    top = top.max(heights[cj * ni + ci]);
                }
            }
        }
        if top < low {
            return true;
        }
        let n = (f[1] - f[0]).cross(f[2] - f[0]);
        let steep = n.length() > 1e-6 && (n.z / n.length()).abs() < STEEP_NZ;
        c.z <= top + ON_GROUND && (!steep || top - low > 1.0)
    };
    let mut top = vec![f32::NEG_INFINITY; ni * nj];
    let mut colours: Vec<Option<[u8; 3]>> = vec![None; ni * nj];
    for (p, uv, texture) in &tile.faces {
        let Some(texture) = texture else { continue };
        if !on_ground(p) {
            continue;
        }
        let level = texture.level_within(COLOUR_LEVEL) as f32;
        covered_with(p.map(to_pixels), ni, nj, pixel, |k, z, b| {
            if z > heights[k] + ON_GROUND || z <= top[k] {
                return;
            }
            let (u, v) = (b[0] * uv[0][0] + b[1] * uv[1][0] + b[2] * uv[2][0], b[0] * uv[0][1] + b[1] * uv[1][1] + b[2] * uv[2][1]);
            let c = texture.sample(u, v, level, false);
            // (A cut-out's hole: what is under it shows.)
            if c[3] < 128.0 {
                return;
            }
            top[k] = z;
            colours[k] = Some([c[0].round() as u8, c[1].round() as u8, c[2].round() as u8]);
        });
    }
    fill(&mut colours, ni, nj, None, Option::is_some, |near: &[Option<[u8; 3]>]| {
        let mut sum = [0u32; 3];
        for c in near.iter().flatten() {
            for k in 0..3 {
                sum[k] += u32::from(c[k]);
            }
        }
        let n = near.len() as u32;
        Some(sum.map(|s| (s / n) as u8))
    });
    (heights, colours.into_iter().map(|c| c.unwrap_or([0; 3])).collect())
}

/// Calls `f(sample, z)` for each sample whose pixel centre the triangle (in pixels, z in metres) covers, as the reference grid's
/// raster tests a pixel.
fn covered(t: [Vec3; 3], ni: usize, nj: usize, pixel: impl Fn(usize) -> f32, mut f: impl FnMut(usize, f32)) {
    covered_with(t, ni, nj, pixel, |k, z, _| f(k, z));
}

fn covered_with(t: [Vec3; 3], ni: usize, nj: usize, pixel: impl Fn(usize) -> f32, mut f: impl FnMut(usize, f32, [f32; 3])) {
    let (a, b, c) = (t[0], t[1], t[2]);
    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if area.abs() < 1e-9 {
        return;
    }
    // (The raster visits pixels from the floor of the lowest corner to below the ceiling of the highest.)
    let (x0, x1) = (a.x.min(b.x).min(c.x).floor(), a.x.max(b.x).max(c.x).ceil());
    let (y0, y1) = (a.y.min(b.y).min(c.y).floor(), a.y.max(b.y).max(c.y).ceil());
    for cj in 0..nj {
        let y = pixel(cj);
        if y < y0 || y - 0.5 >= y1 {
            continue;
        }
        for ci in 0..ni {
            let x = pixel(ci);
            if x < x0 || x - 0.5 >= x1 {
                continue;
            }
            let w0 = ((b.x - x) * (c.y - y) - (b.y - y) * (c.x - x)) / area;
            let w1 = ((c.x - x) * (a.y - y) - (c.y - y) * (a.x - x)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                f(cj * ni + ci, w0 * a.z + w1 * b.z + w2 * c.z, [w0, w1, w2]);
            }
        }
    }
}

/// A tile's walkmesh (`walk`, the tile's own square) rasterised at [`FILL`] cells a side, the top face's height in each cell, and
/// the cells no face covers filled as [`fill`] does: NaN everywhere when there is no face at all.
fn filled_walkmesh(walk: &[[Vec3; 3]]) -> Vec<f32> {
    let to_cells = |v: Vec3| Vec3::new((v.x + TILE / 2.0) / TILE * FILL as f32, (v.y + TILE / 2.0) / TILE * FILL as f32, v.z);
    let centre = |c: usize| c as f32 + 0.5;
    let mut z = vec![f32::NAN; FILL * FILL];
    for f in walk {
        covered(f.map(to_cells), FILL, FILL, centre, |k, h| {
            if z[k].is_nan() || h > z[k] {
                z[k] = h;
            }
        });
    }
    // (The reference grid stops after as many passes as its raster has pixels a side, 10 m of steps: what is further from every walkable
    // face keeps the tile's height. tid01_j01_01 has one walkable face, in a corner, and the far corner is 10.1 m from it.)
    fill_heights(&mut z, FILL, FILL, Some(FILL));
    z
}

/// Fills the NaN cells of a height grid `ni` by `nj` (rows of `ni`) with the mean of the heights beside them, as [`fill`] does: the
/// reference grid's gap filling (with `passes` its raster's side), and the probe's.
pub fn fill_heights(z: &mut [f32], ni: usize, nj: usize, passes: Option<usize>) {
    fill(z, ni, nj, passes, |v: &f32| !v.is_nan(), |near: &[f32]| near.iter().sum::<f32>() / near.len() as f32);
}

/// Fills the cells that have nothing (`has` false) from the ones beside them (`mean` of those that have, west, east, south, north),
/// pass after pass, until none is left, a pass fills nothing (a tile with nothing at all) or `passes` passes are done. This is the
/// reference grid's gap filling; a pass only visits the cells beside the ones the pass before filled, which gives the same values without
/// walking every cell.
pub fn fill<T: Clone>(v: &mut [T], ni: usize, nj: usize, passes: Option<usize>, has: impl Fn(&T) -> bool, mean: impl Fn(&[T]) -> T) {
    let beside = |k: usize| {
        let (i, j) = (k % ni, k / ni);
        [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)]
            .into_iter()
            .filter(|&(x, y)| x < ni && y < nj)
            .map(|(x, y)| y * ni + x)
    };
    // Cells to fill this pass: those with nothing beside one that has something, in index order.
    let mut queued = vec![false; v.len()];
    let mut pass: Vec<usize> = Vec::new();
    for k in 0..v.len() {
        if !has(&v[k]) && beside(k).any(|n| has(&v[n])) {
            queued[k] = true;
            pass.push(k);
        }
    }
    let mut done = 0;
    while !pass.is_empty() && passes.is_none_or(|n| done < n) {
        done += 1;
        // (Every value of a pass is taken from the cells as they were before it.)
        let mut near: Vec<T> = Vec::with_capacity(4);
        let made: Vec<T> = pass
            .iter()
            .map(|&k| {
                near.clear();
                near.extend(beside(k).map(|n| v[n].clone()).filter(|t| has(t)));
                mean(&near)
            })
            .collect();
        for (&k, value) in pass.iter().zip(made) {
            v[k] = value;
        }
        let mut next = Vec::new();
        for &k in &pass {
            for n in beside(k) {
                if !queued[n] && !has(&v[n]) {
                    queued[n] = true;
                    next.push(n);
                }
            }
        }
        next.sort_unstable();
        pass = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(heights: Vec<f32>, colours: Vec<[u8; 3]>, nx: usize) -> FloorProbe {
        FloorProbe { nx, ny: heights.len() / nx, heights, colours, ..Default::default() }
    }

    #[test]
    fn a_flat_floor_is_cut_over_its_median() {
        let p = probe(vec![2.0; 100], vec![[90; 3]; 100], 10);
        let s = p.stats();
        assert_eq!((s.roof, s.median, s.top_share, s.floors), (2.0, 2.0, 1.0, 0));
        assert_eq!(s.roof_cut(&RoofRule::default()), RoofCut { at: 5.5, slab: false });
    }

    #[test]
    fn a_cave_dug_out_of_rock_is_cut_over_its_highest_floor() {
        // 70% rock top at 6 m, 30% passages at 0 to 0.29 m.
        let heights: Vec<f32> = (0..1000).map(|k| if k < 700 { 6.0 } else { (k - 700) as f32 / 1000.0 }).collect();
        let s = probe(heights, vec![[90; 3]; 1000], 100).stats();
        assert!(s.top_share > 0.69 && s.floors == 300);
        let cut = s.roof_cut(&RoofRule::default());
        assert!(cut.slab);
        assert!((cut.at - (0.285 + 2.0)).abs() < 1e-3, "{cut:?}");
        // One sunken row along the edge (1% of it) is no cave dug out of rock.
        let heights: Vec<f32> = (0..1000).map(|k| if k < 990 { 6.0 } else { 0.0 }).collect();
        assert!(!probe(heights, vec![[90; 3]; 1000], 100).stats().roof_cut(&RoofRule::default()).slab);
    }

    #[test]
    fn walkable_ground_at_the_top_over_void_or_a_step_is_no_cave_dug_out_of_rock() {
        // A cave whose walkable ground is at 0 to 0.5 m, 20% black void 6 m under it. Cut over the floor, at 3.5 m.
        let heights: Vec<f32> = (0..1000)
            .map(|k| {
                if k < 200 {
                    -6.0
                } else if k < 700 {
                    0.0
                } else {
                    0.5
                }
            })
            .collect();
        let colours: Vec<[u8; 3]> = (0..1000).map(|k| if k < 200 { [0; 3] } else { [90; 3] }).collect();
        let s = probe(heights, colours, 100).stats();
        assert!(s.top_share > 0.25 && s.floors == 0, "{s:?}");
        assert_eq!(s.roof_cut(&RoofRule::default()), RoofCut { at: 3.5, slab: false });
        // Barrows: cells 0.6 m over the floor at 5 m, the floor 40%: no floor is a wall's height under the top.
        let heights: Vec<f32> = (0..1000).map(|k| if k < 400 { 5.0 } else { 5.6 }).collect();
        let s = probe(heights, vec![[90; 3]; 1000], 100).stats();
        assert!(!s.roof_cut(&RoofRule::default()).slab, "{s:?}");
        // Drawn floors under a black rock top still are a cave dug out of rock.
        let heights: Vec<f32> = (0..1000).map(|k| if k < 500 { 4.0 } else { 0.0 }).collect();
        let colours: Vec<[u8; 3]> = (0..1000).map(|k| if k < 500 { [0; 3] } else { [90; 3] }).collect();
        assert_eq!(probe(heights, colours, 100).stats().roof_cut(&RoofRule::default()), RoofCut { at: 2.0, slab: true });
    }

    #[test]
    fn black_void_is_left_out_of_the_floor() {
        // 60% void at 0 m, 40% floor at 5 m: the floor is 5 m.
        let heights: Vec<f32> = (0..100).map(|k| if k < 60 { 0.0 } else { 5.0 }).collect();
        let colours: Vec<[u8; 3]> = (0..100).map(|k| if k < 60 { [2, 2, 2] } else { [80, 70, 60] }).collect();
        let s = probe(heights.clone(), colours, 10).stats();
        assert_eq!((s.median, s.drawn_share), (5.0, 0.4));
        // Nearly all void: the median of all of it.
        let colours: Vec<[u8; 3]> = (0..100).map(|k| if k < 95 { [0; 3] } else { [80; 3] }).collect();
        assert_eq!(probe(heights, colours, 10).stats().median, 0.0);
    }

    #[test]
    fn a_tile_s_samples_are_the_top_of_its_walkmesh_gaps_filled() {
        let q = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| [Vec3::from_array(a), Vec3::from_array(b), Vec3::from_array(c)];
        // The tile's west half walks at 1 m, a raised step in its south-west corner at 2 m; the east half has no walkmesh.
        let walk = vec![
            q([-5.0, -5.0, 1.0], [0.0, -5.0, 1.0], [0.0, 5.0, 1.0]),
            q([-5.0, -5.0, 1.0], [0.0, 5.0, 1.0], [-5.0, 5.0, 1.0]),
            q([-5.0, -5.0, 2.0], [-3.0, -5.0, 2.0], [-5.0, -3.0, 2.0]),
        ];
        let tile = ProbeTile { z: 7.0, walk, wok: Vec::new(), faces: Vec::new() };
        let (h, c) = probe_tile(&tile, 16, 16);
        assert_eq!(h[0], 2.0);
        assert_eq!(h[5 * 16 + 5], 1.0);
        // The east half is filled from the west.
        assert!((h[8 * 16 + 15] - 1.0).abs() < 1e-6);
        // No ground face at all: black.
        assert!(c.iter().all(|c| *c == [0; 3]));
        // No walkmesh at all: the tile's height.
        let (h, _) = probe_tile(&ProbeTile { z: 7.0, walk: Vec::new(), wok: Vec::new(), faces: Vec::new() }, 17, 16);
        assert!(h.iter().all(|&z| z == 7.0));
    }

    #[test]
    fn the_walkable_floor_is_the_top_walkmesh_face_where_it_walks() {
        let q = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| [Vec3::from_array(a), Vec3::from_array(b), Vec3::from_array(c)];
        let wok = vec![
            // The west half walks at 1 m.
            (q([-5.0, -5.0, 1.0], [0.0, -5.0, 1.0], [0.0, 5.0, 1.0]), true),
            (q([-5.0, -5.0, 1.0], [0.0, 5.0, 1.0], [-5.0, 5.0, 1.0]), true),
            // A wall's foot over it in the south-west corner (0.1 m up) does not.
            (q([-5.0, -5.0, 1.1], [-3.0, -5.0, 1.1], [-5.0, -3.0, 1.1]), false),
            // The east half: deep water's bed (not walked) under a walkable bridge along its north edge.
            (q([0.0, -5.0, -2.0], [5.0, -5.0, -2.0], [5.0, 5.0, -2.0]), false),
            (q([0.0, -5.0, -2.0], [5.0, 5.0, -2.0], [0.0, 5.0, -2.0]), false),
            (q([0.0, 3.0, 0.5], [5.0, 3.0, 0.5], [5.0, 5.0, 0.5]), true),
            (q([0.0, 3.0, 0.5], [5.0, 5.0, 0.5], [0.0, 5.0, 0.5]), true),
        ];
        let f = floor_tile(&wok, 16, 16);
        let at = |i: usize, j: usize| f[j * 16 + i];
        assert_eq!(at(0, 0), None);
        assert_eq!(at(5, 5), Some(1.0));
        assert_eq!(at(12, 5), None);
        assert_eq!(at(12, 15), Some(0.5));
        // Of two faces at one height, the walkable one: a walkable face under the same spot as a blocking one 0.5 mm higher.
        let tie = vec![
            (q([-5.0, -5.0, 0.0005], [5.0, -5.0, 0.0005], [-5.0, 5.0, 0.0005]), false),
            (q([-5.0, -5.0, 0.0], [5.0, -5.0, 0.0], [-5.0, 5.0, 0.0]), true),
        ];
        assert_eq!(floor_tile(&tie, 16, 16)[0], Some(0.0));
        // No walkmesh: nothing walks.
        assert!(floor_tile(&[], 17, 17).iter().all(Option::is_none));
    }

    #[test]
    fn the_samples_lie_where_the_probe_tests_them() {
        assert!((sample_metres(0, 33) - 0.009_765_6).abs() < 1e-6);
        assert!((sample_metres(1, 33) - 0.634_765_6).abs() < 1e-6);
        assert!((sample_metres(16, 33) - 10.009_766).abs() < 1e-5);
        // The far edge's sample: the last tile's last pixel.
        assert!((sample_metres(32, 33) - 19.990_234).abs() < 1e-5);
    }

    #[test]
    fn the_floor_file_holds_heights_and_flags_and_reads_back() {
        // A 1 by 1 tile area: 17 by 17 samples.
        let n = 17 * 17;
        let mut p = FloorProbe {
            nx: 17,
            ny: 17,
            heights: vec![-3.0; n],
            colours: vec![[0; 3]; n],
            floor: (0..n).map(|k| (k % 17 < 8).then_some(1.234)).collect(),
            blocked: vec![false; n],
        };
        // A placeable's block over x 2..3.3 m, y 2..3.3 m (two triangles): the cells of samples 3 to 5 each way (sample 3's runs
        // from 1.57 to 2.20 m, sample 6's from 3.45 m).
        p.block(&[[[2.0, 2.0], [3.3, 2.0], [3.3, 3.3]], [[2.0, 2.0], [3.3, 3.3], [2.0, 3.3]]]);
        let blocked: Vec<(usize, usize)> = (0..n).filter(|&k| p.blocked[k]).map(|k| (k % 17, k / 17)).collect();
        assert_eq!(blocked, (3..6).flat_map(|j| (3..6).map(move |i| (i, j))).collect::<Vec<_>>());
        assert_eq!(p.floor_counts(), (8 * 17, 9));
        // A fence 0.3 m thick between two rows of samples (y 7.6 to 7.9 m; samples 12 and 13 at 7.51 and 8.13 m) blocks both rows'
        // cells it overlaps; a thin sliver touching a cell's edge by less than a millimetre does not.
        let mut q = p.clone();
        q.blocked = vec![false; n];
        q.block(&[[[0.0, 7.6], [10.0, 7.6], [10.0, 7.9]], [[0.0, 7.6], [10.0, 7.9], [0.0, 7.9]]]);
        assert!((0..17).all(|i| q.blocked[12 * 17 + i] && q.blocked[13 * 17 + i]));
        assert_eq!(q.blocked.iter().filter(|&&b| b).count(), 2 * 17);
        q.blocked = vec![false; n];
        let edge = sample_metres(1, 17) + SPACING / 2.0;
        q.block(&[[[edge - 0.5, 5.0], [edge + 0.0005, 5.0], [edge - 0.5, 5.3]]]);
        assert_eq!((0..n).filter(|&k| q.blocked[k]).collect::<Vec<_>>(), vec![8 * 17 + 1]);
        let bytes = p.floor_file();
        assert_eq!(bytes.len(), FLOOR_HEADER + 3 * n);
        assert_eq!(&bytes[..4], b"NBFL");
        let f = FloorFile::read(&bytes).unwrap();
        assert_eq!((f.per_tile, f.nx, f.ny), (16, 17, 17));
        // Walkable: the floor's height, in centimetres; elsewhere the probe's ground.
        assert_eq!((f.heights[f.index(0, 0)], f.flags[f.index(0, 0)]), (123, FLOOR_WALKABLE));
        assert_eq!((f.heights[f.index(9, 0)], f.flags[f.index(9, 0)]), (-300, 0));
        assert_eq!(f.flags[f.index(4, 5)], FLOOR_WALKABLE | FLOOR_BLOCKED);
        assert!(f.open(2, 2) && !f.open(3, 3) && !f.open(9, 9));
        assert!(FloorFile::read(&bytes[..bytes.len() - 1]).is_err());
        let mut other = bytes.clone();
        other[4] = 9;
        assert!(FloorFile::read(&other).is_err());
    }

    #[test]
    fn a_walkable_patch_smaller_than_a_sample_still_bounds_the_fill() {
        let q = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| [Vec3::from_array(a), Vec3::from_array(b), Vec3::from_array(c)];
        // Ground at 1 m along the west edge, a 0.4 m patch at 4 m in the east, between them nothing walkable (a cliff): no sample
        // lands on the patch, but the fill from it meets the fill from the west halfway, as in the reference grid.
        let walk = vec![
            q([-5.0, -5.0, 1.0], [-4.0, -5.0, 1.0], [-4.0, 5.0, 1.0]),
            q([-5.0, -5.0, 1.0], [-4.0, 5.0, 1.0], [-5.0, 5.0, 1.0]),
            q([4.5, -5.0, 4.0], [4.9, -5.0, 4.0], [4.9, 5.0, 4.0]),
            q([4.5, -5.0, 4.0], [4.9, 5.0, 4.0], [4.5, 5.0, 4.0]),
        ];
        let (h, _) = probe_tile(&ProbeTile { z: 0.0, walk, wok: Vec::new(), faces: Vec::new() }, 16, 16);
        let row: Vec<f32> = (0..16).map(|i| h[8 * 16 + i]).collect();
        assert!(row.iter().all(|&z| z == 1.0 || z == 4.0 || (z > 1.0 && z < 4.0)), "{row:?}");
        assert_eq!((row[0], row[15]), (1.0, 4.0), "{row:?}");
        // (They meet at x = 0.25 m: samples 0 to 8, up to x = 0.01 m, are the west's.)
        assert_eq!(row.iter().filter(|&&z| z == 1.0).count(), 9, "{row:?}");
    }

    #[test]
    fn the_fill_gives_what_the_reference_grid_s_pass_by_pass_fill_gives() {
        // The reference grid's loop: every pass, each empty cell takes the mean of its filled neighbours as they were before the pass.
        fn by_passes(v: &mut [f32], n: usize, passes: usize) {
            for _ in 0..passes {
                let before = v.to_vec();
                let (mut left, mut filled) = (false, false);
                for j in 0..n {
                    for i in 0..n {
                        if !before[j * n + i].is_nan() {
                            continue;
                        }
                        let near: Vec<f32> = [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)]
                            .iter()
                            .filter(|(x, y)| *x < n && *y < n)
                            .map(|(x, y)| before[y * n + x])
                            .filter(|v| !v.is_nan())
                            .collect();
                        if near.is_empty() {
                            left = true;
                        } else {
                            v[j * n + i] = near.iter().sum::<f32>() / near.len() as f32;
                            filled = true;
                        }
                    }
                }
                if !left || !filled {
                    break;
                }
            }
        }
        let n = 40;
        let mut seed = 12345u32;
        let grid: Vec<f32> = (0..n * n)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                if (seed >> 16).is_multiple_of(9) { ((seed >> 8) % 1000) as f32 / 100.0 } else { f32::NAN }
            })
            .collect();
        let bits = |v: &[f32]| v.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        for (passes, cap) in [(usize::MAX, None), (3, Some(3))] {
            let (mut expected, mut got) = (grid.clone(), grid.clone());
            by_passes(&mut expected, n, passes);
            fill(&mut got, n, n, cap, |v: &f32| !v.is_nan(), |near: &[f32]| near.iter().sum::<f32>() / near.len() as f32);
            assert_eq!(bits(&got), bits(&expected), "{cap:?}");
        }
        // (Three passes leave some cells empty here.)
        let mut three = grid.clone();
        fill(&mut three, n, n, Some(3), |v: &f32| !v.is_nan(), |near: &[f32]| near.iter().sum::<f32>() / near.len() as f32);
        assert!(three.iter().any(|v| v.is_nan()));
    }
}
