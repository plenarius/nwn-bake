// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! An area in 3D for the web: one glTF binary (`<area>.glb`) per area, a small manifest beside it (`<area>.json`), and the textures
//! as WebP files shared by every area (`docs/area-3d.md` has the format).
//!
//! Built from the area's scene ([`crate::area::scene()`], the same scene the area pictures are drawn from) and its placements
//! ([`crate::area::scene::placements`]): each model the area uses is written once, the tiles drawn from it as instances
//! (`EXT_mesh_gpu_instancing`, which three.js's `GLTFLoader` reads into `InstancedMesh`es), each placeable and door as a named node
//! with `extras` (its `.git` index, tag, resref, position and bearing). Before writing, the placements are checked to give the
//! scene's triangles exactly ([`crate::area::scene::placements::Placements::check`]). Water is one more mesh, in the area's space.
//!
//! glTF is Y-up: a game position (x east, y north, z up) is written as (x, z, -y), metres; a turn about the game's vertical is the
//! same angle about glTF's Y. UVs are written with v down (1 - the model's v), the textures top row first.
//!
//! Beside the `.glb`, the floor file (`<area>.floor`): the walkable floor of the tiles' walkmeshes every 0.625 m, with its height,
//! and where the `.git`'s placeables block it ([`crate::area::FloorProbe::floor_file`]).
//!
//! Textures are named by what they are: the first 20 hex digits of the SHA-256 of the encoded file, so two areas that use the same
//! image share one file, and a texture that did not change keeps its name from one bake to the next.

pub mod batch;
pub mod gltf;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::area::scene::placements::{Placements, placements};
use crate::area::{Kind, Light, Part, Scene, SceneOptions};
use crate::game::Game;
use crate::module::AreaFiles;
use crate::sync::Recover;
use crate::texture::Mipped;
use gltf::{Gltf, Primitive};

/// The manifest's format and version.
pub const FORMAT: &str = "nwn-bake/area-3d";
/// The manifest's version: raised when its fields, the files beside it or the `.glb`'s layout change in a way a reader must know.
/// Version 2 added the floor file (`<area>.floor`, [`FloorInfo`]); the `.glb`'s layout is version 1's ([`GLB_LAYOUT`]).
pub const VERSION: u32 = 2;
/// The `.glb`'s layout version, in its root's `extras`: 1 still, so a `.glb` baked from the same inputs is the same file, byte for
/// byte, as before the floor file (a browser's copy stays good).
pub const GLB_LAYOUT: u32 = 1;
/// The version of what a bake writes: the `.glb`s, the texture files and the manifests. The batch's keys hold it in place of the
/// crate's version and the build's commit, so a new build that writes the same files keeps every key and bakes nothing again. Raise
/// it by one in any change that alters those files for the same inputs; `tests/glb_version.rs` fails when the fixture's files
/// change and it was not raised (`docs/testing.md`, "The `.glb`s' version"). 2: the floor file, and the manifest's `floor`.
pub const GLB_VERSION: u32 = 2;
/// A see-through texel (alpha below this, 0..1) is cut out (`alphaMode: MASK`).
pub const ALPHA_CUTOFF: f32 = 0.4;
/// Water: the area pictures' colour and opacity.
pub const WATER: [f32; 4] = [0.173, 0.416, 0.525, 0.62];

/// Turns an image (RGBA, rows from the top) into a file's bytes at a quality (0..100).
pub type Encode = fn(&mg_image::Rgba, f32) -> Vec<u8>;

/// Lossy WebP, alpha kept (libwebp drops the alpha channel of an image that has none).
#[cfg(feature = "webp")]
pub fn webp(img: &mg_image::Rgba, quality: f32) -> Vec<u8> {
    webp::Encoder::from_rgba(&img.data, img.width, img.height).encode(quality).to_vec()
}

/// What to write.
#[derive(Clone, Debug, Serialize)]
pub struct Options {
    /// Textures are at most this many texels wide and high (the first mip level that fits).
    pub max_tex: usize,
    /// WebP quality.
    pub quality: f32,
    /// The `.git`'s placeables and doors.
    pub objects: bool,
    /// What the `.glb` puts before a texture's file name: the texture folder relative to the `.glb`'s (`tex/`, `../tex/`), or a URL.
    pub tex_uri: String,
    /// Threads that encode textures.
    #[serde(skip)]
    pub threads: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_tex: 512, quality: 70.0, objects: true, tex_uri: "tex/".into(), threads: 4 }
    }
}

/// A texture file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct TexFile {
    /// `<20 hex digits>.webp`.
    pub file: String,
    /// The file's length in bytes.
    pub bytes: u64,
    /// Width in texels.
    pub width: usize,
    /// Height in texels.
    pub height: usize,
    /// It has see-through texels (drawn as a cut-out).
    pub alpha: bool,
}

/// A file's name from its bytes: the first 20 hex digits of their SHA-256, and `ext`.
pub fn content_name(bytes: &[u8], ext: &str) -> String {
    let h = Sha256::digest(bytes);
    format!("{}.{ext}", h.iter().take(10).map(|b| format!("{b:02x}")).collect::<String>())
}

/// The texture folder: each image encoded once per process (remembered by its texels), written when it is not there yet.
pub struct TextureFiles {
    dir: PathBuf,
    encode: Encode,
    ext: &'static str,
    known: Mutex<HashMap<[u8; 32], Result<TexFile, String>>>,
    written: std::sync::atomic::AtomicUsize,
}

impl TextureFiles {
    /// Textures in `dir`, made with `encode` (whose files end in `ext`: `webp`).
    pub fn new(dir: &Path, encode: Encode, ext: &'static str) -> TextureFiles {
        TextureFiles { dir: dir.to_path_buf(), encode, ext, known: Mutex::new(HashMap::new()), written: Default::default() }
    }

    /// The folder the texture files go to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// How many files were written (the others were there already).
    pub fn written(&self) -> usize {
        self.written.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `image`'s file at most `max` texels a side, at `quality`: made and written the first time, else the one already made.
    pub fn file(&self, image: &Mipped, max: usize, quality: f32) -> Result<TexFile> {
        let level = &image.levels[image.level_within(max)];
        let mut h = Sha256::new();
        h.update(b"nwn-bake/area-3d texture 1\0");
        h.update(quality.to_le_bytes());
        h.update((level.width as u64).to_le_bytes());
        h.update((level.height as u64).to_le_bytes());
        h.update(&level.rgba);
        let key: [u8; 32] = h.finalize().into();
        if let Some(r) = self.known.locked().get(&key) {
            return r.clone().map_err(anyhow::Error::msg);
        }
        let made = self.make(level, quality).map_err(|e| format!("{e:#}"));
        self.known.locked().entry(key).or_insert(made).clone().map_err(anyhow::Error::msg)
    }

    fn make(&self, level: &crate::texture::Level, quality: f32) -> Result<TexFile> {
        // (Rows from the top, as image files have them.)
        let row = level.width * 4;
        let data: Vec<u8> = level.rgba.chunks_exact(row).rev().flatten().copied().collect();
        let alpha = data.as_chunks::<4>().0.iter().any(|p| p[3] < 250);
        let bytes = (self.encode)(&mg_image::Rgba { width: level.width as u32, height: level.height as u32, data }, quality);
        let file = content_name(&bytes, self.ext);
        if write_new(&self.dir.join(&file), &bytes)? {
            self.written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(TexFile { file, bytes: bytes.len() as u64, width: level.width, height: level.height, alpha })
    }
}

/// Writes `bytes` to `path` unless a file of that length is there already (texture files are named by their content), through a
/// temporary file and a rename, so a reader never sees half a file. Whether it wrote.
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<bool> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() == bytes.len() as u64) {
        return Ok(false);
    }
    write_atomic(path, bytes).map(|()| true)
}

/// Writes `bytes` to `path` through a temporary file beside it and a rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    // (A name of its own for each write: two threads may write the same shared texture at once.)
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{}.{}.{n}.tmp", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))
}

/// A door of the `.git`, as the manifest lists it (every door, drawn or not).
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct Door {
    /// Its index in the `.git`'s `Door List`.
    pub index: u32,
    /// Its `Tag`.
    pub tag: String,
    /// Its blueprint (`TemplateResRef`).
    pub resref: String,
    /// Its name in the game, when the talk table has it.
    pub name: Option<String>,
    /// Its model; None when it has none the game can read (it is not drawn).
    pub model: Option<String>,
    /// Its node in the `.glb`; None when it is not drawn.
    pub node: Option<String>,
    /// The `.git`'s X: game metres east, unrounded.
    pub x: f32,
    /// The `.git`'s Y: game metres north, unrounded.
    pub y: f32,
    /// The `.git`'s Z: game metres up, unrounded.
    pub z: f32,
    /// The `.git`'s `Bearing`: radians counter-clockwise from east, unrounded.
    pub bearing: f32,
    /// The bearing in degrees, 0..360.
    pub bearing_deg: f32,
    /// The direction its street side faces, degrees counter-clockwise from east (0 east, 90 north): the door panel lies along its
    /// bearing and the street side faces the bearing less 90 degrees (as measured on real doors).
    pub front_deg: f32,
    /// The same as a unit vector in the `.glb`'s axes (Y up).
    pub front: [f32; 3],
    /// Its `Locked` field.
    pub locked: bool,
    /// The door or waypoint its transition leads to (`LinkedTo`), when it has one.
    pub linked_to: Option<String>,
}

/// What `area-3d` holds, counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Counts {
    /// Tiles of the area, with a model or not: width times height.
    pub tiles: usize,
    /// Placeables drawn (those whose model was read).
    pub placeables: usize,
    /// Doors drawn (those whose model was read).
    pub doors: usize,
    /// Distinct models (a model placed by tiles and by objects counts twice: a tile's water is apart).
    pub models: usize,
    /// glTF meshes: a model, or a model less the stray geometry one placement leaves out.
    pub meshes: usize,
    /// Nodes drawing tiles (one a mesh, instanced).
    pub tile_nodes: usize,
    /// Triangles stored: each mesh once, water included.
    pub triangles_stored: usize,
    /// Triangles drawn: every instance's, water included.
    pub triangles_drawn: usize,
    /// Vertices stored, each mesh once, water included.
    pub vertices: usize,
    /// Water triangles (stored once, in the area's space).
    pub water_triangles: usize,
    /// Texture names the meshes use (two names may mean one image, written as one file).
    pub textures: usize,
    /// Of those, the names with no texture file: the game has no image for them, or the image could not be encoded.
    pub textures_missing: usize,
}

/// The manifest (`<area>.json`); see `docs/area-3d.md`.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct Manifest {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`VERSION`] of the renderer that wrote it.
    pub version: u32,
    /// The program that wrote it (as the area pictures' `renderer`, with `glb` in place of their `pictures`).
    pub renderer: Renderer,
    /// The batch's key (`batch::quick_key`), when baked by a batch.
    pub key: Option<String>,
    /// The area: its names, size, flags and light.
    pub area: AreaInfo,
    /// `y-up`: a game (x, y, z) is the `.glb`'s (x, z, -y), metres.
    pub axes: String,
    /// The `.glb`'s file name, length and SHA-256.
    pub glb: FileInfo,
    /// The floor file (version 2 on; None in a manifest of version 1).
    #[serde(default)]
    pub floor: Option<FloorInfo>,
    /// What the `.glb` puts before each texture's file name.
    pub tex_uri: String,
    /// The texture files the `.glb` uses, each once, in its order.
    pub textures: Vec<TexFile>,
    /// What the `.glb` holds, counted.
    pub counts: Counts,
    /// What is drawn (water included), game metres: lowest and highest x, y, z.
    pub bounds: [[f32; 3]; 2],
    /// Every door of the `.git`, drawn or not, in its order.
    pub doors: Vec<Door>,
    /// The scene's notes (missing models and textures, a tileset's absent sections).
    pub notes: Vec<String>,
    /// What the area was baked from.
    pub inputs: Inputs,
    /// The options that change the output.
    pub options: OptionsInfo,
}

/// The program that wrote a manifest.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct Renderer {
    /// Always `nwn-bake`.
    pub name: String,
    /// Its version, from `Cargo.toml`.
    pub version: String,
    /// The git commit it was built from (`NWN_BAKE_COMMIT` at build time, as release builds set it); None for a local build.
    pub commit: Option<String>,
    /// [`GLB_VERSION`]; 0 in a manifest written before it was recorded.
    #[serde(default)]
    pub glb: u32,
}

impl Renderer {
    /// This program.
    pub fn this() -> Renderer {
        Renderer {
            name: "nwn-bake".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            commit: option_env!("NWN_BAKE_COMMIT").filter(|c| !c.is_empty()).map(str::to_string),
            glb: GLB_VERSION,
        }
    }
}

/// The area, as the manifest describes it.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct AreaInfo {
    /// Its resref, lower case.
    pub resref: String,
    /// Its name in the game (the `.are`'s `Name`, through the talk table), when it has one.
    pub name: Option<String>,
    /// Its tileset's resref, lower case.
    pub tileset: String,
    /// Tiles along x (east) and y (north).
    pub tiles: [u32; 2],
    /// Its size in metres along x and y: the tiles times 10 m.
    pub size_m: [f32; 2],
    /// The `.are`'s `Flags`: 1 interior, 2 underground, 4 natural.
    pub flags: u32,
    /// `surface`, `interior` or `underground` ([`crate::area::Scene::kind`]): whether a viewer cuts the roof.
    pub kind: String,
    /// The area's sun.
    pub light: SunLight,
}

/// The area's sun (`SunAmbientColor` and `SunDiffuseColor`), RGB, as the manifest writes it.
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize)]
pub struct SunLight {
    /// `SunAmbientColor`: the light everything gets, whichever way it faces.
    pub ambient: [u8; 3],
    /// `SunDiffuseColor`: the sun's light on faces turned towards it.
    pub diffuse: [u8; 3],
}

impl From<Light> for SunLight {
    fn from(l: Light) -> Self {
        SunLight { ambient: l.ambient, diffuse: l.diffuse }
    }
}

/// A file the manifest names.
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct FileInfo {
    /// Its name, in the manifest's folder.
    pub file: String,
    /// Its length in bytes.
    pub bytes: u64,
    /// The SHA-256 of its bytes: `sha256:` and hex, as every digest in the manifest.
    pub sha256: String,
}

/// The floor file the manifest names (`docs/area-3d.md`, "The floor file").
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct FloorInfo {
    /// `<area>.floor`, in the manifest's folder.
    pub file: String,
    /// Its length in bytes.
    pub bytes: u64,
    /// The SHA-256 of its bytes.
    pub sha256: String,
    /// The file's own version ([`crate::area::floor::FLOOR_VERSION`]), as in its header.
    pub version: u16,
    /// Metres between samples: 0.625.
    pub spacing_m: f32,
    /// Samples a tile's side: 16.
    pub per_tile: usize,
    /// Samples along x and y: the tiles times 16, and one more.
    pub samples: [usize; 2],
    /// A height in the file is in this many metres: 0.01.
    pub unit_m: f32,
    /// How many samples are walkable floor.
    pub walkable: usize,
    /// How many of those a `.git` placeable's walkmesh blocks.
    pub blocked: usize,
}

/// What the area was baked from.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct Inputs {
    /// `sha256:` of the `.are` as binary GFF, hex (a `.mod` and its nasher JSON give the same).
    pub are: String,
    /// `sha256:` of the `.git` as binary GFF; None when the area has none.
    pub git: Option<String>,
    /// The haks the game was opened with, first highest.
    pub haks: Vec<String>,
    /// The haks asked for that were found nowhere.
    pub missing_haks: Vec<String>,
    /// The custom talk table asked for (`--tlk`, else the module's `Mod_CustomTlk`), as given; None for none.
    pub tlk: Option<String>,
}

impl Inputs {
    /// The inputs of `area`: the digests of its `.are` and `.git`, with the haks, the haks not found and the talk table as given.
    /// Fails when a GFF can't be written back as binary.
    pub fn of(area: &AreaFiles, haks: &[String], missing: &[String], tlk: Option<&str>) -> Result<Inputs> {
        let digest = |g: &mg_gff::Gff| -> Result<String> {
            let bytes = g.to_bytes().map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(format!("sha256:{:x}", Sha256::digest(&bytes)))
        };
        Ok(Inputs {
            are: digest(&area.are).with_context(|| format!("{}.are", area.resref))?,
            git: area.git.as_ref().map(digest).transpose().with_context(|| format!("{}.git", area.resref))?,
            haks: haks.to_vec(),
            missing_haks: missing.to_vec(),
            tlk: tlk.map(str::to_string),
        })
    }
}

/// The options as the manifest records them.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct OptionsInfo {
    /// [`Options::max_tex`].
    pub max_tex: usize,
    /// [`Options::quality`]: the WebP quality, named as the area pictures' manifest names it.
    pub webp_quality: f32,
    /// [`Options::objects`].
    pub objects: bool,
}

/// An area baked: the `.glb`'s and the floor file's bytes, its manifest (not written yet) and how long each part took.
pub struct Baked {
    /// The `.glb` file's bytes.
    pub glb: Vec<u8>,
    /// The floor file's bytes.
    pub floor: Vec<u8>,
    /// Its manifest, to be written as `<area>.json`.
    pub manifest: Manifest,
    /// How long each part took. Not in the manifest, which has no times, so the same inputs give the same bytes.
    pub timings: Timings,
}

/// How long each part of a bake took, in seconds.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Timings {
    /// The scene (reading the area, flattening the models, decoding the textures, the floor probe).
    pub scene: f64,
    /// The placements and their check against the scene.
    pub placements: f64,
    /// Encoding and writing the textures not made before.
    pub textures: f64,
    /// Building the glb.
    pub glb: f64,
}

/// A game position as glTF's (Y up).
pub fn y_up(p: [f32; 3]) -> [f32; 3] {
    [p[0], p[2], -p[1]]
}

/// A turn of `angle` radians counter-clockwise about the game's vertical, as a glTF rotation (x, y, z, w): the same angle about Y.
/// Quarter turns are exact.
pub fn turn_quaternion(angle: f32, quarters: Option<u8>) -> [f32; 4] {
    let h = std::f32::consts::FRAC_1_SQRT_2;
    match quarters.map(|q| q % 4) {
        Some(0) => [0.0, 0.0, 0.0, 1.0],
        Some(1) => [0.0, h, 0.0, h],
        Some(2) => [0.0, 1.0, 0.0, 0.0],
        Some(_) => [0.0, h, 0.0, -h],
        None => [0.0, (angle / 2.0).sin(), 0.0, (angle / 2.0).cos()],
    }
}

/// A node's name: `kind_index`, then the tag with only letters, digits, `_` and `-` kept (three.js drops some characters from
/// names; these survive).
fn node_name(kind: &str, index: u32, tag: &str) -> String {
    let tag: String = tag.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    if tag.is_empty() { format!("{kind}_{index}") } else { format!("{kind}_{index}_{tag}") }
}

/// A `.git` object's text field.
fn text(s: &mg_gff::Struct, label: &str) -> String {
    s.string(label).map_or_default(|b| String::from_utf8_lossy(b).trim().to_string())
}

/// Bakes `area`: builds its scene and placements from `game`, writes the textures it needs into `tex` and gives the `.glb` and its
/// manifest (`key` goes into the manifest; `inputs` is what the area was baked from). Fails when the placements do not give the
/// scene's triangles.
pub fn bake(game: &Game, area: &AreaFiles, o: &Options, tex: &TextureFiles, inputs: Inputs, key: Option<String>) -> Result<Baked> {
    let mut timings = Timings::default();
    let started = Instant::now();
    // (The update fills the options a later scene has: the texture threads of the pictures' batch.)
    #[allow(clippy::needless_update, reason = "needless only while SceneOptions has no field but `objects`, which depends on the features")]
    let so = SceneOptions { objects: o.objects, ..Default::default() };
    let scene = crate::area::scene(game, area, &so)?;
    timings.scene = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let placed = placements(game, area, &so, &scene)?;
    placed.check(&scene).with_context(|| format!("{}: the placements are not the scene", area.resref))?;
    timings.placements = started.elapsed().as_secs_f64();
    let title = area.are.root.locstring("Name").and_then(|n| game.locstring(n)).map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    let doors = doors(game, area, &placed);
    let objects = objects(area);
    write(&scene, &placed, &objects, doors, title, o, tex, inputs, key, timings)
}

/// The `.git`'s placeables' and doors' tags and blueprints (`TemplateResRef`), by (is a door, index in its list).
pub type Objects = BTreeMap<(bool, u32), (String, String)>;

/// The `.git`'s placeables and doors: their tags and blueprints.
pub fn objects(area: &AreaFiles) -> Objects {
    let mut out = Objects::new();
    let Some(git) = &area.git else { return out };
    for (list, door) in [("Placeable List", false), ("Door List", true)] {
        for (i, s) in git.root.list(list).unwrap_or(&[]).iter().enumerate() {
            out.insert(
                (door, i as u32),
                (text(s, "Tag"), s.resref("TemplateResRef").map_or_default(|r| r.to_string().to_ascii_lowercase())),
            );
        }
    }
    out
}

/// Every door of the `.git`, with the node that draws it (if any).
fn doors(game: &Game, area: &AreaFiles, placed: &Placements) -> Vec<Door> {
    let Some(git) = &area.git else {
        return Vec::new();
    };
    let drawn: BTreeMap<u32, &str> = placed
        .placements
        .iter()
        .filter_map(|p| if let Part::Door(i) = p.part { Some((i, placed.models[p.model].name.as_str())) } else { None })
        .collect();
    git.root
        .list("Door List")
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let f = |l: &str| s.float(l).unwrap_or(0.0);
            let (x, y, z, bearing) = (f("X"), f("Y"), f("Z"), f("Bearing"));
            let bearing_deg = bearing.to_degrees().rem_euclid(360.0);
            let front_deg = (bearing_deg - 90.0).rem_euclid(360.0);
            let fr = front_deg.to_radians();
            let tag = text(s, "Tag");
            let model = drawn.get(&(i as u32)).map(ToString::to_string);
            let node = model.as_ref().map(|_| node_name("door", i as u32, &tag));
            Door {
                index: i as u32,
                resref: s.resref("TemplateResRef").map_or_default(|r| r.to_string().to_ascii_lowercase()),
                name: s.locstring("LocName").and_then(|n| game.locstring(n)).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
                model,
                node,
                tag,
                x,
                y,
                z,
                bearing,
                bearing_deg,
                front_deg,
                front: y_up([fr.cos(), fr.sin(), 0.0]),
                locked: s.integer("Locked").unwrap_or(0) != 0,
                linked_to: Some(text(s, "LinkedTo")).filter(|t| !t.is_empty()),
            }
        })
        .collect()
}

/// The `.glb` and the manifest of a scene and its placements (see [`bake`]).
pub fn write(
    scene: &Scene,
    placed: &Placements,
    objects: &Objects,
    doors: Vec<Door>,
    title: Option<String>,
    o: &Options,
    tex: &TextureFiles,
    inputs: Inputs,
    key: Option<String>,
    mut timings: Timings,
) -> Result<Baked> {
    // Each mesh: a model less what one placement drops, in the order first placed.
    let mut mesh_of: BTreeMap<(usize, Vec<u32>), usize> = BTreeMap::new();
    let mut meshes: Vec<(usize, Vec<u32>)> = Vec::new();
    let placement_mesh: Vec<usize> = placed
        .placements
        .iter()
        .map(|p| {
            *mesh_of.entry((p.model, p.dropped.clone())).or_insert_with(|| {
                meshes.push((p.model, p.dropped.clone()));
                meshes.len() - 1
            })
        })
        .collect();

    // The textures the meshes use, in the order first used; encoded on a few threads.
    let mut used: Vec<u32> = Vec::new();
    let mut seen = vec![false; scene.textures.len()];
    for (m, _) in &meshes {
        for t in &placed.models[*m].triangles {
            if !std::mem::replace(&mut seen[t.texture as usize], true) {
                used.push(t.texture);
            }
        }
    }
    let started = Instant::now();
    let files = encode_all(scene, &used, o, tex);
    timings.textures = started.elapsed().as_secs_f64();
    let started = Instant::now();

    let mut g = Gltf::new(&format!("nwn-bake {} area-3d", env!("CARGO_PKG_VERSION")));
    let mut material_of: HashMap<u32, usize> = HashMap::new();
    // (Two texture names may mean one image, so one file: one glTF texture a file.)
    let mut texture_of: HashMap<String, usize> = HashMap::new();
    let mut textures: Vec<TexFile> = Vec::new();
    let mut notes = scene.notes.clone();
    for (&id, file) in used.iter().zip(&files) {
        let name = &scene.textures[id as usize].name;
        let mut m = json!({ "name": name, "doubleSided": true, "pbrMetallicRoughness": { "metallicFactor": 0.0, "roughnessFactor": 1.0 } });
        match file {
            Some(Ok(f)) => {
                let t = *texture_of.entry(f.file.clone()).or_insert_with(|| {
                    textures.push(f.clone());
                    g.webp_texture(&format!("{}{}", o.tex_uri, f.file))
                });
                m["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": t });
                if f.alpha {
                    m["alphaMode"] = json!("MASK");
                    m["alphaCutoff"] = json!(ALPHA_CUTOFF);
                }
            }
            Some(Err(e)) => {
                notes.push(format!("texture {name}: {e:#}"));
                m["pbrMetallicRoughness"]["baseColorFactor"] = json!([0.5, 0.5, 0.5, 1.0]);
            }
            // (No image: the scene noted it.)
            None => m["pbrMetallicRoughness"]["baseColorFactor"] = json!([0.5, 0.5, 0.5, 1.0]),
        }
        material_of.insert(id, g.material(m));
    }

    let mut counts = Counts {
        tiles: scene.counts.tiles,
        placeables: scene.counts.placeables,
        doors: scene.counts.doors,
        models: placed.models.len(),
        meshes: meshes.len(),
        ..Default::default()
    };
    counts.textures = used.len();
    counts.textures_missing = files.iter().filter(|f| !matches!(f, Some(Ok(_)))).count();
    let mut mesh_index = Vec::with_capacity(meshes.len());
    let mut mesh_triangles = Vec::with_capacity(meshes.len());
    for (k, (m, dropped)) in meshes.iter().enumerate() {
        let model = &placed.models[*m];
        // One primitive a texture, in the order the model first uses them.
        let mut groups: Vec<(u32, Vec<gltf::Corners>)> = Vec::new();
        let mut dropped = dropped.iter().peekable();
        for (i, t) in model.triangles.iter().enumerate() {
            if dropped.peek().is_some_and(|&&d| d as usize == i) {
                dropped.next();
                continue;
            }
            let tri = (t.p.map(y_up), t.uv.map(|uv| [uv[0], 1.0 - uv[1]]));
            match groups.iter_mut().find(|(id, _)| *id == t.texture) {
                Some((_, list)) => list.push(tri),
                None => groups.push((t.texture, vec![tri])),
            }
        }
        let prims: Vec<Primitive> = groups.into_iter().map(|(id, tris)| Primitive::welded(tris, material_of.get(&id).copied())).collect();
        counts.vertices += prims.iter().map(|p| p.positions.len()).sum::<usize>();
        let tris: usize = prims.iter().map(Primitive::triangles).sum();
        counts.triangles_stored += tris;
        mesh_triangles.push(tris);
        let name = if meshes[..k].iter().any(|(other, _)| other == m) { format!("{}~{k}", model.name) } else { model.name.clone() };
        mesh_index.push(g.mesh(&name, &prims));
    }

    let mut children = Vec::new();
    // The tiles: one instanced node a mesh, in the order first placed.
    let mut tile_groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for (i, p) in placed.placements.iter().enumerate() {
        if !matches!(p.part, Part::Tile(_)) {
            continue;
        }
        match tile_groups.iter_mut().find(|(m, _)| *m == placement_mesh[i]) {
            Some((_, list)) => list.push(i),
            None => tile_groups.push((placement_mesh[i], vec![i])),
        }
    }
    for (mesh, list) in &tile_groups {
        let (model, _) = &meshes[*mesh];
        let translations: Vec<[f32; 3]> = list.iter().map(|&i| y_up(placed.placements[i].position)).collect();
        let rotations: Vec<[f32; 4]> = list
            .iter()
            .map(|&i| match placed.placements[i].turn {
                crate::area::scene::placements::Turn::Quarters(q) => turn_quaternion(0.0, Some(q)),
                crate::area::scene::placements::Turn::Bearing(b) => turn_quaternion(b, None),
            })
            .collect();
        let tiles: Vec<u32> =
            list.iter().filter_map(|&i| if let Part::Tile(t) = placed.placements[i].part { Some(t) } else { None }).collect();
        let name = format!("tiles_{}", placed.models[*model].name);
        let extras = json!({ "part": "tiles", "model": placed.models[*model].name, "tiles": tiles });
        children.push(g.instanced(&name, mesh_index[*mesh], &translations, &rotations, extras));
        counts.triangles_drawn += mesh_triangles[*mesh] * list.len();
    }
    counts.tile_nodes = tile_groups.len();

    // The placeables and doors: a node each, in the `.git`'s order (placeables first), sharing their models' meshes.
    for (i, p) in placed.placements.iter().enumerate() {
        let (kind, index) = match p.part {
            Part::Tile(_) => continue,
            Part::Placeable(k) => ("placeable", k),
            Part::Door(k) => ("door", k),
        };
        let (tag, resref) = objects.get(&(kind == "door", index)).map_or(("", ""), |(t, r)| (t.as_str(), r.as_str()));
        let name = node_name(kind, index, tag);
        let mut extras = json!({
            "part": kind, "index": index, "tag": tag, "resref": resref, "model": placed.models[p.model].name,
            "x": p.position[0], "y": p.position[1], "z": p.position[2], "bearing": p.angle(),
        });
        if let Some(d) = doors.iter().find(|d| d.index == index).filter(|_| kind == "door") {
            extras["front_deg"] = json!(d.front_deg);
            extras["locked"] = json!(d.locked);
        }
        let node = json!({
            "name": name, "mesh": mesh_index[placement_mesh[i]], "translation": y_up(p.position),
            "rotation": turn_quaternion(p.angle(), None), "extras": extras,
        });
        children.push(g.node(node));
        counts.triangles_drawn += mesh_triangles[placement_mesh[i]];
    }

    // The water: one mesh in the area's space.
    if !scene.water.is_empty() {
        let mat = g.material(json!({
            "name": "water", "doubleSided": true, "alphaMode": "BLEND",
            "pbrMetallicRoughness": { "baseColorFactor": WATER, "metallicFactor": 0.0, "roughnessFactor": 0.3 },
        }));
        let prim = Primitive::welded(scene.water.iter().map(|t| (t.map(y_up), [[0.0; 2]; 3])), Some(mat));
        counts.vertices += prim.positions.len();
        let mesh = g.mesh("water", &[prim]);
        children.push(g.node(json!({ "name": "water", "mesh": mesh, "extras": { "part": "water" } })));
        counts.water_triangles = scene.water.len();
        counts.triangles_stored += scene.water.len();
        counts.triangles_drawn += scene.water.len();
    }

    let root = g.node(json!({
        "name": scene.resref, "children": children,
        "extras": { "format": FORMAT, "version": GLB_LAYOUT, "area": scene.resref, "tileset": scene.tileset, "tiles": scene.tiles, "axes": "y-up" },
    }));
    g.root(root);
    let glb = g.to_glb();
    timings.glb = started.elapsed().as_secs_f64();

    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in scene.triangles.iter().flat_map(|t| t.p.iter()).chain(scene.water.iter().flatten()) {
        for c in 0..3 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    let bounds = if lo[0].is_finite() { [lo, hi] } else { [[0.0; 3]; 2] };
    let floor = scene.probe.floor_file();
    let (walkable, blocked) = scene.probe.floor_counts();
    let kind = match scene.kind() {
        Kind::Surface => "surface",
        Kind::Interior => "interior",
        Kind::Underground => "underground",
    };
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        renderer: Renderer::this(),
        key,
        area: AreaInfo {
            resref: scene.resref.clone(),
            name: title,
            tileset: scene.tileset.clone(),
            tiles: scene.tiles,
            size_m: scene.size(),
            flags: scene.flags,
            kind: kind.into(),
            light: scene.light.into(),
        },
        axes: "y-up".into(),
        glb: FileInfo {
            file: format!("{}.glb", scene.resref),
            bytes: glb.len() as u64,
            sha256: format!("sha256:{:x}", Sha256::digest(&glb)),
        },
        floor: Some(FloorInfo {
            file: format!("{}.floor", scene.resref),
            bytes: floor.len() as u64,
            sha256: format!("sha256:{:x}", Sha256::digest(&floor)),
            version: crate::area::floor::FLOOR_VERSION,
            spacing_m: crate::area::floor::SPACING,
            per_tile: crate::area::floor::PER_TILE,
            samples: [scene.probe.nx, scene.probe.ny],
            unit_m: crate::area::floor::FLOOR_UNIT,
            walkable,
            blocked,
        }),
        tex_uri: o.tex_uri.clone(),
        textures,
        counts,
        bounds,
        doors,
        notes,
        inputs,
        options: OptionsInfo { max_tex: o.max_tex, webp_quality: o.quality, objects: o.objects },
    };
    Ok(Baked { glb, floor, manifest, timings })
}

/// The used textures' files (None when the scene has no image for one), encoded on `o.threads` threads, in `used`'s order.
fn encode_all(scene: &Scene, used: &[u32], o: &Options, tex: &TextureFiles) -> Vec<Option<Result<TexFile>>> {
    let images: Vec<Option<Arc<Mipped>>> = used.iter().map(|&id| scene.textures[id as usize].image.clone()).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<Result<TexFile>>>> = images.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..o.threads.clamp(1, 16).min(images.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(image) = images.get(i) else { break };
                    if let Some(m) = image {
                        *slots[i].locked() = Some(tex.file(m, o.max_tex, o.quality));
                    }
                }
            });
        }
    });
    slots.into_iter().map(Recover::take).collect()
}
