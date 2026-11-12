// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An area's scene, straight from its files: the tile models' triangles (water apart), the placeables and doors, the floor probe,
//! and the area's size, flags and light. What an area's picture is drawn from.
//!
//! The scene is built in a fixed order (the tiles as the `.are` lists them, each model's meshes in the model's order, then the
//! `.git`'s placeables and doors in its order; a texture's number is the order it was first used in), and nothing in it depends on
//! a hash map's order or on which thread finishes first (textures are decoded on several threads, each into its own place). The
//! same inputs give the same scene, bit for bit: [`Scene::digest`] is the same, which is what a cache keyed by the scene relies on.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use glam::Vec3;
use mg_core::ResType;
use mg_mdl::NodeKind;
use mg_mdl::walkmesh::{Walkmesh, WalkmeshKind};
use mg_rules::ObjectKind;
use mg_rules::appearance::AppearanceRow;
use sha2::{Digest, Sha256};

use super::floor::{FloorProbe, ProbeTile, RoofCut, RoofRule, TILE};
use crate::game::{Game, ResKind};
use crate::module::AreaFiles;
use crate::sync::Recover;
use crate::texture::Mipped;

/// Walkmesh surface: deep water (its bed counts as the floor).
const DEEP_WATER: u32 = 17;
/// Geometry this far outside the area, or below `LOWEST` or above `HIGHEST`, is stray (some tiles carry meshes hundreds of metres
/// away) and left out.
const MARGIN: f32 = 15.0;
const LOWEST: f32 = -60.0;
const HIGHEST: f32 = 80.0;

/// A chasm's floor (see [`Triangle::chasm`]) lies at least this far below its tile's floor, in metres.
pub const CHASM_DEPTH: f32 = 1.0;
/// A chasm's floor is at least this level: the vertical part of its unit normal, facing up or down.
pub const CHASM_LEVEL: f32 = 0.95;
/// A chasm's floor is no brighter than this where it is drawn: a texel's channels added up, 0..765.
pub const CHASM_DARK: f32 = 30.0;
/// A walkable walkmesh face this near a face's height (metres), over or under it, is where it is walked: not a chasm's floor.
const CHASM_STAND: f32 = 1.0;

/// What to put in a scene.
#[derive(Clone, Debug)]
pub struct SceneOptions {
    /// The `.git`'s placeables and doors.
    pub objects: bool,
    /// How many threads decode the textures: 0 is as many as there are cores, at most 8. (A batch drawing several areas at once
    /// gives each fewer.) The scene is the same whatever the number.
    pub threads: usize,
}

impl Default for SceneOptions {
    fn default() -> Self {
        SceneOptions { objects: true, threads: 0 }
    }
}

/// What a triangle belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The tile at this index of the `.are`'s `Tile_List`.
    Tile(u32),
    /// The placeable at this index of the `.git`'s `Placeable List`.
    Placeable(u32),
    /// The door at this index of the `.git`'s `Door List`.
    Door(u32),
}

/// A triangle to draw: positions in game metres (x east, y north, z up, the area's south-west corner at 0), UVs as the model has them
/// (v up), and its texture's number in [`Scene::textures`].
#[derive(Clone, Debug, PartialEq)]
pub struct Triangle {
    /// Its three corners, in game metres.
    pub p: [[f32; 3]; 3],
    /// Its corners' texture coordinates, as the model has them (v up).
    pub uv: [[f32; 2]; 3],
    /// Its texture's index in [`Scene::textures`].
    pub texture: u32,
    /// The tile, placeable or door it comes from.
    pub part: Part,
    /// A chasm's floor ([`is_chasm_floor`] on a tile with chasm terrain, [`is_chasm_terrain`]): the tile's black bottom of a drop,
    /// which the pictures draw in their own colour.
    pub chasm: bool,
}

/// A texture name the scene's meshes use, and its image (None when the game has none: the triangles are drawn without one).
#[derive(Clone, Debug)]
pub struct SceneTexture {
    /// The texture name, lower case.
    pub name: String,
    /// The decoded image, with its mip chain.
    pub image: Option<Arc<Mipped>>,
}

/// Whether the roof is cut: an interior or underground area is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Neither flag set, on a tileset that is not an interior one: the roof stays.
    Surface,
    /// The `.are`'s interior flag (1), or an interior tileset with neither flag set.
    Interior,
    /// The `.are`'s underground flag (2), without the interior flag.
    Underground,
}

/// The area's sun (its `SunAmbientColor` and `SunDiffuseColor`), RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Light {
    /// `SunAmbientColor`: the light everything gets, whichever way it faces.
    pub ambient: [u8; 3],
    /// `SunDiffuseColor`: the sun's light on faces turned towards it.
    pub diffuse: [u8; 3],
}

/// How long each part of building the scene took.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    /// Reading the tileset and the area's tiles.
    pub area: Duration,
    /// Flattening the tile models and reading their walkmeshes.
    pub tiles: Duration,
    /// Flattening the placeables' and doors' models.
    pub objects: Duration,
    /// Decoding the textures.
    pub textures: Duration,
    /// The floor probe.
    pub probe: Duration,
}

/// What a scene holds, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Counts {
    /// Every tile of the area, with a model or not: width times height.
    pub tiles: usize,
    /// Distinct tile models.
    pub tile_models: usize,
    /// The tiles' triangles, water apart, before stray geometry is left out.
    pub tile_triangles: usize,
    /// Placeables whose model was read.
    pub placeables: usize,
    /// Doors whose model was read.
    pub doors: usize,
    /// The placeables' and doors' triangles, before stray geometry is left out.
    pub object_triangles: usize,
    /// The tiles' water triangles that are in the scene (stray ones apart).
    pub water_triangles: usize,
    /// Triangles left out as stray geometry.
    pub stray: usize,
    /// Distinct texture names in the scene.
    pub textures: usize,
    /// Texture names the game has no image for.
    pub textures_missing: usize,
    /// Tile triangles that are a chasm's floor ([`Triangle::chasm`]).
    pub chasm_triangles: usize,
}

/// An area's scene (see the module).
#[derive(Clone, Debug)]
pub struct Scene {
    /// The area's resref, lower case.
    pub resref: String,
    /// Its tileset's resref, lower case.
    pub tileset: String,
    /// Tiles along x (east) and y (north).
    pub tiles: [u32; 2],
    /// The `.are`'s `Flags`: 1 interior, 2 underground, 4 natural.
    pub flags: u32,
    /// The tileset says it is an interior one (`Interior=1`).
    pub tileset_interior: bool,
    /// The area's sun.
    pub light: Light,
    /// Whether the `.git`'s placeables and doors were asked for ([`SceneOptions::objects`]).
    pub objects: bool,
    /// Every texture name the triangles use, by the number they give it ([`Triangle::texture`]): in the order first used.
    pub textures: Vec<SceneTexture>,
    /// What is drawn: the tiles' faces (water apart), then the placeables and doors.
    pub triangles: Vec<Triangle>,
    /// The tiles' water faces (textures named water, not waterfalls or water cliffs): drawn as one translucent colour.
    pub water: Vec<[[f32; 3]; 3]>,
    /// The ground's heights and colours on a 0.625 m grid, for the roof cut, and the walkable floor (`area-3d`'s floor file).
    pub probe: FloorProbe,
    /// What the scene holds, counted.
    pub counts: Counts,
    /// What could not be read or was odd (a tileset's absent sections, missing models and textures), in the order met.
    pub notes: Vec<String>,
    /// Every resource the scene looked up in the game, found or not, by name (lower case) and how it was looked up, sorted: the
    /// tileset, the tile models and their walkmeshes, the placeables' and doors' tables, models and walkmeshes, `surfacemat`, and
    /// for each texture name its material and its image. What [`Game::resources_digest`] hashes for the exact cache key: the same
    /// lookups finding the same bytes give the same scene.
    pub reads: Vec<(String, ResKind)>,
    /// How long each part of building the scene took. Not part of [`Scene::digest`].
    pub timings: Timings,
}

/// A tile as placed, for the probe: its height, its walkmesh's faces the probe reads (walkable and deep water's), every face of its
/// walkmesh with whether it walks, and its drawn faces with their UVs and texture numbers.
type Placed = (f32, Vec<[Vec3; 3]>, Vec<([Vec3; 3], bool)>, Vec<([Vec3; 3], [[f32; 2]; 3], u32)>);

/// A tile of the area, as its `.are` places it.
struct Tile {
    /// The tileset's model for it, lower case; None for a tile the tileset does not have or has as a hole.
    model: Option<String>,
    /// Quarter turns counter-clockwise.
    orientation: u8,
    /// The middle of its floor: (column + 0.5, row + 0.5) tiles, its height steps times the tileset's `Transition`.
    position: Vec3,
    /// The tileset calls one of its corners a chasm ([`is_chasm_terrain`]).
    chasm: bool,
}

/// The most tiles an area has along a side: the toolset makes areas of 1 to 32 tiles a side.
pub const MAX_TILES: usize = 32;

/// The `.are`'s `Width` and `Height` in tiles. Fails past [`MAX_TILES`]: an `.are` claiming 1,024 by 1,024 would otherwise build a
/// million tiles and stall a batch for minutes.
fn area_size(are: &mg_gff::Struct) -> Result<(usize, usize)> {
    let side = |label: &str| -> Result<usize> {
        let n = are.integer(label).unwrap_or(0);
        usize::try_from(n).ok().filter(|&n| n <= MAX_TILES).with_context(|| format!("{label} is {n} tiles; an area has 0 to {MAX_TILES}"))
    };
    Ok((side("Width")?, side("Height")?))
}

/// The `.are`'s `Tile_List`, row by row from the south-west, as the game reads it (as Moonglow's `mg_area::AreaModel` does; that
/// crate is not used here because it brings the GPU renderer with it). What is odd is noted; a list of another length than
/// `Width` by `Height` fails.
fn read_tiles(are: &mg_gff::Struct, tileset: &mg_set::Tileset, notes: &mut Vec<String>) -> Result<Vec<Tile>> {
    let (width, height) = area_size(are)?;
    let list = are.list("Tile_List").unwrap_or(&[]);
    ensure!(list.len() == width * height, "Tile_List has {} tiles for a {width} by {height} area", list.len());
    let (mut unknown, mut holes) = (0, Vec::new());
    let tiles: Vec<Tile> = list
        .iter()
        .take(width * height)
        .enumerate()
        .map(|(index, t)| {
            let int = |label: &str| t.integer(label).unwrap_or(0);
            let id = int("Tile_ID");
            let tile = usize::try_from(id).ok().and_then(|i| tileset.tiles.get(i));
            if tile.is_none() {
                unknown += 1;
            }
            // (A hole in the tileset: nothing, as in the game.)
            if tile.is_some_and(mg_set::Tile::is_hole) && !holes.contains(&id) {
                holes.push(id);
            }
            let (column, row) = (index % width.max(1), index / width.max(1));
            Tile {
                model: tile.filter(|t| !t.is_hole()).map(|t| t.model.to_ascii_lowercase()),
                orientation: int("Tile_Orientation").rem_euclid(4) as u8,
                chasm: tile.is_some_and(|t| t.corners.iter().any(|(terrain, _)| is_chasm_terrain(terrain))),
                position: Vec3::new(
                    (column as f32 + 0.5) * TILE,
                    (row as f32 + 0.5) * TILE,
                    int("Tile_Height").clamp(-1024, 1024) as f32 * tileset.general.transition,
                ),
            }
        })
        .collect();
    if unknown > 0 {
        notes.push(format!("{unknown} tiles are not in the tileset"));
    }
    if !holes.is_empty() {
        holes.sort_unstable();
        let named: Vec<String> = holes.iter().map(|t| format!("tile {t}")).collect();
        notes.push(format!("the tileset names no model for {} (its section or Model= is missing)", named.join(", ")));
    }
    Ok(tiles)
}

/// A tile model's parts, in the model's space (centred on the tile).
struct TileModel {
    flat: Option<Arc<crate::Flat>>,
    /// Its walkmesh's faces the game walks, and deep water's.
    walk: Vec<[Vec3; 3]>,
    /// Every face of its walkmesh, and whether the game walks it.
    wok: Vec<([Vec3; 3], bool)>,
    /// For each of the model's triangles, in its order: whether its shape and place make it a chasm's floor (all of
    /// [`is_chasm_floor`] but the texture's colour, which is looked at once the textures are decoded).
    chasm: Vec<bool>,
}

/// Builds area `area`'s scene from the game `game` (opened with the area's module's haks).
pub fn scene(game: &Game, area: &AreaFiles, o: &SceneOptions) -> Result<Scene> {
    let mut timings = Timings::default();
    let mut notes = Vec::new();
    let started = Instant::now();
    let are = &area.are.root;
    let tileset_ref =
        are.resref("Tileset").filter(|r| !r.is_empty()).with_context(|| format!("{}: the area has no tileset", area.resref))?;
    let mut reads: BTreeSet<(String, ResKind)> = BTreeSet::new();
    reads.insert((tileset_ref.to_string().to_ascii_lowercase(), ResKind::Set));
    let read = super::tileset(game, tileset_ref).with_context(|| format!("{}: its tileset", area.resref))?;
    notes.extend(read.notes(&tileset_ref.to_string().to_ascii_lowercase()));
    let tiles = read_tiles(are, &read.tileset, &mut notes).with_context(|| format!("{}: its tiles", area.resref))?;
    let (w, h) = area_size(are)?;
    timings.area = started.elapsed();

    let started = Instant::now();
    reads.insert(("surfacemat".into(), ResKind::TwoDa));
    let surfacemat = game.table("surfacemat").ok();
    let stands = |m: u32| surfacemat.as_ref().and_then(|t| t.get(m as usize, "Walk")).is_some_and(|v| v.trim() == "1");
    let walkable = |m: u32| m == DEEP_WATER || stands(m);
    let mut textures = Textures::default();
    let mut triangles = Vec::new();
    let mut water = Vec::new();
    let mut counts = Counts { tiles: tiles.len(), ..Default::default() };
    // Each tile model once, by name (a BTreeMap: looked up, never walked).
    let mut models: BTreeMap<String, TileModel> = BTreeMap::new();
    // Each tile, turned as placed and lifted to its height, in its own square: what the probe reads.
    let mut placed: Vec<Placed> = Vec::with_capacity(tiles.len());
    for (index, t) in tiles.iter().enumerate() {
        let Some(name) = &t.model else {
            placed.push((t.position.z, Vec::new(), Vec::new(), Vec::new()));
            continue;
        };
        let tm = models.entry(name.clone()).or_insert_with(|| {
            // (`flatten` looks for a placeable walkmesh too.)
            reads.extend([ResKind::Mdl, ResKind::Pwk, ResKind::Wok].map(|k| (name.clone(), k)));
            tile_model(game, name, &walkable, &stands, &mut notes)
        });
        let turn = |p: Vec3| quarter_turns(p, t.orientation) + Vec3::new(0.0, 0.0, t.position.z);
        let (cx, cy) = (t.position.x, t.position.y);
        let walk: Vec<[Vec3; 3]> = tm.walk.iter().map(|f| f.map(turn)).collect();
        let wok: Vec<([Vec3; 3], bool)> = tm.wok.iter().map(|(f, w)| (f.map(turn), *w)).collect();
        let mut faces = Vec::new();
        if let Some(flat) = &tm.flat {
            let mut k = 0;
            for_each_triangle(flat, |texture, p, uv| {
                let local = p.map(|v| turn(Vec3::from_array(v)));
                let world = local.map(|v| [v.x + cx, v.y + cy, v.z]);
                if is_water(texture) {
                    water.push(world);
                } else {
                    let id = textures.id(texture);
                    faces.push((local, uv, id));
                    let chasm = t.chasm && tm.chasm.get(k).copied().unwrap_or(false);
                    triangles.push(Triangle { p: world, uv, texture: id, part: Part::Tile(index as u32), chasm });
                }
                k += 1;
            });
        }
        placed.push((t.position.z, walk, wok, faces));
    }
    counts.tile_models = models.len();
    counts.tile_triangles = triangles.len();
    timings.tiles = started.elapsed();

    let started = Instant::now();
    // The placeables' walkmeshes' unwalkable faces, placed (x, y): where they block the floor.
    let mut blocks: Vec<[[f32; 2]; 3]> = Vec::new();
    if let Some(git) = area.git.as_ref().filter(|_| o.objects) {
        for (list, kind) in [("Placeable List", ObjectKind::Placeable), ("Door List", ObjectKind::Door)] {
            for (i, s) in git.root.list(list).unwrap_or(&[]).iter().enumerate() {
                let Some(at) = AppearanceRow::of(kind, s) else { continue };
                reads.insert((at.table.to_string(), ResKind::TwoDa));
                let table = game.table(at.table).ok();
                // (The model the row names is looked for whether it is there or not.)
                let named = table
                    .as_deref()
                    .zip(usize::try_from(at.row).ok())
                    .and_then(|(t, r)| t.get(r, at.column))
                    .map(|m| m.trim().to_ascii_lowercase())
                    .filter(|m| !m.is_empty());
                if let Some(m) = named {
                    reads.extend([ResKind::Mdl, ResKind::Pwk].map(|k| (m.clone(), k)));
                }
                let model = match at.model(table.as_deref(), &game.resman) {
                    Ok(Some(m)) => m,
                    Ok(None) => continue,
                    Err(e) => {
                        notes.push(format!("{list} {i}: {e}"));
                        continue;
                    }
                };
                let flat = match game.flat(&model) {
                    Ok(f) => f,
                    Err(e) => {
                        notes.push(format!("{list} {i}: model {model}: {e:#}"));
                        continue;
                    }
                };
                let f = |l: &str| s.float(l).unwrap_or(0.0);
                let (ox, oy, oz, bearing) = (f("X"), f("Y"), f("Z"), f("Bearing"));
                let (cb, sb) = (bearing.cos(), bearing.sin());
                let part = match kind {
                    ObjectKind::Door => {
                        counts.doors += 1;
                        Part::Door(i as u32)
                    }
                    _ => {
                        counts.placeables += 1;
                        blocks.extend(flat.footprint.iter().map(|t| t.map(|v| [ox + v[0] * cb - v[1] * sb, oy + v[0] * sb + v[1] * cb])));
                        Part::Placeable(i as u32)
                    }
                };
                for_each_triangle(&flat, |texture, p, uv| {
                    let p = p.map(|v| [ox + v[0] * cb - v[1] * sb, oy + v[0] * sb + v[1] * cb, oz + v[2]]);
                    triangles.push(Triangle { p, uv, texture: textures.id(texture), part, chasm: false });
                    counts.object_triangles += 1;
                });
            }
        }
    }
    timings.objects = started.elapsed();

    // Stray geometry far outside the area.
    let (sx, sy) = (w as f32 * TILE, h as f32 * TILE);
    let inside =
        |p: &[f32; 3]| p[0] > -MARGIN && p[0] < sx + MARGIN && p[1] > -MARGIN && p[1] < sy + MARGIN && p[2] > LOWEST && p[2] < HIGHEST;
    let before = triangles.len() + water.len();
    triangles.retain(|t| t.p.iter().all(inside));
    water.retain(|t: &[[f32; 3]; 3]| t.iter().all(inside));
    counts.stray = before - triangles.len() - water.len();
    counts.water_triangles = water.len();

    let started = Instant::now();
    for name in &textures.names {
        reads.insert((name.clone(), ResKind::Mtr));
        if let Some(r) = crate::texture::resolve(game, name) {
            reads.insert((r.image_name(), ResKind::Texture));
        }
    }
    let images = decode(game, &textures.names, o.threads);
    let textures: Vec<SceneTexture> = textures
        .names
        .into_iter()
        .zip(images)
        .map(|(name, image)| {
            let image = image.map_err(|e| notes.push(format!("texture {name}: {e:#}"))).ok();
            SceneTexture { name, image }
        })
        .collect();
    counts.textures = textures.len();
    counts.textures_missing = textures.iter().filter(|t| t.image.is_none()).count();
    timings.textures = started.elapsed();

    // A chasm's floor by its shape and place is one when it is also black where it is drawn and its texture is not black void.
    for t in triangles.iter_mut().filter(|t| t.chasm) {
        t.chasm = textures[t.texture as usize].image.as_deref().is_some_and(|m| chasm_black(m, &t.uv));
    }
    counts.chasm_triangles = triangles.iter().filter(|t| t.chasm).count();

    let started = Instant::now();
    let tiles: Vec<ProbeTile<'_>> = placed
        .into_iter()
        .map(|(z, walk, wok, faces)| ProbeTile {
            z,
            walk,
            wok,
            faces: faces.into_iter().map(|(p, uv, id)| (p, uv, textures[id as usize].image.as_deref())).collect(),
        })
        .collect();
    let mut probe = FloorProbe::measure(w, h, &tiles);
    probe.block(&blocks);
    timings.probe = started.elapsed();

    // (Colours are 0x00BBGGRR.)
    let colour = |label: &str| {
        let c = are.integer(label).unwrap_or(0) as u32;
        [(c & 0xff) as u8, ((c >> 8) & 0xff) as u8, ((c >> 16) & 0xff) as u8]
    };
    Ok(Scene {
        resref: area.resref.clone(),
        tileset: tileset_ref.to_string().to_ascii_lowercase(),
        tiles: [w as u32, h as u32],
        // (No flags is a natural surface area.)
        flags: are.integer("Flags").unwrap_or(4).clamp(0, i64::from(u32::MAX)) as u32,
        tileset_interior: read.tileset.general.interior,
        light: Light { ambient: colour("SunAmbientColor"), diffuse: colour("SunDiffuseColor") },
        objects: o.objects,
        textures,
        triangles,
        water,
        probe,
        counts,
        notes,
        reads: reads.into_iter().collect(),
        timings,
    })
}

impl Scene {
    /// The area's size in metres (x, y).
    pub fn size(&self) -> [f32; 2] {
        [self.tiles[0] as f32 * TILE, self.tiles[1] as f32 * TILE]
    }

    /// Interior, underground or surface, by the `.are`'s flags; an area on an interior tileset is an interior even when its builder
    /// left the flag off (on tei01, for one, the roofs would otherwise hide the rooms).
    pub fn kind(&self) -> Kind {
        match self.flags {
            f if f & 1 != 0 => Kind::Interior,
            f if f & 2 != 0 => Kind::Underground,
            _ if self.tileset_interior => Kind::Interior,
            _ => Kind::Surface,
        }
    }

    /// Where the roof is cut by `rule`: None for a surface area.
    pub fn roof_cut(&self, rule: &RoofRule) -> Option<RoofCut> {
        (self.kind() != Kind::Surface).then(|| self.probe.stats().roof_cut(rule))
    }

    /// Cuts away everything above `at` metres ([`cut`]). Water is left as it is (the renderer cuts it too).
    pub fn cut_roof(&mut self, at: f32) {
        self.triangles = cut(std::mem::take(&mut self.triangles), at);
    }

    /// A SHA-256 of everything in the scene (not its timings), hex. Two scenes built from the same inputs have the same digest.
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        let text = |h: &mut Sha256, s: &str| {
            h.update((s.len() as u64).to_le_bytes());
            h.update(s.as_bytes());
        };
        let floats = |h: &mut Sha256, v: &[f32]| v.iter().for_each(|x| h.update(x.to_bits().to_le_bytes()));
        text(&mut h, "nwn-bake/scene 1");
        text(&mut h, &self.resref);
        text(&mut h, &self.tileset);
        h.update(self.tiles[0].to_le_bytes());
        h.update(self.tiles[1].to_le_bytes());
        h.update(self.flags.to_le_bytes());
        h.update([u8::from(self.tileset_interior)]);
        h.update(self.light.ambient);
        h.update(self.light.diffuse);
        h.update([u8::from(self.objects)]);
        h.update((self.textures.len() as u64).to_le_bytes());
        for t in &self.textures {
            text(&mut h, &t.name);
            match &t.image {
                None => h.update([0]),
                Some(m) => {
                    h.update([1]);
                    text(&mut h, &m.image);
                    let base = &m.levels[0];
                    h.update((base.width as u64).to_le_bytes());
                    h.update((base.height as u64).to_le_bytes());
                    h.update(&base.rgba);
                }
            }
        }
        h.update((self.triangles.len() as u64).to_le_bytes());
        for t in &self.triangles {
            floats(&mut h, t.p.as_flattened());
            floats(&mut h, t.uv.as_flattened());
            h.update(t.texture.to_le_bytes());
            let (kind, i) = match t.part {
                Part::Tile(i) => (0u8, i),
                Part::Placeable(i) => (1, i),
                Part::Door(i) => (2, i),
            };
            h.update([kind]);
            h.update(i.to_le_bytes());
        }
        // (Only a scene with chasm floors adds them, so a scene without keeps the digest it had before they were marked.)
        if self.triangles.iter().any(|t| t.chasm) {
            text(&mut h, "chasm");
            for (i, _) in self.triangles.iter().enumerate().filter(|(_, t)| t.chasm) {
                h.update((i as u64).to_le_bytes());
            }
        }
        h.update((self.water.len() as u64).to_le_bytes());
        for t in &self.water {
            floats(&mut h, t.as_flattened());
        }
        h.update((self.probe.nx as u64).to_le_bytes());
        h.update((self.probe.ny as u64).to_le_bytes());
        floats(&mut h, &self.probe.heights);
        h.update(self.probe.colours.as_flattened());
        text(&mut h, "floor");
        for f in &self.probe.floor {
            h.update(f.map_or(u32::MAX, f32::to_bits).to_le_bytes());
        }
        h.update(self.probe.blocked.iter().map(|&b| u8::from(b)).collect::<Vec<u8>>());
        h.update((self.notes.len() as u64).to_le_bytes());
        for n in &self.notes {
            text(&mut h, n);
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// `triangles` with everything above `at` metres cut away: a triangle partly above it is cut along the plane, and the part below
/// stays (one or two triangles, in the triangle's place, so the order stays the scene's).
pub fn cut(triangles: Vec<Triangle>, at: f32) -> Vec<Triangle> {
    let mut kept = Vec::with_capacity(triangles.len());
    for t in triangles {
        let inside: Vec<usize> = (0..3).filter(|&i| t.p[i][2] <= at).collect();
        let lerp = |a: usize, b: usize| -> ([f32; 3], [f32; 2]) {
            let f = (at - t.p[a][2]) / (t.p[b][2] - t.p[a][2]);
            let (p, q) = (t.p[a], t.p[b]);
            (
                [p[0] + (q[0] - p[0]) * f, p[1] + (q[1] - p[1]) * f, at],
                [t.uv[a][0] + (t.uv[b][0] - t.uv[a][0]) * f, t.uv[a][1] + (t.uv[b][1] - t.uv[a][1]) * f],
            )
        };
        let at_corner = |i: usize| (t.p[i], t.uv[i]);
        let make = |v: [([f32; 3], [f32; 2]); 3]| Triangle {
            p: v.map(|x| x.0),
            uv: v.map(|x| x.1),
            texture: t.texture,
            part: t.part,
            chasm: t.chasm,
        };
        match inside.len() {
            3 => kept.push(t),
            0 => {}
            1 => {
                let a = inside[0];
                let (b, c) = ((a + 1) % 3, (a + 2) % 3);
                kept.push(make([at_corner(a), lerp(a, b), lerp(a, c)]));
            }
            _ => {
                let out = (0..3).find(|i| !inside.contains(i)).unwrap_or(0);
                let (a, b) = ((out + 1) % 3, (out + 2) % 3);
                let (pa, pb) = (lerp(a, out), lerp(b, out));
                kept.push(make([at_corner(a), at_corner(b), pb]));
                kept.push(make([at_corner(a), pb, pa]));
            }
        }
    }
    kept
}

/// The scene's texture names, numbered in the order they are first used.
#[derive(Default)]
struct Textures {
    names: Vec<String>,
    ids: BTreeMap<String, u32>,
}

impl Textures {
    fn id(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);
        id
    }
}

/// Each texture decoded ([`Game::mipped`]), on a few threads (decoding is most of a scene's time). Each result goes to its name's
/// place, so the order is the names' whatever thread finished first.
fn decode(game: &Game, names: &[String], threads: usize) -> Vec<Result<Arc<Mipped>>> {
    let threads = if threads == 0 { std::thread::available_parallelism().map_or(1, std::num::NonZero::get).clamp(1, 8) } else { threads };
    let threads = threads.min(names.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Result<Arc<Mipped>>>>> = names.iter().map(|_| Default::default()).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(name) = names.get(i) else { break };
                    *slots[i].locked() = Some(game.mipped(name));
                }
            });
        }
    });
    slots.into_iter().map(|s| s.take().unwrap_or_else(|| Err(anyhow::anyhow!("not decoded")))).collect()
}

/// Whether a texture is water: named so, and no waterfall or water cliff.
pub fn is_water(texture: &str) -> bool {
    texture.contains("water") && !texture.contains("watcliff") && !texture.contains("fall")
}

/// A tile model flattened, its walkmesh's faces (those `walkable`, for the probe) and which of its triangles may be a chasm's floor
/// (`stands` says which surfaces the game walks), each failure a note.
fn tile_model(game: &Game, name: &str, walkable: &dyn Fn(u32) -> bool, stands: &dyn Fn(u32) -> bool, notes: &mut Vec<String>) -> TileModel {
    let flat = game.flat(name).map_err(|e| notes.push(format!("tile model {name}: {e:#}"))).ok();
    let faces = tile_walk(game, name).map_err(|e| notes.push(format!("tile walkmesh {name}: {e:#}"))).unwrap_or_default();
    let walk = faces.iter().filter(|(_, m)| walkable(*m)).map(|(f, _)| *f).collect();
    let wok = faces.iter().map(|(f, m)| (*f, stands(*m))).collect();
    let chasm = match &flat {
        Some(flat) => {
            let stand: Vec<[Vec3; 3]> = faces.iter().filter(|(_, m)| stands(*m)).map(|(f, _)| *f).collect();
            let mut water = Vec::new();
            for_each_triangle(flat, |texture, p, _| {
                if is_water(texture) {
                    water.push(p.map(Vec3::from_array));
                }
            });
            let mut chasm = Vec::new();
            for_each_triangle(flat, |texture, p, _| chasm.push(!is_water(texture) && chasm_shape(p.map(Vec3::from_array), &stand, &water)));
            chasm
        }
        None => Vec::new(),
    };
    TileModel { flat, walk, wok, chasm }
}

/// The tile's walkmesh's faces, with their surfaces (`surfacemat.2da` rows).
fn tile_walk(game: &Game, name: &str) -> Result<Vec<([Vec3; 3], u32)>> {
    let data = game.resman.get_named(name, ResType::WOK).with_context(|| format!("no {name}.wok"))?;
    let wok = Walkmesh::read(&data, WalkmeshKind::Tile)?;
    let pose = crate::rest_pose(&wok.model);
    let mut walk = Vec::new();
    for (i, node) in wok.model.nodes.iter().enumerate() {
        let NodeKind::Mesh(mesh) = &node.kind else { continue };
        for f in &mesh.faces {
            walk.push((f.vertices.map(|v| pose[i].transform_point3(Vec3::from_array(mesh.vertices[v as usize]))), f.material));
        }
    }
    Ok(walk)
}

/// Whether a tileset's terrain is a chasm, by its name: `Chasym` (the underdark tilesets ttu01, tlv23, tcee0, tec01) or one with
/// `Chasm` in it (bwr01, tdm01, ttd01, trm02's `Chasm...`). Only a tile with such a corner can have a chasm's floor. (Not `Pit`:
/// tbm01's pits have black walls all the way down, so a coloured floor there shows as slivers at the bottom of a black hole.)
pub fn is_chasm_terrain(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("chasm") || name.contains("chasym")
}

/// Whether a tile's face `p` (in the tile model's space, its floor at 0) is a chasm's floor by its shape and place: level, at least
/// [`CHASM_DEPTH`] below the tile's floor, no walkable face of the tile's walkmesh (`stand`) at its height over or under it, and no
/// water face of the tile (`water`) over it. With its colour ([`is_chasm_floor`]) and its tile's terrain ([`is_chasm_terrain`]) this
/// is the whole rule.
pub fn chasm_shape(p: [Vec3; 3], stand: &[[Vec3; 3]], water: &[[Vec3; 3]]) -> bool {
    let n = (p[1] - p[0]).cross(p[2] - p[0]);
    let len = n.length();
    if len < 1e-6 || n.z.abs() < CHASM_LEVEL * len || p.iter().any(|v| v.z > -CHASM_DEPTH) {
        return false;
    }
    let c = (p[0] + p[1] + p[2]) / 3.0;
    !stand.iter().any(|f| height_at(f, c).is_some_and(|z| (z - c.z).abs() < CHASM_STAND))
        && !water.iter().any(|f| height_at(f, c).is_some_and(|z| z > c.z))
}

/// The whole rule for a chasm's floor: [`chasm_shape`] and [`chasm_black`].
pub fn is_chasm_floor(p: [Vec3; 3], uv: &[[f32; 2]; 3], image: Option<&Mipped>, stand: &[[Vec3; 3]], water: &[[Vec3; 3]]) -> bool {
    chasm_shape(p, stand, water) && image.is_some_and(|m| chasm_black(m, uv))
}

/// Whether a face with UVs `uv` on `image` is black where it is drawn ([`brightest_under`] at most [`CHASM_DARK`]) while the image is
/// not black all over (black void, which the pictures leave out: a chasm is not void).
pub fn chasm_black(image: &Mipped, uv: &[[f32; 2]; 3]) -> bool {
    !image.is_black() && brightest_under(image, uv) <= CHASM_DARK
}

/// The brightest a texture is under a triangle (a texel's channels added up, 0..765): sampled at 45 points spread over it, on its
/// full-size level.
pub fn brightest_under(image: &Mipped, uv: &[[f32; 2]; 3]) -> f32 {
    const STEPS: usize = 8;
    let mut most = 0.0f32;
    for i in 0..=STEPS {
        for j in 0..=STEPS - i {
            let (a, b) = (i as f32 / STEPS as f32, j as f32 / STEPS as f32);
            let c = 1.0 - a - b;
            let s = image.sample(a * uv[0][0] + b * uv[1][0] + c * uv[2][0], a * uv[0][1] + b * uv[1][1] + c * uv[2][1], 0.0, false);
            most = most.max(s[0] + s[1] + s[2]);
        }
    }
    most
}

/// The height of face `f` over point `c` (its x and y), if `c` is on it seen from above.
fn height_at(f: &[Vec3; 3], c: Vec3) -> Option<f32> {
    let (a, b, d) = (f[0], f[1], f[2]);
    let den = (b.x - a.x) * (d.y - a.y) - (d.x - a.x) * (b.y - a.y);
    if den.abs() < 1e-9 {
        return None;
    }
    let l1 = ((b.x - c.x) * (d.y - c.y) - (d.x - c.x) * (b.y - c.y)) / den;
    let l2 = ((d.x - c.x) * (a.y - c.y) - (a.x - c.x) * (d.y - c.y)) / den;
    let l3 = 1.0 - l1 - l2;
    (l1 >= -1e-4 && l2 >= -1e-4 && l3 >= -1e-4).then_some(l1 * a.z + l2 * b.z + l3 * d.z)
}

/// Calls `f(texture, positions, uvs)` for each triangle of a flattened model, in its order.
fn for_each_triangle(flat: &crate::Flat, mut f: impl FnMut(&str, [[f32; 3]; 3], [[f32; 2]; 3])) {
    for (texture, first, count) in &flat.groups {
        for k in *first..first + count {
            if k * 3 + 2 >= flat.positions.len() {
                break;
            }
            f(texture, [0, 1, 2].map(|v| flat.positions[k * 3 + v]), [0, 1, 2].map(|v| flat.uvs[k * 3 + v]));
        }
    }
}

/// `p` turned `n` quarter turns counter-clockwise about the vertical, exactly (coordinates swapped and negated).
fn quarter_turns(p: Vec3, n: u8) -> Vec3 {
    match n % 4 {
        0 => p,
        1 => Vec3::new(-p.y, p.x, p.z),
        2 => Vec3::new(-p.x, -p.y, p.z),
        _ => Vec3::new(p.y, -p.x, p.z),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A texture like `ttu01_pit_d`: rock at the top fading to black, its bottom quarter black (rows from the bottom).
    fn pit() -> Mipped {
        let rows: Vec<u8> = (0..16u8).flat_map(|y| if y < 4 { [0, 0, 0, 255] } else { [y * 12, y * 11, y * 10, 255] }).collect();
        Mipped::new("pit".into(), &mg_image::Rgba { width: 1, height: 16, data: rows })
    }

    fn void() -> Mipped {
        Mipped::new("black".into(), &mg_image::Rgba { width: 1, height: 1, data: vec![2, 2, 2, 255] })
    }

    /// A 10 m square's south-east half at height `z`, on the tile's middle.
    fn floor(z: f32) -> [Vec3; 3] {
        [Vec3::new(-5.0, -5.0, z), Vec3::new(5.0, -5.0, z), Vec3::new(5.0, 5.0, z)]
    }

    /// UVs on the pit texture's black bottom (as the ttu01 chasm floor's, at v 0.045), or on its rock.
    const BLACK: [[f32; 2]; 3] = [[0.1, 0.1], [0.9, 0.1], [0.9, 0.12]];
    const ROCK: [[f32; 2]; 3] = [[0.1, 0.7], [0.9, 0.7], [0.9, 0.9]];

    #[test]
    fn a_chasm_floor_is_a_black_level_face_well_below_the_tile_where_nobody_walks_and_no_water_lies() {
        let (pit, void) = (pit(), void());
        let yes = |p: [Vec3; 3], uv: &[[f32; 2]; 3], image: &Mipped, stand: &[[Vec3; 3]], water: &[[Vec3; 3]]| {
            is_chasm_floor(p, uv, Some(image), stand, water)
        };
        // The ttu01 chasm: a level face 2.4 m down, black where drawn, deep water's walkmesh (not walkable) under it.
        assert!(yes(floor(-2.4), &BLACK, &pit, &[], &[]));
        // Not deep enough: a dark patch of an ordinary floor.
        assert!(!yes(floor(-0.5), &BLACK, &pit, &[], &[]));
        // Not level: a wall fading to black at its foot.
        assert!(!yes([Vec3::new(-5.0, 5.0, -2.4), Vec3::new(5.0, 5.0, -2.4), Vec3::new(5.0, 5.0, -1.0)], &BLACK, &pit, &[], &[]));
        assert!(!yes([Vec3::new(-5.0, 5.0, -2.4), Vec3::new(5.0, 5.0, -2.4), Vec3::new(5.0, 3.0, -1.2)], &BLACK, &pit, &[], &[]));
        // Drawn with the texture's rock, not its black.
        assert!(!yes(floor(-2.4), &ROCK, &pit, &[], &[]));
        // Black void is left out of the pictures already, and is not a chasm.
        assert!(!yes(floor(-2.4), &BLACK, &void, &[], &[]));
        // No texture: nothing to say it is black.
        assert!(!is_chasm_floor(floor(-2.4), &BLACK, None, &[], &[]));
        // A walkable face at its height (a sunken floor people walk on) makes it ground; a bridge 4 m over it does not.
        assert!(!yes(floor(-2.4), &BLACK, &pit, &[floor(-2.0)], &[]));
        assert!(yes(floor(-2.4), &BLACK, &pit, &[floor(1.9)], &[]));
        // Water over it: a river's bed, drawn under the water as it is.
        assert!(!yes(floor(-2.4), &BLACK, &pit, &[], &[floor(-0.3)]));
        assert!(yes(floor(-2.4), &BLACK, &pit, &[], &[floor(-3.0)]));
        // Its shape alone is the same rule without the colour.
        assert!(chasm_shape(floor(-2.4), &[], &[]) && !chasm_shape(floor(-0.5), &[], &[]));
        assert!(chasm_black(&pit, &BLACK) && !chasm_black(&pit, &ROCK) && !chasm_black(&void, &BLACK));
    }

    #[test]
    fn only_a_tile_with_chasm_terrain_has_chasm_floors() {
        for t in ["Chasym", "Chasm", "ChasmGrass2", "chasm"] {
            assert!(is_chasm_terrain(t), "{t}");
        }
        for t in ["Floor", "Rock", "Water", "wall", "Platform", "Pit", "Pit4", "Firepit", "Lava", "Ravine"] {
            assert!(!is_chasm_terrain(t), "{t}");
        }
    }
}

/// The scene as placed copies of models, for outputs that keep each model once (a child of this module: it reuses the scene's own
/// reading and transforms).
pub mod placements;
