// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pictures and plain data from Neverwinter Nights: Enhanced Edition, for websites and tools.
//!
//! - [`iso`]: area pictures, drawn on the CPU from the four corners and from the top, with a manifest that maps game coordinates
//!   to pixels ([`iso::batch`]: a whole module, with keys and a cache). The format is `docs/area-iso.md`.
//! - [`area3d`]: an area as one glTF binary for a web page, each model stored once, with shared WebP textures (`docs/area-3d.md`).
//! - [`area`]: an area's scene (the tiles' triangles, water, placeables and doors) that both are built from; [`area::tileset`]: a
//!   tileset as the game reads it.
//! - `look` and `creature` (the `creature` feature): characters as parts, and pictures of creatures, items and placeables.
//! - `scene` (the `creature` feature): several objects (placeables, models, creatures, items) in one picture, at a camera of the
//!   caller's (`docs/scene.md`).
//! - [`items`]: where a creature blueprint's equipped items are found (a folder of the module's `.uti` files, then the game), and
//!   which were not.
//! - [`Flat`]: a model at rest as triangles grouped by texture (placeables, doors, item models), with its walkmesh footprint.
//! - [`game::open`]: the game, with a module's haks in the module's order.
//! - [`module::ModuleSource`]: a module's areas, haks and talk table, from a `.mod`, a folder, a nasher project or one area file.
//! - [`texture::resolve`]: what a mesh's texture name means (an EE material names another image); [`texture::png`]: a game texture
//!   as PNG.
//!
//! Built on Moonglow's crates (one pinned revision, see MOONGLOW.md).

pub mod area;
pub mod area3d;
#[cfg(feature = "creature")]
pub mod creature;
#[cfg(feature = "creature")]
pub mod emitters;
pub mod game;
pub mod iso;
pub mod items;
#[cfg(feature = "creature")]
pub mod look;
pub mod module;
#[cfg(feature = "creature")]
pub mod scene;
mod sync;
pub mod texture;

use anyhow::{Context, Result};
use glam::{Mat4, Quat, Vec3};
use mg_core::{ResType, StrRef};
use mg_mdl::walkmesh::{Walkmesh, WalkmeshKind};
use mg_mdl::{Model, NodeKind};
use mg_rules::GameData;

/// The module's custom talk table, from the user folder: `explicit` (a path, or a file name in `tlk/` or `hak/`, `.tlk` added when it
/// has no extension), else the one `.tlk` in `user/tlk` or `user/hak` that isn't the game's own `dialog`/`dialogf`, when there is
/// exactly one.
pub fn custom_tlk(user: &std::path::Path, explicit: Option<&str>) -> Option<mg_tlk::Tlk> {
    custom_tlk_path(user, explicit).and_then(|p| std::fs::read(p).ok()).and_then(|b| mg_tlk::Tlk::read(&b).ok())
}

/// The file [`custom_tlk`] reads, when there is one.
pub fn custom_tlk_path(user: &std::path::Path, explicit: Option<&str>) -> Option<std::path::PathBuf> {
    if let Some(name) = explicit.filter(|n| !n.trim().is_empty()) {
        let as_path = std::path::Path::new(name);
        if as_path.is_absolute() {
            return as_path.is_file().then(|| as_path.to_path_buf());
        }
        // (A module names its talk table without `.tlk`, as the game reads it: `Mod_CustomTlk` "mymodule" is `tlk/mymodule.tlk`.)
        let with_ext = as_path.extension().is_none().then(|| format!("{name}.tlk"));
        return ["tlk", "hak"]
            .iter()
            .flat_map(|d| [Some(name.to_string()), with_ext.clone()].into_iter().flatten().map(move |n| user.join(d).join(n)))
            .find(|p| p.is_file());
    }
    let found: Vec<std::path::PathBuf> = ["tlk", "hak"]
        .iter()
        .flat_map(|d| std::fs::read_dir(user.join(d)).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tlk")))
        .filter(|p| !p.file_stem().is_some_and(|s| ["dialog", "dialogf"].contains(&s.to_string_lossy().to_ascii_lowercase().as_str())))
        .collect();
    match found.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// A placeable: placeables.2da's row, its name and model.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Placeable {
    /// Its row in placeables.2da.
    pub row: usize,
    /// The row's `Label` column.
    pub label: String,
    /// The name shown for it: the talk table string of its `StrRef`, or the label when that string is empty or missing.
    pub name: String,
    /// Its model's resref (`ModelName`), lower case.
    pub model: String,
}

/// placeables.2da's rows with a model.
pub fn placeables(game: &GameData) -> Result<Vec<Placeable>> {
    let t = game.table("placeables")?;
    let mut out = Vec::new();
    for row in 0..t.len() {
        let Some(model) = t.get(row, "ModelName").map(str::trim).filter(|m| !m.is_empty() && *m != "****") else { continue };
        let label = t.get(row, "Label").unwrap_or("").to_string();
        let name = t
            .get(row, "StrRef")
            .and_then(|s| s.parse::<u32>().ok())
            .and_then(|s| game.string(StrRef(s)))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| label.clone());
        out.push(Placeable { row, label, name, model: model.to_ascii_lowercase() });
    }
    Ok(out)
}

/// A texture a model draws with: the name its meshes give (what `ReplaceObjectTexture` replaces), the image that name means (a material
/// can name another) and whether that image is a PLT (`plt`), a plain image (`plain`) or missing (`missing`). Only a plain image can stand in
/// for another in the game ("sNew must refer to a simple texture, not PLT").
#[derive(Clone, Debug, serde::Serialize)]
pub struct ModelTexture {
    /// The texture name its meshes give, lower case: what `ReplaceObjectTexture` replaces.
    pub name: String,
    /// The image that name means: the name itself, or the image a material of that name points to.
    pub image: String,
    /// `plain`, `plt` or `missing`.
    pub kind: &'static str,
    /// How many triangles use it: the largest is most of what is seen.
    pub triangles: usize,
}

/// The textures of model `resref`, most used first: each drawn mesh's diffuse texture, its `bitmap`, or for a mesh that names a material
/// (EE's `materialname`, an MTR) the material's `texture0`, which is the name the game and the renderer replace.
pub fn model_textures(game: &GameData, resref: &str) -> Result<Vec<ModelTexture>> {
    let data = game.resman.get_named(resref, ResType::MDL).with_context(|| format!("no model {resref}"))?;
    let model = Model::read(&data).with_context(|| format!("reading {resref}"))?;
    let named = |t: &Option<String>| t.clone().filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("null")).map(|t| t.to_ascii_lowercase());
    let mut count: std::collections::BTreeMap<String, usize> = Default::default();
    for node in &model.nodes {
        let NodeKind::Mesh(mesh) = &node.kind else { continue };
        if !mesh.render {
            continue;
        }
        let name = named(&mesh.textures[0]).or_else(|| named(&texture::material(game, &named(&mesh.material)?)?.textures[0]));
        let Some(name) = name else { continue };
        let n = mesh.triangles().count();
        if n > 0 {
            *count.entry(name).or_default() += n;
        }
    }
    let mut out: Vec<ModelTexture> = count
        .into_iter()
        .map(|(name, triangles)| {
            let (image, kind) = image_of(game, &name);
            ModelTexture { name, image, kind, triangles }
        })
        .collect();
    out.sort_by(|a, b| b.triangles.cmp(&a.triangles).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

/// The image a texture name means ([`texture::resolve`]) and its kind: `plain`, `plt` or `missing`.
fn image_of(game: &GameData, name: &str) -> (String, &'static str) {
    let Some(r) = texture::resolve(game, name) else { return (name.to_string(), "missing") };
    let kind = match r.kind(game, false) {
        Some(texture::Kind::Plain) => "plain",
        Some(texture::Kind::Plt) => "plt",
        None => "missing",
    };
    (r.image_name(), kind)
}

/// The model of placeables.2da row `row`, lower case.
pub fn placeable_model(game: &GameData, row: usize) -> Result<String> {
    let t = game.table("placeables")?;
    t.get(row, "ModelName")
        .map(str::trim)
        .filter(|m| !m.is_empty() && *m != "****")
        .map(str::to_ascii_lowercase)
        .with_context(|| format!("placeables.2da row {row} has no model"))
}

/// What a placeables.2da row's model is, for a browser of models: where it comes from, how big it is, whether it has particles, what it is
/// drawn with. One [`PlaceableInfo`] a row with a model; `missing` when the game has no such model.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlaceableInfo {
    /// Its row in placeables.2da.
    pub row: usize,
    /// The row's `Label` column.
    pub label: String,
    /// The name shown for it: the talk table string of its `StrRef`, or the label when that string is empty or missing.
    pub name: String,
    /// Its model's resref (`ModelName`), lower case.
    pub model: String,
    /// The layer the model comes from (a hak's label, or the game's own data), as the resource manager names it; None when missing.
    pub origin: Option<String>,
    /// The game has no model of that name, or it could not be read. Then `size`, `textures` and `art` are empty and `emitters` is
    /// false.
    pub missing: bool,
    /// Width, depth and height in metres of what its meshes cover at rest (None for a model of no mesh).
    pub size: Option<[f32; 3]>,
    /// What its meshes cover at rest: the lowest and highest corner (x, y, z) in model space, metres, to the centimetre (x east and y
    /// north as the model faces at bearing 0, z up from its origin). None for a model of no mesh.
    pub bounds: Option<[[f32; 3]; 2]>,
    /// Its placeable walkmesh (`.pwk`, of the model's name); None when it has none.
    pub walkmesh: Option<PlaceableWalkmesh>,
    /// It has particle emitters (a flame, a glow, smoke, water).
    pub emitters: bool,
    /// placeables.2da gives it a light (LightColor).
    pub light: bool,
    /// Its textures, most used first, as [`model_textures`] reads them: (name, kind).
    pub textures: Vec<(String, &'static str)>,
    /// A fingerprint of the files its picture is drawn from (the model, and for each texture its image, material and TXI), hex: a picture
    /// named by it is drawn again only when one of those files changes, not on every hak build. Empty for a missing model.
    pub art: String,
}

/// A placeable's walkmesh (`.pwk`), as [`placeables_index`] reports it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PlaceableWalkmesh {
    /// The faces whose surface can't be walked (`surfacemat.2da`'s `Walk` is not `1`): where the placeable blocks walking.
    pub blocking_faces: usize,
    /// The ground those faces cover: the lowest and highest corner (x, y) in model space, metres, to the centimetre (as `bounds`).
    /// None when no face blocks.
    pub footprint: Option<[[f32; 2]; 2]>,
}

/// A placeable's walkmesh (`.pwk` of `model`), read for [`placeables_index`]; None when the game has none or it can't be read.
fn placeable_walkmesh(game: &GameData, model: &str, walkable: &dyn Fn(u32) -> bool) -> Option<PlaceableWalkmesh> {
    let data = game.resman.get_named(model, ResType::PWK).ok()?;
    let w = Walkmesh::read(&data, WalkmeshKind::Placeable).ok()?;
    let pose = rest_pose(&w.model);
    let (mut lo, mut hi, mut faces) = ([f32::MAX; 2], [f32::MIN; 2], 0);
    for s in &w.surfaces {
        let NodeKind::Mesh(mesh) = &w.model.nodes[s.node].kind else { continue };
        for face in mesh.faces.iter().filter(|f| !walkable(f.material)) {
            faces += 1;
            for v in face.vertices {
                let q = pose[s.node].transform_point3(Vec3::from_array(mesh.vertices[v as usize]));
                lo = [lo[0].min(q.x), lo[1].min(q.y)];
                hi = [hi[0].max(q.x), hi[1].max(q.y)];
            }
        }
    }
    // (Plus 0.0: no -0.0.)
    let cm = |v: [f32; 2]| v.map(|x| (x * 100.0).round() / 100.0 + 0.0);
    Some(PlaceableWalkmesh { blocking_faces: faces, footprint: (faces > 0).then(|| [cm(lo), cm(hi)]) })
}

/// What [`placeables_index`] reads of a model, once however many rows use it.
#[derive(Clone, Default)]
struct ModelInfo {
    origin: Option<String>,
    missing: bool,
    /// The meshes' lowest and highest corner at rest, unrounded.
    extent: Option<(Vec3, Vec3)>,
    emitters: bool,
    textures: Vec<(String, &'static str)>,
    art: String,
    walkmesh: Option<PlaceableWalkmesh>,
}

/// FNV-1a, 64 bits: a fingerprint that stays the same from one build of this program to the next (std's hasher may not).
fn fnv(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3))
}
const FNV0: u64 = 0xcbf2_9ce4_8422_2325;

/// The fingerprint of a texture name's files: its material (MTR), the image it means (DDS, TGA or PLT) and that image's TXI.
fn texture_art(game: &GameData, name: &str, memo: &mut std::collections::HashMap<String, u64>) -> u64 {
    use mg_core::ResRef;
    use mg_resman::ResKey;
    if let Some(h) = memo.get(name) {
        return *h;
    }
    let mut h = fnv(FNV0, name.as_bytes());
    if let Some(r) = ResRef::cut(name) {
        if let Ok(d) = game.resman.get(&ResKey::new(r, ResType::MTR)) {
            h = fnv(h, &d);
        }
        let (image, _) = image_of(game, name);
        if let Some(i) = ResRef::cut(&image) {
            for t in [ResType::DDS, ResType::TGA, ResType::PLT, ResType::TXI] {
                if let Ok(d) = game.resman.get(&ResKey::new(i, t)) {
                    h = fnv(fnv(h, t.extension().unwrap_or("").as_bytes()), &d);
                }
            }
        }
    }
    memo.insert(name.to_string(), h);
    h
}

/// Every placeables.2da row with a model, described ([`PlaceableInfo`]). A model several rows share is read once.
pub fn placeables_index(game: &GameData) -> Result<Vec<PlaceableInfo>> {
    use mg_core::ResRef;
    use mg_resman::ResKey;
    let t = game.table("placeables")?;
    let surfacemat = game.table("surfacemat").ok();
    let walkable = |m: u32| surfacemat.as_ref().and_then(|t| t.get(m as usize, "Walk")).is_some_and(|v| v.trim() == "1");
    let mut seen: std::collections::HashMap<String, ModelInfo> = Default::default();
    // (Plus 0.0: no -0.0.)
    let cm = |v: Vec3| v.to_array().map(|x| (x * 100.0).round() / 100.0 + 0.0);
    let mut tex_memo = std::collections::HashMap::new();
    let mut out = Vec::new();
    for p in placeables(game)? {
        let light = t.get(p.row, "LightColor").and_then(|v| v.trim().parse::<i64>().ok()).is_some_and(|v| v > 0);
        let info = seen.entry(p.model.clone()).or_insert_with(|| {
            let origin = ResRef::cut(&p.model).and_then(|r| game.resman.origin(&ResKey::new(r, ResType::MDL))).map(str::to_string);
            let Some(origin) = origin else { return ModelInfo { missing: true, ..Default::default() } };
            let bytes = game.resman.get_named(&p.model, ResType::MDL).ok();
            let read = bytes.as_ref().and_then(|d| Model::read(d).ok());
            let Some(model) = read else { return ModelInfo { origin: Some(origin), missing: true, ..Default::default() } };
            let pose = rest_pose(&model);
            let (mut lo, mut hi, mut any) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), false);
            for (i, node) in model.nodes.iter().enumerate() {
                let NodeKind::Mesh(mesh) = &node.kind else { continue };
                if !mesh.render {
                    continue;
                }
                for v in &mesh.vertices {
                    let q = pose[i].transform_point3(Vec3::from_array(*v));
                    lo = lo.min(q);
                    hi = hi.max(q);
                    any = true;
                }
            }
            let emitters = model.nodes.iter().any(|n| matches!(n.kind, NodeKind::Emitter(_)));
            let textures: Vec<(String, &'static str)> =
                model_textures(game, &p.model).map_or_default(|v| v.into_iter().map(|t| (t.name, t.kind)).collect());
            let mut art = fnv(FNV0, bytes.as_deref().unwrap_or_default());
            for (name, _) in &textures {
                art = fnv(art, &texture_art(game, name, &mut tex_memo).to_le_bytes());
            }
            ModelInfo {
                origin: Some(origin),
                missing: false,
                extent: any.then_some((lo, hi)),
                emitters,
                textures,
                art: format!("{art:016x}"),
                walkmesh: placeable_walkmesh(game, &p.model, &walkable),
            }
        });
        out.push(PlaceableInfo {
            row: p.row,
            label: p.label,
            name: p.name,
            model: p.model,
            origin: info.origin.clone(),
            missing: info.missing,
            size: info.extent.map(|(lo, hi)| cm(hi - lo)),
            bounds: info.extent.map(|(lo, hi)| [cm(lo), cm(hi)]),
            walkmesh: info.walkmesh.clone(),
            emitters: info.emitters,
            light,
            textures: info.textures.clone(),
            art: info.art.clone(),
        });
    }
    Ok(out)
}

/// A model flattened: triangles in model space (x east, y north, z up, as
/// the model faces at bearing 0), grouped by texture, and the footprint of
/// its walkmesh's unwalkable faces (x, y).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Flat {
    /// Three positions a triangle.
    pub positions: Vec<[f32; 3]>,
    /// Three UVs a triangle.
    pub uvs: Vec<[f32; 2]>,
    /// (texture, first triangle, triangle count), in order.
    pub groups: Vec<(String, usize, usize)>,
    /// Transparency hints by group (foliage cut-outs keep their look).
    pub transparency: Vec<u32>,
    /// The walkmesh's unwalkable faces, as triangles on the ground (x, y) in model space: where the placeable blocks walking.
    pub footprint: Vec<[[f32; 2]; 3]>,
    /// The lowest corner of the triangles' bounding box, in model space (metres). Zero for a model with no drawn triangles.
    pub min: [f32; 3],
    /// The highest corner of the triangles' bounding box, in model space (metres). Zero for a model with no drawn triangles.
    pub max: [f32; 3],
}

/// Each node's model-space transform at rest.
pub fn rest_pose(model: &Model) -> Vec<Mat4> {
    let mut out: Vec<Mat4> = Vec::with_capacity(model.nodes.len());
    for (i, n) in model.nodes.iter().enumerate() {
        let local = Mat4::from_scale_rotation_translation(
            Vec3::splat(n.scale),
            Quat::from_array(n.orientation).normalize(),
            Vec3::from_array(n.position),
        );
        out.push(match n.parent.filter(|&p| p < i) {
            Some(p) => out[p] * local,
            None => local,
        });
    }
    out
}

/// Model `resref` flattened ([`Flat`]): every drawn mesh with a texture, at rest, and the footprint of its placeable walkmesh (PWK),
/// where a face is unwalkable when surfacemat.2da's `Walk` is not `1`. A mesh with no texture or `NULL` is left out. Fails when the
/// game has no such model or it can't be read.
pub fn flatten(game: &GameData, resref: &str) -> Result<Flat> {
    let data = game.resman.get_named(resref, ResType::MDL).with_context(|| format!("no model {resref}"))?;
    let model = Model::read(&data).with_context(|| format!("reading {resref}"))?;
    let pose = rest_pose(&model);
    let mut f = Flat { min: [f32::MAX; 3], max: [f32::MIN; 3], ..Default::default() };
    for (i, node) in model.nodes.iter().enumerate() {
        let NodeKind::Mesh(mesh) = &node.kind else { continue };
        let Some(texture) = mesh.textures[0].clone().filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("null")) else { continue };
        if !mesh.render || mesh.vertices.is_empty() {
            continue;
        }
        let m = pose[i];
        let start = f.positions.len() / 3;
        for t in mesh.triangles() {
            for v in t {
                let p = m.transform_point3(Vec3::from_array(mesh.vertices[v as usize]));
                for k in 0..3 {
                    f.min[k] = f.min[k].min(p[k]);
                    f.max[k] = f.max[k].max(p[k]);
                }
                f.positions.push(p.to_array());
                f.uvs.push(mesh.uvs[0].get(v as usize).copied().unwrap_or([0.0, 0.0]));
            }
        }
        let count = f.positions.len() / 3 - start;
        if count > 0 {
            f.groups.push((texture.to_ascii_lowercase(), start, count));
            f.transparency.push(mesh.transparency_hint);
        }
    }
    // The walkmesh: faces whose surface can't be walked block the ground.
    if let Ok(data) = game.resman.get_named(resref, ResType::PWK) {
        let w = Walkmesh::read(&data, WalkmeshKind::Placeable)?;
        let wpose = rest_pose(&w.model);
        let surfacemat = game.table("surfacemat").ok();
        let walkable = |m: u32| surfacemat.as_ref().and_then(|t| t.get(m as usize, "Walk")).is_some_and(|v| v.trim() == "1");
        for s in &w.surfaces {
            let NodeKind::Mesh(mesh) = &w.model.nodes[s.node].kind else { continue };
            let m = wpose[s.node];
            for face in &mesh.faces {
                if walkable(face.material) {
                    continue;
                }
                let p = face.vertices.map(|v| {
                    let q = m.transform_point3(Vec3::from_array(mesh.vertices[v as usize]));
                    [q.x, q.y]
                });
                f.footprint.push(p);
            }
        }
    }
    if f.positions.is_empty() {
        f.min = [0.0; 3];
        f.max = [0.0; 3];
    }
    Ok(f)
}
