// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A composed creature (a .bic or .utc) as meshes and textures: posed, skinned and PLT-coloured on the CPU, in world space, as the
//! toolset's renderer composes it (`mg_preview::compose`, `mg_render::anim`). Written as a bundle: `scene.json` plus `tex/*.png`.
//!
//! The device is only needed to load models: a software adapter (Mesa lavapipe) does as well as a GPU.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mg_gff::Gff;
use mg_mdl::Model;
use mg_render::Assets;
use mg_rules::GameData;
use serde_json::json;

use crate::items::{Items, MissingItem};
use crate::sync::Recover;

/// The device [`gpu`] gives, for callers that keep one.
pub use mg_render::Gpu;

/// Whether `NWN_BAKE_SOFTWARE` is set (to anything): use the software adapter even when there is a GPU.
fn software_only() -> bool {
    std::env::var_os("NWN_BAKE_SOFTWARE").is_some()
}

/// Reads the models a composition asks for, and remembers the ones the game has but that do not read, with why.
pub(crate) struct Loader<'a> {
    game: &'a GameData,
    broken: std::sync::Mutex<HashMap<String, String>>,
}

impl<'a> Loader<'a> {
    pub(crate) fn new(game: &'a GameData) -> Loader<'a> {
        Loader { game, broken: Default::default() }
    }

    pub(crate) fn load(&self, name: &str) -> Option<Arc<Model>> {
        let d = self.game.resman.get_named(name, mg_core::ResType::MDL).ok()?;
        match Model::read(&d) {
            Ok(m) => Some(Arc::new(m)),
            Err(e) => {
                self.broken.locked().insert(name.to_ascii_lowercase(), format!("{e:#}"));
                None
            }
        }
    }

    /// `missing` (the composition's models not loaded), each one that the game has but that did not read as `name: why`.
    pub(crate) fn missing(&self, missing: &[String]) -> Vec<String> {
        let broken = self.broken.locked();
        missing.iter().map(|m| broken.get(&m.to_ascii_lowercase()).map_or_else(|| m.clone(), |why| format!("{m}: {why}"))).collect()
    }
}

/// A device for the model loading: the real GPU if there is one, else a software adapter. `NWN_BAKE_SOFTWARE=1` goes straight to
/// software.
pub fn gpu() -> Option<Gpu> {
    if !software_only()
        && let Some(g) = Gpu::headless()
    {
        return Some(g);
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { force_fallback_adapter: true, ..Default::default() }))
            .ok()?;
    let features = adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nwn-bake"),
        required_features: features,
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .ok()?;
    Some(Gpu::new(device, queue))
}

/// The largest side of a texture in a baked scene. The HD textures are 2048 pixels (1 to 5 MB each as PNG, eight on one HD placeable) for a
/// 3D view a few hundred pixels across; a web view of that size loses nothing at 1024. Pictures are drawn from the game's own textures, not these.
const MAX_SCENE_TEXTURE: u32 = 1024;

/// `img` halved (each pixel the mean of four) until neither side is over `max`, or a side is odd.
fn shrink_to(mut img: mg_image::Rgba, max: u32) -> mg_image::Rgba {
    while (img.width > max || img.height > max) && img.width.is_multiple_of(2) && img.height.is_multiple_of(2) {
        let (w, h) = (img.width / 2, img.height / 2);
        let src = img.width as usize * 4;
        let mut data = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = 2 * y * src + 2 * x * 4;
                for c in 0..4 {
                    let sum = u32::from(img.data[i + c])
                        + u32::from(img.data[i + 4 + c])
                        + u32::from(img.data[i + src + c])
                        + u32::from(img.data[i + src + 4 + c]);
                    data[(y * w as usize + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        img = mg_image::Rgba { width: w, height: h, data };
    }
    img
}

/// A mesh's texture in a baked scene: lossy WebP (alpha kept), about a tenth of the PNG for the HD textures, which are detailed enough that PNG
/// barely shrinks them (1 to 2 MB each at 1024 pixels). Particle textures stay PNG: they are small, and their soft edges are all alpha.
const WEBP_QUALITY: f32 = 85.0;

fn write_webp(path: &Path, img: &mg_image::Rgba) -> Result<()> {
    let img = img.top_down();
    let encoded = webp::Encoder::from_rgba(&img.data, img.width, img.height).encode(WEBP_QUALITY);
    std::fs::write(path, &*encoded)?;
    Ok(())
}

fn write_png(path: &Path, img: &mg_image::Rgba) -> Result<()> {
    let img = img.top_down();
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(f, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img.data)?;
    Ok(())
}

/// What a picture of a creature left out.
#[derive(Clone, Debug, Default)]
pub struct Thumbnail {
    /// The models it names that the game does not have or cannot read (drawn without): a name, or `name: why`.
    pub missing_models: Vec<String>,
    /// The equipped items its blueprint names that `items` did not find (drawn without).
    pub missing_items: Vec<MissingItem>,
}

/// The creature `gff` (a .utc or .bic) as Moonglow composes it, its equipped items read from `items`, and the items not found.
/// Fails when `items` is strict and one is missing.
pub(crate) fn dressed(game: &GameData, gff: &Gff, items: &Items) -> Result<(mg_preview::Preview, Vec<MissingItem>)> {
    let worn = crate::items::equipped(game, &gff.root, items)?;
    let preview = mg_preview::creature(game, &gff.root, &|r| worn.get(r))?;
    Ok((preview, worn.missing))
}

/// A picture of the creature in `gff_bytes` (a .utc or .bic): a square PNG of `size` pixels, seen from the front and a little above, in animation `anim`
/// at `t` seconds, on a plain dark background, framed by its own bounds. Its particles are drawn as a placeable's are (run for a while first), so
/// a creature made only of them (a will-o'-wisp, a lantern archon, an elemental's flames) has a picture too. A blueprint's equipped items are read
/// from `items`. Says what it drew without: models the game does not have, items not found.
pub fn thumbnail(
    game: &GameData,
    gpu: &Gpu,
    gff_bytes: &[u8],
    items: &Items,
    out: &Path,
    size: u32,
    anim: &str,
    t: f32,
    yaw_degrees: f32,
) -> Result<Thumbnail> {
    let gff = Gff::read(gff_bytes)?;
    let (preview, missing_items) = dressed(game, &gff, items)?;
    let (missing_models, _) = thumbnail_of(
        game,
        gpu,
        &preview,
        out,
        size,
        size,
        anim,
        t,
        yaw_degrees,
        true,
        "the creature has no base model",
        "the creature has nothing to draw",
    )?;
    Ok(Thumbnail { missing_models, missing_items })
}

/// A picture of the item in `gff_bytes` (a .uti: a weapon with the three parts of its ModelPart1-3, a helmet, a shield...), on its own, framed by its bounds:
/// the same picture as [`thumbnail`] makes of a creature, `size` pixels wide and `height` high (a long weapon seen from the side fills a wide, low picture).
/// Returns the models it names that the game does not have.
pub fn item_thumbnail(
    game: &GameData,
    gpu: &Gpu,
    gff_bytes: &[u8],
    out: &Path,
    size: u32,
    height: u32,
    yaw_degrees: f32,
) -> Result<Vec<String>> {
    let gff = Gff::read(gff_bytes)?;
    let preview = mg_preview::item(game, &gff.root)?;
    Ok(thumbnail_of(
        game,
        gpu,
        &preview,
        out,
        size,
        height,
        "pause1",
        0.0,
        yaw_degrees,
        false,
        "the item has no model",
        "the item has nothing to draw",
    )?
    .0)
}

/// A picture of placeables.2da row `row` as a placed, not static piece of furniture shows it (its environment map and idle animation, as
/// [`mg_preview::placeable`] makes it, and its particles under way: a candle's flame, a brazier's fire, or all there is of a piece made only of
/// effects), with `textures` (old name, new name) drawn instead of the model's, as `ReplaceObjectTexture` does in the
/// game: the furniture catalogue's styles. `size` pixels wide and `height` high, from `yaw_degrees` round (0: from the front). Returns the models
/// it names that the game does not have, and whether particles are in the picture (a caller that kept pictures drawn before particles were
/// drawn knows which to draw again).
pub fn placeable_thumbnail(
    game: &GameData,
    gpu: &Gpu,
    row: u32,
    textures: &[(String, String)],
    out: &Path,
    size: u32,
    height: u32,
    yaw_degrees: f32,
) -> Result<(Vec<String>, bool)> {
    let preview = placeable_preview(game, row, textures)?;
    let idle = preview.idle.clone().unwrap_or_else(|| "default".into());
    let (missing, quads) = thumbnail_of(
        game,
        gpu,
        &preview,
        out,
        size,
        height,
        &idle,
        0.0,
        yaw_degrees,
        true,
        "the placeable has no model",
        "the placeable has nothing to draw",
    )?;
    Ok((missing, quads > 0))
}

/// What [`placeable_thumbnail`] draws: row `row` of placeables.2da with these texture replacements (an empty name on either side is skipped).
pub fn placeable_preview(game: &GameData, row: u32, textures: &[(String, String)]) -> Result<mg_preview::Preview> {
    let mut utp = mg_gff::Struct::new(0);
    utp.set("Appearance", mg_gff::Value::Dword(row));
    let mut preview = mg_preview::placeable(game, &utp).with_context(|| format!("placeables.2da row {row}"))?;
    for (old, new) in textures {
        if !old.trim().is_empty() && !new.trim().is_empty() {
            preview.base.textures.insert(old.trim().to_ascii_lowercase(), new.trim().to_ascii_lowercase());
        }
    }
    Ok(preview)
}

/// How long a picture's particles run before it is taken (seconds, in steps of PARTICLES_STEP): long enough for the slowest emitters the area
/// view knows to fill out (its PARTICLES_AHEAD).
const PARTICLES_AHEAD: f32 = 20.0;
const PARTICLES_STEP: f32 = 0.25;

/// The base model's particles after [`PARTICLES_AHEAD`] seconds, as quads facing `camera`, the model placed by `transform`; None for a
/// model without emitters. The simulation's random numbers start from the same seed every time, so the same model makes the same
/// picture.
pub(crate) fn particles_now(
    composed: &mg_preview::compose::Composed,
    anim: &str,
    t: f32,
    camera: &mg_render::Camera,
    transform: Mat4,
) -> Option<Vec<mg_render::particles::ParticleBatch>> {
    let (model, playing, pose) = composed.emitters(Some(anim), t)?;
    let mut sim = mg_render::particles::Particles::new(model);
    if sim.is_empty() {
        return None;
    }
    let mut ahead = 0.0;
    while ahead < PARTICLES_AHEAD {
        sim.update(model, playing, t, PARTICLES_STEP, &pose, transform);
        ahead += PARTICLES_STEP;
    }
    Some(sim.batches(model, playing, t, &pose, transform, camera.view()))
}

/// The box round the particles' quads that show: those at least a quarter opaque (a flame's faint trail of smoke would make the flame a speck in
/// the picture). All of them when none is.
pub(crate) fn particle_bounds(batches: &[mg_render::particles::ParticleBatch]) -> Option<(Vec3, Vec3)> {
    let all = || batches.iter().flat_map(|b| b.vertices.iter());
    let seen = all().any(|v| v.color[3] >= 0.25);
    let mut it = all().filter(|v| !seen || v.color[3] >= 0.25).map(|v| Vec3::from_array(v.pos));
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

fn thumbnail_of(
    game: &GameData,
    gpu: &Gpu,
    preview: &mg_preview::Preview,
    out: &Path,
    size: u32,
    height: u32,
    anim: &str,
    t: f32,
    yaw_degrees: f32,
    particles: bool,
    no_base: &str,
    nothing: &str,
) -> Result<(Vec<String>, usize)> {
    let loader = Loader::new(game);
    let load = |n: &str| loader.load(n);
    let composed = mg_preview::compose::Composed::new(gpu, preview, &load).context(no_base.to_string())?;
    // Framed by the meshes and, where they are drawn, the particles once under way (seen from the picture's side, so the quads face about the
    // same way): a candle's glow stays in the picture, and a piece made only of particles is framed by them.
    let side = mg_render::Camera::orbit(Vec3::ZERO, 10.0, yaw_degrees.to_radians(), 18f32.to_radians());
    let sparks =
        if particles { particles_now(&composed, anim, t, &side, Mat4::IDENTITY).as_deref().and_then(particle_bounds) } else { None };
    let (min, max) = match (composed.has_meshes(), sparks) {
        (true, None) => composed.bounds_in(Some(anim), t),
        (true, Some((lo, hi))) => {
            let (a, b) = composed.bounds_in(Some(anim), t);
            (a.min(lo), b.max(hi))
        }
        (false, Some(found)) => found,
        (false, None) => anyhow::bail!("{nothing}"),
    };
    let radius = ((max - min).length() * 0.5).max(0.1);
    let mut renderer = mg_render::Renderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm, 4);
    // The view is framed by its width: in a picture lower than it is wide the camera comes closer in proportion.
    let fit = (height as f32 / size as f32).min(1.0);
    let camera = mg_render::Camera::orbit(
        (min + max) * 0.5,
        radius / 20f32.to_radians().sin() * 1.05 * fit,
        yaw_degrees.to_radians(),
        18f32.to_radians(),
    );
    let scene = mg_render::Scene {
        instances: composed.instances(Some(anim), t, Mat4::IDENTITY),
        area: mg_render::AreaLight::default(),
        background: [0.10, 0.11, 0.14],
        particles: if particles { particles_now(&composed, anim, t, &camera, Mat4::IDENTITY).unwrap_or_default() } else { Vec::new() },
        ..Default::default()
    };
    let quads = scene.particles.iter().map(|b| b.vertices.len() / 6).sum::<usize>();
    let img = renderer.render_image(gpu, &game.resman, &scene, &camera, size, height);
    // (`render_image` gives the rows top first, as a PNG wants them.)
    write_rgba_png(out, img.width, img.height, &img.data)?;
    Ok((loader.missing(&composed.missing), quads))
}

/// Writes RGBA pixels, rows top first, as a PNG.
pub(crate) fn write_rgba_png(out: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(out).with_context(|| format!("writing {}", out.display()))?);
    let mut enc = png::Encoder::new(f, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// The texture a mesh is drawn with before renames, as `mg_render` picks it (`Renderer::slots`): the texture0 of the material the model
/// names (`materialname`) comes first, since models made for EE often keep a placeholder bitmap beside it (`toolcolors`, a 16x16 swatch of
/// four colours: some HD placeables drew red, yellow and blue); else the bitmap (whose own lookup follows an MTR of its name); else the
/// material's name. A PLT part's MTR names no texture0, so it keeps its bitmap.
pub(crate) fn mesh_texture(game: &GameData, bitmap: Option<&String>, mtr: Option<&String>) -> Option<String> {
    mtr.and_then(|n| crate::texture::material(game, n))
        .and_then(|m| m.textures[0].clone())
        .or_else(|| bitmap.cloned())
        .or_else(|| mtr.cloned())
}

/// A mesh's normal, specular and roughness maps (MTR slots 1 to 3), as `mg_render` picks them (`Renderer::slots`): the model's own
/// `texture1..3`, each replaced by the material's where it names one (the material the model names, else one named like its bitmap).
/// Only for a mesh drawn normal-mapped (a render hint on the model or the material): without one the game's shaders read no maps. None
/// for a material with its own shader (its slots mean something else).
pub(crate) fn material_maps(game: &GameData, m: &mg_mdl::Mesh, mtr: Option<&String>) -> [Option<String>; 3] {
    let found = mtr
        .and_then(|n| crate::texture::material(game, n))
        .or_else(|| m.textures[0].as_deref().and_then(|n| crate::texture::material(game, n)));
    let hinted =
        m.renderhint.as_deref().is_some_and(|h| ["normalandspecmapped", "normaltangents"].iter().any(|w| h.eq_ignore_ascii_case(w)))
            || found.as_ref().is_some_and(|f| f.renderhint != mg_image::mtr::RenderHint::None);
    let custom =
        found.as_ref().is_some_and(|f| f.shader_fs.as_deref().is_some_and(|s| !s.trim().is_empty() && !s.eq_ignore_ascii_case("null")));
    if !hinted || custom {
        return [None, None, None];
    }
    let mut names = [m.textures[1].clone(), m.textures[2].clone(), m.textures[3].clone()];
    if let Some(f) = &found {
        for (i, n) in names.iter_mut().enumerate() {
            if f.textures[i + 1].is_some() {
                *n = f.textures[i + 1].clone();
            }
        }
    }
    names
}

/// A normal map as a viewer reads it (RGB, tangent space, Z out): the game's shaders read only its red and green and rebuild Z
/// (`z = sqrt(1 - x^2 - y^2)`), so blue is whatever the file had; it is rebuilt here the same way.
fn rebuild_normal_z(img: &mut mg_image::Rgba) {
    for px in img.data.as_chunks_mut::<4>().0.iter_mut() {
        let x = f32::from(px[0]) / 127.5 - 1.0;
        let y = f32::from(px[1]) / 127.5 - 1.0;
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        px[2] = ((z * 0.5 + 0.5) * 255.0).round() as u8;
        px[3] = 255;
    }
}

/// The material map of a normal-mapped mesh: green the roughness, blue the metalness (red unused, 255), as the renderer's shader works
/// them out (`SetupSpecularity` in mg_render's shader.wgsl): `spec0` is the specular map's red (else 0.04); the roughness is the roughness
/// map's red, else `mix(0.125, 0.55, (1 - spec0)^2)`; the metalness is `m / (0.04 + 0.96 m)` with `m = spec0^2`. At the larger of the two
/// maps' sizes (the other sampled to it). None if neither map could be read.
fn material_map(spec: Option<&mg_image::Rgba>, rough: Option<&mg_image::Rgba>) -> Option<mg_image::Rgba> {
    let (w, h) = match (spec, rough) {
        (Some(a), Some(b)) => (a.width.max(b.width), a.height.max(b.height)),
        (Some(a), None) | (None, Some(a)) => (a.width, a.height),
        (None, None) => return None,
    };
    let red = |img: &mg_image::Rgba, x: u32, y: u32| {
        let (sx, sy) =
            ((u64::from(x) * u64::from(img.width) / u64::from(w)) as u32, (u64::from(y) * u64::from(img.height) / u64::from(h)) as u32);
        f32::from(img.pixel(sx, sy)[0]) / 255.0
    };
    let mut out = mg_image::Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let spec0 = spec.map_or(0.04, |s| red(s, x, y));
            let rough = rough.map_or_else(|| 0.125 + (0.55 - 0.125) * (1.0 - spec0) * (1.0 - spec0), |r| red(r, x, y));
            let m = spec0 * spec0;
            let metal = (m / (0.04 + 0.96 * m)).clamp(0.0, 1.0);
            let i = (y as usize * w as usize + x as usize) * 4;
            out.data[i..i + 4].copy_from_slice(&[255, (rough * 255.0).round() as u8, (metal * 255.0).round() as u8, 255]);
        }
    }
    Some(out)
}

/// What a bake wrote.
pub struct Baked {
    /// How many meshes `scene.json` lists.
    pub meshes: usize,
    /// How many particle emitters `scene.json` lists.
    pub emitters: usize,
    /// How many texture files went to `tex/`.
    pub textures: usize,
    /// Models the creature names that the game does not have or cannot read (it is drawn without them): a name, or `name: why`
    /// for a model the game has that does not read.
    pub missing_models: Vec<String>,
    /// The equipped items its blueprint names that were not found (it is drawn without them).
    pub missing_items: Vec<MissingItem>,
}

/// Bakes the creature in `gff_bytes` (a .bic or .utc) into `out` (`scene.json`, `tex/`), in animation `anim` at `t` seconds, a
/// blueprint's equipped items read from `items`.
pub fn bake_creature(game: &GameData, gpu: &Gpu, gff_bytes: &[u8], items: &Items, out: &Path, anim: &str, t: f32) -> Result<Baked> {
    let gff = Gff::read(gff_bytes)?;
    let (mut preview, missing_items) = dressed(game, &gff, items)?;
    // A held weapon's visual effect (its flames, its sparks), which Moonglow does not compose.
    let effects = crate::emitters::weapon_effects(game, &gff.root, &preview);
    preview.parts.extend(effects);
    let nm = |l: &str| gff.root.locstring(l).and_then(|s| game.locstring(s)).unwrap_or_default();
    let name = format!("{} {}", nm("FirstName"), nm("LastName")).trim().to_string();
    let level: i64 = gff.root.list("ClassList").unwrap_or(&[]).iter().filter_map(|c| c.integer("ClassLevel")).sum();
    let race = game
        .table("racialtypes")
        .ok()
        .and_then(|t| game.string(mg_core::StrRef(t.get_int(gff.root.integer("Race")? as usize, "Name")? as u32)))
        .unwrap_or_default();
    let about = json!({ "name": name, "race": race, "level": level });
    let mut baked = bake_scene(game, gpu, &preview, out, anim, t, about, "the creature has no base model")?;
    baked.missing_items = missing_items;
    Ok(baked)
}

/// Bakes placeables.2da row `row` into `out` (`scene.json`, `tex/`) as [`bake_creature`] does a creature: the model a placed piece of
/// furniture shows ([`placeable_preview`]: its environment map, `textures` (old name, new name) drawn instead of the model's, as the
/// furniture catalogue's styles), in animation `anim` (None: the model's idle animation, else `default`) at `t` seconds. The scene names
/// the row (`row`), its model (`model`) and its name (`name`: the row's StrRef, else its label). A row whose model has no meshes (a piece
/// made only of particles) bakes with no meshes; a row with no model is an error.
pub fn bake_placeable(
    game: &GameData,
    gpu: &Gpu,
    row: u32,
    textures: &[(String, String)],
    out: &Path,
    anim: Option<&str>,
    t: f32,
) -> Result<Baked> {
    let preview = placeable_preview(game, row, textures)?;
    let anim = anim.map(str::to_string).or_else(|| preview.idle.clone()).unwrap_or_else(|| "default".into());
    let table = game.table("placeables").ok();
    let cell = |c: &str| table.as_ref().and_then(|t| t.get(row as usize, c)).map(str::trim).filter(|v| !v.is_empty() && *v != "****");
    let name = cell("StrRef")
        .and_then(|v| v.parse::<u32>().ok())
        .and_then(|s| game.string(mg_core::StrRef(s)))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| cell("Label").map(|l| l.trim_matches('"').to_string()))
        .unwrap_or_default();
    let model = crate::placeable_model(game, row as usize).ok();
    let about = json!({ "name": name, "row": row, "model": model });
    bake_scene(game, gpu, &preview, out, &anim, t, about, "the placeable has no model")
}

/// Writes `preview` as one scene into `out`: `scene.json` (the fields of `about`, then `animation`, `t`, `meshes` and `emitters`) and its
/// textures in `tex/`. `no_model` is the error when the preview has no base model.
#[allow(clippy::too_many_arguments)]
fn bake_scene(
    game: &GameData,
    gpu: &Gpu,
    preview: &mg_preview::Preview,
    out: &Path,
    anim: &str,
    t: f32,
    about: serde_json::Value,
    no_model: &'static str,
) -> Result<Baked> {
    std::fs::create_dir_all(out.join("tex"))?;
    let loader = Loader::new(game);
    let load = |n: &str| loader.load(n);
    let composed = mg_preview::compose::Composed::new(gpu, preview, &load).context(no_model)?;
    let mut tex_files: HashMap<String, Option<(String, bool)>> = HashMap::new();
    let mut meshes = Vec::new();
    for inst in composed.instances(Some(anim), t, Mat4::IDENTITY) {
        let gm = &inst.model;
        let pose: &Vec<Mat4> = inst.pose.as_deref().unwrap_or(&gm.rest);
        for (j, gmesh) in gm.meshes.iter().enumerate() {
            let Some(m) = gm.mesh_data(j) else { continue };
            let node_now = pose.get(gmesh.node).copied().unwrap_or(Mat4::IDENTITY);
            // Skin: bind pose to current pose, in the skin node's space (as mg_render's renderer does).
            let bones: Option<Vec<Mat4>> = gmesh.skin.as_ref().map(|s| {
                let to_skin = node_now.inverse();
                s.bones.iter().zip(&s.inverse_bind).map(|(&b, ib)| to_skin * pose.get(b).copied().unwrap_or(Mat4::IDENTITY) * *ib).collect()
            });
            let skin_w = match &m.extra {
                mg_mdl::MeshExtra::Skin(s) if bones.is_some() => Some(&s.weights),
                _ => None,
            };
            let world = inst.transform * node_now;
            let (mut pos, mut nor, mut uv) = (Vec::new(), Vec::new(), Vec::new());
            for (v, p) in m.vertices.iter().enumerate() {
                let n = m.normals.get(v).copied().unwrap_or([0.0, 0.0, 1.0]);
                let (p, n) = (Vec3::from(*p), Vec3::from(n));
                let (p, n) = match (skin_w, &bones) {
                    (Some(w), Some(b)) => {
                        let (mut acc_p, mut acc_n) = (Vec3::ZERO, Vec3::ZERO);
                        for (bi, wt) in w[v] {
                            if wt <= 0.0 {
                                continue;
                            }
                            let Some(mx) = b.get(bi as usize) else { continue };
                            acc_p += mx.transform_point3(p) * wt;
                            acc_n += mx.transform_vector3(n) * wt;
                        }
                        (acc_p, acc_n)
                    }
                    _ => (p, n),
                };
                let wp = world.transform_point3(p);
                let wn = world.transform_vector3(n).normalize_or_zero();
                pos.extend_from_slice(&wp.to_array());
                nor.extend_from_slice(&wn.to_array());
                let u = m.uvs[0].get(v).copied().unwrap_or([0.0; 2]);
                uv.extend_from_slice(&u);
            }
            let idx: Vec<u32> = m.triangles().flatten().collect();

            // The texture the renderer would pick (renames, then PLT colours on the diffuse texture).
            let tex = mesh_texture(game, m.textures[0].as_ref(), gmesh.material.mtr.as_ref())
                .or_else(|| inst.textures.as_ref()?.get("").cloned())
                // A mesh naming no texture takes one named like its model (helmets), if there is one.
                .or_else(|| Assets::texture(&game.resman, &gm.model.name.to_ascii_lowercase()).map(|_| gm.model.name.to_ascii_lowercase()))
                .map(|n| {
                    let n = inst.textures.as_ref().and_then(|t| t.get(&n)).cloned().unwrap_or(n);
                    match inst.plt_colors {
                        Some(c) => mg_render::colored_name(&n, c),
                        None => n,
                    }
                });
            let tex_ref = tex.as_ref().and_then(|name| {
                tex_files
                    .entry(name.clone())
                    .or_insert_with(|| {
                        let lt = Assets::texture(&game.resman, name)?;
                        let file = format!("{}.webp", name.replace(['#', ','], "_"));
                        let mut rgba = lt.texture.to_rgba();
                        // An environment-mapped texture (its TXI has `envmaptexture`) uses its alpha as the reflection's mask, not as
                        // transparency: the engine draws it solid, and where the alpha is low the colour tends to grey (the renderer's
                        // `env_level = 1 - alpha`, `mix(rgb, 0.666667, env_level)`). Baked so, with the alpha made opaque, so the
                        // shiny parts of a weapon do not vanish or turn black when a viewer treats alpha as a cut-out.
                        let env = lt.txi.envmap().is_some();
                        if env {
                            for px in rgba.data.as_chunks_mut::<4>().0.iter_mut() {
                                let a = f32::from(px[3]) / 255.0;
                                for c in &mut px[..3] {
                                    *c = (f32::from(*c) * a + 0.666_667 * 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
                                }
                                px[3] = 255;
                            }
                        }
                        write_webp(&out.join("tex").join(&file), &shrink_to(rgba, MAX_SCENE_TEXTURE)).ok()?;
                        Some((file, lt.texture.has_alpha && !env))
                    })
                    .clone()
            });
            // Its normal map (Z rebuilt) and its material map (roughness in green, metalness in blue, as the glTF and three.js read
            // them), made from its specular and roughness maps the way the renderer reads them; written as its texture is (WebP, at
            // most MAX_SCENE_TEXTURE). Keyed apart from the colour textures, as a name may be both.
            let [normal_name, spec_name, rough_name] = material_maps(game, m, gmesh.material.mtr.as_ref());
            let normal_map = normal_name.as_ref().and_then(|name| {
                tex_files
                    .entry(format!("normal:{name}"))
                    .or_insert_with(|| {
                        let mut rgba = Assets::texture(&game.resman, name)?.texture.to_rgba();
                        rebuild_normal_z(&mut rgba);
                        let file = format!("{}.webp", name.replace(['#', ','], "_"));
                        write_webp(&out.join("tex").join(&file), &shrink_to(rgba, MAX_SCENE_TEXTURE)).ok()?;
                        Some((file, false))
                    })
                    .as_ref()
                    .map(|t| t.0.clone())
            });
            let material_map = (spec_name.is_some() || rough_name.is_some())
                .then(|| {
                    let key = format!("material:{}+{}", spec_name.as_deref().unwrap_or("-"), rough_name.as_deref().unwrap_or("-"));
                    tex_files
                        .entry(key)
                        .or_insert_with(|| {
                            let spec = spec_name.as_deref().and_then(|n| Assets::texture(&game.resman, n)).map(|t| t.texture.to_rgba());
                            let rough = rough_name.as_deref().and_then(|n| Assets::texture(&game.resman, n)).map(|t| t.texture.to_rgba());
                            let img = material_map(spec.as_ref(), rough.as_ref())?;
                            let stem =
                                format!("{}_{}", spec_name.as_deref().unwrap_or("nospec"), rough_name.as_deref().unwrap_or("norough"));
                            let file = format!("mat_{}.webp", stem.replace(['#', ','], "_"));
                            write_webp(&out.join("tex").join(&file), &shrink_to(img, MAX_SCENE_TEXTURE)).ok()?;
                            Some((file, false))
                        })
                        .as_ref()
                        .map(|t| t.0.clone())
                })
                .flatten();
            meshes.push(json!({
                "model": gm.model.name, "node": gm.model.nodes[gmesh.node].name,
                "texture": tex, "file": tex_ref.as_ref().map(|t| &t.0), "alpha": tex_ref.as_ref().map(|t| t.1),
                "normalMap": normal_map, "materialMap": material_map,
                "materialAlpha": gmesh.material.alpha, "hint": gmesh.material.transparency_hint,
                "diffuse": gmesh.material.diffuse.to_array(), "skinned": bones.is_some(),
                "position": pos, "normal": nor, "uv": uv, "index": idx,
            }));
        }
    }
    // The particle emitters of every piece (a weapon's flames, a brazier's fire), for the viewer to run; their textures are written as
    // the meshes' are, keeping the alpha (a particle's shape).
    let mut emitters = Vec::new();
    for inst in composed.instances(Some(anim), t, Mat4::IDENTITY) {
        let gm = &inst.model;
        if !crate::emitters::has_emitters(&gm.model) {
            continue;
        }
        let pose: &Vec<Mat4> = inst.pose.as_deref().unwrap_or(&gm.rest);
        let mut texture = |name: &str| -> Option<String> {
            tex_files
                .entry(format!("particle:{name}"))
                .or_insert_with(|| {
                    let lt = Assets::texture(&game.resman, name)?;
                    // (Its own file: a mesh's copy of the same texture may have had its alpha made opaque.)
                    let file = format!("particle_{}.png", name.replace(['#', ','], "_"));
                    write_png(&out.join("tex").join(&file), &lt.texture.to_rgba()).ok()?;
                    Some((file, lt.texture.has_alpha))
                })
                .clone()
                .map(|f| f.0)
        };
        for mut e in crate::emitters::describe(&gm.model, pose, inst.transform, anim, t, &mut texture) {
            e["model"] = json!(gm.model.name);
            emitters.push(e);
        }
    }
    let (n_meshes, n_emitters) = (meshes.len(), emitters.len());
    let mut scene = match about {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    scene.insert("animation".into(), json!(anim));
    scene.insert("t".into(), json!(t));
    scene.insert("meshes".into(), serde_json::Value::Array(meshes));
    scene.insert("emitters".into(), serde_json::Value::Array(emitters));
    std::fs::write(out.join("scene.json"), serde_json::to_vec(&scene)?)?;
    Ok(Baked {
        meshes: n_meshes,
        emitters: n_emitters,
        textures: tex_files.len(),
        missing_models: loader.missing(&composed.missing),
        missing_items: Vec::new(),
    })
}
