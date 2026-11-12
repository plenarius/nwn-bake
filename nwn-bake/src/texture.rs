// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! What a mesh's texture name means, as the game reads it, in one place.
//!
//! A name is first looked up as a material: an EE `.mtr` of that name wins, and its `texture0` is the image (tilesets name materials:
//! tii01's `tii01_brup_flr2`, mst03's `mtr_nkey_0177`). Else the name is the image. The image is then a DDS or TGA by resource
//! priority ([`mg_resman::ResMan::texture`]: within each layer class a DDS beats a TGA), or a PLT. A coloured part (a creature's or
//! an item's) that no material names takes its PLT before a plain image of the same name. This is the rule Moonglow's renderer
//! uses (`mg_render::Assets for ResMan`), so a picture drawn by it and data written here agree.

use std::borrow::Cow;

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_image::mtr::Mtr;
use mg_resman::ResKey;
use mg_rules::GameData;

/// The material (`.mtr`) named `name`, if the game has one.
pub fn material(game: &GameData, name: &str) -> Option<Mtr> {
    let resref = ResRef::cut(name)?;
    game.resman.get(&ResKey::new(resref, ResType::MTR)).ok().map(|d| Mtr::parse(&d))
}

/// Whether an image is a plain one (DDS or TGA) or a PLT (coloured by layers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A PLT: layers the game colours per creature or item.
    Plt,
    /// A plain image: a DDS or TGA.
    Plain,
}

/// A texture name, resolved.
pub struct Resolved {
    /// The image the name means: the material's `texture0` when a material of that name names another, else the name itself.
    pub image: ResRef,
    /// The material of that name, if there is one.
    pub material: Option<Mtr>,
}

/// Resolves texture name `name` (see the module). None when the name cannot be a resource name, or a material of that name names an
/// image that cannot be.
pub fn resolve(game: &GameData, name: &str) -> Option<Resolved> {
    let resref = ResRef::cut(name)?;
    let material = game.resman.get(&ResKey::new(resref, ResType::MTR)).ok().map(|d| Mtr::parse(&d));
    let image = match material.as_ref().and_then(|m| m.textures[0].as_deref()) {
        Some(t) if !t.eq_ignore_ascii_case(name) => ResRef::cut(t)?,
        _ => resref,
    };
    Some(Resolved { image, material })
}

impl Resolved {
    /// The image's name, lower case.
    pub fn image_name(&self) -> String {
        self.image.to_string().to_ascii_lowercase()
    }

    /// The image as a DDS or TGA, whichever resource priority finds first, and its bytes.
    pub fn plain<'a>(&self, game: &'a GameData) -> Option<(ResType, Cow<'a, [u8]>)> {
        game.resman.texture(self.image)
    }

    /// The image as a PLT, its bytes.
    pub fn plt<'a>(&self, game: &'a GameData) -> Option<Cow<'a, [u8]>> {
        game.resman.get(&ResKey::new(self.image, ResType::PLT)).ok()
    }

    /// The image's TXI (cut-outs, blending, environment maps), its bytes.
    pub fn txi<'a>(&self, game: &'a GameData) -> Option<Cow<'a, [u8]>> {
        game.resman.get(&ResKey::new(self.image, ResType::TXI)).ok()
    }

    /// Whether the image the game draws is plain or a PLT; None when it has neither. `colored`: for a coloured part, whose PLT comes
    /// first unless a material names its image.
    pub fn kind(&self, game: &GameData, colored: bool) -> Option<Kind> {
        let has_plt = || self.plt(game).is_some();
        let has_plain = || self.plain(game).is_some();
        if colored && self.material.is_none() {
            if has_plt() {
                Some(Kind::Plt)
            } else if has_plain() {
                Some(Kind::Plain)
            } else {
                None
            }
        } else if has_plain() {
            Some(Kind::Plain)
        } else if has_plt() {
            Some(Kind::Plt)
        } else {
            None
        }
    }
}

/// The plain image (DDS or TGA) texture name `name` means, and its bytes; None when there is none.
pub fn image<'a>(game: &'a GameData, name: &str) -> Option<(ResType, Cow<'a, [u8]>)> {
    resolve(game, name)?.plain(game)
}

/// The plain image texture name `name` means, decoded (rows from the bottom, as the game stores them: v up).
pub fn rgba(game: &GameData, name: &str) -> Result<mg_image::Rgba> {
    let (restype, data) = image(game, name).with_context(|| format!("no texture {name}"))?;
    Ok(mg_image::read(restype, &data)?.to_rgba())
}

/// A texture as PNG (rows top first): the plain image `name` means, or `own` (a type and its bytes) when the caller has one.
pub fn png(game: &GameData, name: &str, own: Option<(ResType, Vec<u8>)>) -> Result<Vec<u8>> {
    let (restype, data) =
        own.or_else(|| image(game, name).map(|(t, d)| (t, d.into_owned()))).with_context(|| format!("no texture {name}"))?;
    let rgba = mg_image::read(restype, &data)?.to_rgba().top_down();
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, rgba.width, rgba.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&rgba.data)?;
    }
    Ok(out)
}

/// One level of a [`Mipped`] image: RGBA, four bytes a texel, rows from the bottom (v up, as the game stores them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    /// Width in texels.
    pub width: usize,
    /// Height in texels.
    pub height: usize,
    /// The texels: RGBA, four bytes each, `width * height * 4` bytes, rows from the bottom.
    pub rgba: Vec<u8>,
}

/// An image decoded with its mip chain (each level half the one before, down to one texel), for drawing on the CPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mipped {
    /// The image the texture name means (a material's `texture0`, else the name), lower case.
    pub image: String,
    /// The levels, largest first: level 0 is the image itself.
    pub levels: Vec<Level>,
    /// Whether any texel is see-through (a cut-out: leaves, fences).
    pub cutout: bool,
}

impl Mipped {
    /// The mip chain of `rgba` (rows from the bottom). A level's texel is the average of the four under it, its colour weighted by
    /// alpha so a cut-out's edge does not turn dark.
    pub fn new(image: String, rgba: &mg_image::Rgba) -> Mipped {
        let (w, h) = (rgba.width.max(1) as usize, rgba.height.max(1) as usize);
        let mut base = rgba.data.clone();
        base.resize(w * h * 4, 0);
        let mut levels = vec![Level { width: w, height: h, rgba: base }];
        while levels.last().is_some_and(|l| l.width > 1 || l.height > 1) {
            let l = levels.last().unwrap();
            let (nw, nh) = ((l.width / 2).max(1), (l.height / 2).max(1));
            let mut out = Vec::with_capacity(nw * nh * 4);
            for y in 0..nh {
                for x in 0..nw {
                    let (mut sum, mut n) = ([0u32; 4], 0u32);
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(l.width - 1), (y * 2 + dy).min(l.height - 1));
                        let p = &l.rgba[(sy * l.width + sx) * 4..][..4];
                        for c in 0..3 {
                            sum[c] += u32::from(p[c]) * u32::from(p[3]);
                        }
                        sum[3] += u32::from(p[3]);
                        n += 1;
                    }
                    let a = sum[3] as f32;
                    out.extend(if a > 0.0 {
                        [(sum[0] as f32 / a) as u8, (sum[1] as f32 / a) as u8, (sum[2] as f32 / a) as u8, (sum[3] / n) as u8]
                    } else {
                        [0; 4]
                    });
                }
            }
            levels.push(Level { width: nw, height: nh, rgba: out });
        }
        let cutout = levels[0].rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 250);
        Mipped { image, levels, cutout }
    }

    /// Whether the image is black all over, as a tileset's void filler is (`tdm01_black`, `tbw01_black1`, `black`): every texel of
    /// its first level 32 texels or smaller has a colour whose channels add up to 15 or less (the floor probe's test for void).
    pub fn is_black(&self) -> bool {
        self.levels[self.level_within(32)].rgba.as_chunks::<4>().0.iter().all(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) <= 15)
    }

    /// The first level no wider and no higher than `size` texels (the last level when none is).
    pub fn level_within(&self, size: usize) -> usize {
        self.levels.iter().position(|l| l.width <= size && l.height <= size).unwrap_or(self.levels.len() - 1)
    }

    /// The colour (0..255 a channel) at (u, v) on level `lod` (rounded, clamped to the levels there are), blended between the four
    /// nearest texels; the texture repeats, or with `clamp` stops at its edges.
    pub fn sample(&self, u: f32, v: f32, lod: f32, clamp: bool) -> [f32; 4] {
        let level = &self.levels[(lod.round().max(0.0) as usize).min(self.levels.len() - 1)];
        let (fu, fv) = if clamp { (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0)) } else { (u - u.floor(), v - v.floor()) };
        let x = fu * level.width as f32 - 0.5;
        let y = fv * level.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (ax, ay) = (x - x0, y - y0);
        let wrap =
            |i: f32, n: usize| -> usize { if clamp { (i.max(0.0) as usize).min(n - 1) } else { (i as i64).rem_euclid(n as i64) as usize } };
        let (xa, xb, ya, yb) = (wrap(x0, level.width), wrap(x0 + 1.0, level.width), wrap(y0, level.height), wrap(y0 + 1.0, level.height));
        let texel = |x: usize, y: usize| &level.rgba[(y * level.width + x) * 4..][..4];
        let mut out = [0f32; 4];
        for (p, w) in [
            (texel(xa, ya), (1.0 - ax) * (1.0 - ay)),
            (texel(xb, ya), ax * (1.0 - ay)),
            (texel(xa, yb), (1.0 - ax) * ay),
            (texel(xb, yb), ax * ay),
        ] {
            for c in 0..4 {
                out[c] += f32::from(p[c]) * w;
            }
        }
        out
    }
}

/// The plain image texture name `name` means ([`resolve`]), decoded with its mip chain.
pub fn mipped(game: &GameData, name: &str) -> Result<Mipped> {
    let r = resolve(game, name).with_context(|| format!("no texture {name}"))?;
    let (restype, data) = r.plain(game).with_context(|| format!("no texture {name}"))?;
    let rgba = mg_image::read(restype, &data).with_context(|| format!("texture {name}: the image can't be read"))?.to_rgba();
    Ok(Mipped::new(r.image_name(), &rgba))
}
