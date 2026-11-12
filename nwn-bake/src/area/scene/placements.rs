// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The scene's tiles, placeables and doors as placed copies of models, for outputs that keep each model once (`area-3d`'s glTF)
//! rather than every triangle in the area's space.
//!
//! [`placements`] reads the area the way [`scene`] does, with the scene's own code (the `.are`'s tiles, the tileset's models, the
//! quarter turns, the `.git`'s appearance rows, the flattened models, the water rule and the stray-geometry rule), and keeps what
//! the scene throws away: which model each tile or object is, and how it is turned and moved. Every model is kept once in its own
//! space; a placement whose copy loses some triangles as stray geometry names them ([`Placement::dropped`]), so
//! [`Placements::expand`] gives back the scene's triangles exactly, bit for bit and in its order, which [`Placements::check`]
//! confirms.
//!
//! [`scene`]: crate::area::scene()
//! [`Placement::dropped`]: crate::area::scene::placements::Placement::dropped
//! [`Placements::expand`]: crate::area::scene::placements::Placements::expand
//! [`Placements::check`]: crate::area::scene::placements::Placements::check

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use glam::Vec3;
use mg_rules::ObjectKind;
use mg_rules::appearance::AppearanceRow;

use super::{HIGHEST, LOWEST, MARGIN, Part, Scene, SceneOptions, Triangle, for_each_triangle, is_water, quarter_turns, read_tiles};
use crate::area::floor::TILE;
use crate::game::Game;
use crate::module::AreaFiles;

/// A model's triangle in the model's own space (x east, y north, z up, as it faces at bearing 0 or with no turn), and its
/// texture's number in the scene's [`Scene::textures`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelTriangle {
    /// Its three corners in model space, metres.
    pub p: [[f32; 3]; 3],
    /// Its corners' texture coordinates (the mesh's first UV set).
    pub uv: [[f32; 2]; 3],
    /// Its texture's index in [`Scene::textures`].
    pub texture: u32,
}

/// A model the area places, as the scene uses it.
#[derive(Clone, Debug)]
pub struct PlacedModel {
    /// The model's resref, lower case.
    pub name: String,
    /// Whether tiles place it. A tile's water faces are the scene's water, not its triangles; a placeable's or door's are drawn as
    /// they are (the scene's rules), so one model used both ways is two entries.
    pub tile: bool,
    /// Its triangles, in the flattened model's order (a tile's without its water faces).
    pub triangles: Vec<ModelTriangle>,
}

/// How a placement turns and moves its model into the area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    /// A tile: this many quarter turns counter-clockwise (exact), then moved to the tile's middle at its height.
    Quarters(u8),
    /// A placeable or door: turned by its `.git` `Bearing` (radians, counter-clockwise), then moved.
    Bearing(f32),
}

/// One tile, placeable or door: its model, how it is turned, where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// What it places: a tile, a placeable or a door.
    pub part: Part,
    /// Its model's index in [`Placements::models`].
    pub model: usize,
    /// How its model is turned.
    pub turn: Turn,
    /// Where the model's origin goes, game metres: a tile's middle at its height, an object's `.git` X, Y and Z.
    pub position: [f32; 3],
    /// The model's triangles (indices into [`PlacedModel::triangles`]) the scene leaves out here as stray geometry (far outside the
    /// area), ascending; empty for almost every placement.
    pub dropped: Vec<u32>,
}

impl Placement {
    /// A point of the model in the area, computed exactly as the scene computes it (the same operations in the same order).
    pub fn place(&self, v: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = self.position;
        match self.turn {
            Turn::Quarters(n) => {
                let local = quarter_turns(Vec3::from_array(v), n) + Vec3::new(0.0, 0.0, z);
                [local.x + x, local.y + y, local.z]
            }
            Turn::Bearing(b) => {
                let (cb, sb) = (b.cos(), b.sin());
                [x + v[0] * cb - v[1] * sb, y + v[0] * sb + v[1] * cb, z + v[2]]
            }
        }
    }

    /// The turn as an angle about the vertical, radians counter-clockwise.
    pub fn angle(&self) -> f32 {
        match self.turn {
            Turn::Quarters(n) => f32::from(n % 4) * std::f32::consts::FRAC_PI_2,
            Turn::Bearing(b) => b,
        }
    }
}

/// The area's placements (see the module).
#[derive(Clone, Debug, Default)]
pub struct Placements {
    /// Each model placed, once: a placement refers to one by its index.
    pub models: Vec<PlacedModel>,
    /// The tiles that have a model, in the `.are`'s order, then the `.git`'s placeables and doors whose model was read, in its order
    /// (placeables first): the scene's order.
    pub placements: Vec<Placement>,
}

impl Placements {
    /// Every placement's triangles in the area, in the scene's order: what [`Scene::triangles`] holds (but for
    /// [`Triangle::chasm`], which is the pictures' and is false here).
    pub fn expand(&self) -> Vec<Triangle> {
        let mut out = Vec::new();
        for pl in &self.placements {
            let model = &self.models[pl.model];
            let mut dropped = pl.dropped.iter().peekable();
            for (k, t) in model.triangles.iter().enumerate() {
                if dropped.peek().is_some_and(|&&d| d as usize == k) {
                    dropped.next();
                    continue;
                }
                out.push(Triangle { p: t.p.map(|v| pl.place(v)), uv: t.uv, texture: t.texture, part: pl.part, chasm: false });
            }
        }
        out
    }

    /// Fails unless [`Placements::expand`] gives `scene`'s triangles exactly (positions and UVs bit for bit, textures, parts, order).
    pub fn check(&self, scene: &Scene) -> Result<()> {
        let mine = self.expand();
        if mine.len() != scene.triangles.len() {
            bail!("the placements give {} triangles, the scene has {}", mine.len(), scene.triangles.len());
        }
        let bits = |t: &Triangle| (t.p.map(|v| v.map(f32::to_bits)), t.uv.map(|v| v.map(f32::to_bits)), t.texture, t.part);
        if let Some(i) = mine.iter().zip(&scene.triangles).position(|(a, b)| bits(a) != bits(b)) {
            bail!("the placements' triangle {i} differs from the scene's: {:?} against {:?}", mine[i], scene.triangles[i]);
        }
        Ok(())
    }
}

/// Area `area`'s placements, read as [`super::scene`](crate::area::scene()) read `scene` (built from the same `game`, `area` and options `o`), with
/// `scene`'s texture numbers. Models and appearances that the scene could not read are left out here too (the scene's notes say
/// why).
pub fn placements(game: &Game, area: &AreaFiles, o: &SceneOptions, scene: &Scene) -> Result<Placements> {
    let are = &area.are.root;
    let tileset_ref =
        are.resref("Tileset").filter(|r| !r.is_empty()).ok_or_else(|| anyhow::anyhow!("{}: the area has no tileset", area.resref))?;
    let read = crate::area::tileset(game, tileset_ref)?;
    let tiles = read_tiles(are, &read.tileset, &mut Vec::new())?;
    let (sx, sy) = (scene.tiles[0] as f32 * TILE, scene.tiles[1] as f32 * TILE);
    let inside =
        |p: &[f32; 3]| p[0] > -MARGIN && p[0] < sx + MARGIN && p[1] > -MARGIN && p[1] < sy + MARGIN && p[2] > LOWEST && p[2] < HIGHEST;
    let ids: BTreeMap<&str, u32> = scene.textures.iter().enumerate().map(|(i, t)| (t.name.as_str(), i as u32)).collect();
    let id = |name: &str| ids.get(name).copied().ok_or_else(|| anyhow::anyhow!("texture {name} is not in the scene"));

    let mut out = Placements::default();
    let mut index: BTreeMap<(String, bool), Option<usize>> = BTreeMap::new();
    // The model's index, read the first time (None when the game has no such model: the scene noted it).
    let mut model_of = |name: &str, tile: bool, out: &mut Placements| -> Result<Option<usize>> {
        if let Some(&m) = index.get(&(name.to_string(), tile)) {
            return Ok(m);
        }
        let m = match game.flat(name) {
            Ok(flat) => {
                let mut triangles = Vec::new();
                let mut failed = None;
                for_each_triangle(&flat, |texture, p, uv| {
                    if tile && is_water(texture) {
                        return;
                    }
                    match id(texture) {
                        Ok(texture) => triangles.push(ModelTriangle { p, uv, texture }),
                        Err(e) => failed = Some(e),
                    }
                });
                if let Some(e) = failed {
                    return Err(e);
                }
                out.models.push(PlacedModel { name: name.to_string(), tile, triangles });
                Some(out.models.len() - 1)
            }
            Err(_) => None,
        };
        index.insert((name.to_string(), tile), m);
        Ok(m)
    };
    let place = |out: &mut Placements, part: Part, model: usize, turn: Turn, position: [f32; 3]| {
        let mut pl = Placement { part, model, turn, position, dropped: Vec::new() };
        pl.dropped = out.models[model]
            .triangles
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.p.iter().all(|v| inside(&pl.place(*v))))
            .map(|(k, _)| k as u32)
            .collect();
        out.placements.push(pl);
    };

    for (i, t) in tiles.iter().enumerate() {
        let Some(name) = &t.model else { continue };
        let Some(m) = model_of(name, true, &mut out)? else {
            continue;
        };
        place(&mut out, Part::Tile(i as u32), m, Turn::Quarters(t.orientation), [t.position.x, t.position.y, t.position.z]);
    }
    if let Some(git) = area.git.as_ref().filter(|_| o.objects) {
        for (list, kind) in [("Placeable List", ObjectKind::Placeable), ("Door List", ObjectKind::Door)] {
            for (i, s) in git.root.list(list).unwrap_or(&[]).iter().enumerate() {
                let Some(at) = AppearanceRow::of(kind, s) else {
                    continue;
                };
                let table = game.table(at.table).ok();
                let Ok(Some(model)) = at.model(table.as_deref(), &game.resman) else {
                    continue;
                };
                let Some(m) = model_of(&model, false, &mut out)? else {
                    continue;
                };
                let f = |l: &str| s.float(l).unwrap_or(0.0);
                let part = if matches!(kind, ObjectKind::Door) { Part::Door(i as u32) } else { Part::Placeable(i as u32) };
                place(&mut out, part, m, Turn::Bearing(f("Bearing")), [f("X"), f("Y"), f("Z")]);
            }
        }
    }
    Ok(out)
}
