// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A creature as a *list of parts*, for a viewer that assembles it itself.
//!
//! [`describe`] says what a character is made of: the base model (the skeleton, and the body where it has one) and each part hung on it
//! (a head, an armour piece, a weapon, a cloak, wings), with the PLT colours of each. [`bake_part`] makes the world-space meshes of
//! *one* part, posed on the base's skeleton, and nothing but that part: a part's meshes do not depend on the other parts or on its
//! colours, so they are made once, cached for good, and shared by every character that wears it. A recolour or a swap of one part
//! then costs a viewer nothing it does not already have, or one small file.
//!
//! The file of a part ([`Chunk`]): `NWPC`, a version (u32), the length of the JSON (u32), the JSON (padded to 4 bytes), then the
//! numbers it points at: per mesh positions (f32 x 3), normals (f32 x 3), texture coordinates (f32 x 2) and triangle indices (u32), all little endian.
//! Textures are not in it: a mesh names its texture, and [`texture_kind`] says whether that is a plain image or a PLT. The JSON also lists the
//! part's particle emitters (`emitters`, see [`crate::emitters`]), whose textures are plain images.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use mg_core::{ResRef, ResType};
use mg_gff::Gff;
use mg_mdl::Model;
use mg_render::Assets;
use mg_resman::ResKey;
use mg_rules::GameData;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

/// What decides a part's meshes (its colours do not).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartSpec {
    /// The model's resref, lower case.
    pub model: String,
    /// The base model's node it hangs from, lower case; None for the base's origin.
    pub attach: Option<String>,
    /// The size it is drawn at, the same along every axis (1 is the model's own size).
    pub scale: f32,
    /// Textures it draws with in place of the model's own, lower case, from the model's name to the one used.
    pub textures: BTreeMap<String, String>,
    /// The environment map (texture alpha is how much it reflects); None for only what its textures ask for.
    pub env_map: Option<String>,
    /// It plays the animation on its own nodes: skinned robes and cloaks, wings and tails.
    pub animated: bool,
    /// Whether the part is coloured (PLT) at all: a coloured part takes a PLT before a plain image of the same name.
    pub colored: bool,
}

/// A part of the look and the colours it is in.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookPart {
    /// Names its meshes (a hash of the base, the pose and the part); the file of the part is `chunk/<id>`.
    pub id: String,
    /// What decides its meshes.
    #[serde(flatten)]
    pub spec: PartSpec,
    /// A colour per PLT layer (skin, hair, metal 1 and 2, cloth 1 and 2, leather 1 and 2, tattoo 1 and 2).
    pub colors: Option<[u8; 10]>,
}

/// A creature as parts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Look {
    /// The animation it stands in.
    pub anim: String,
    /// The time into the animation, in seconds.
    pub t: f32,
    /// The base model: the skeleton, and the body where the model has one.
    pub base: LookPart,
    /// What hangs on the base: head, armour pieces, weapons, cloak, wings, tail, and a held weapon's visual effect.
    pub parts: Vec<LookPart>,
    /// The equipped items its blueprint names that were not found ([`crate::items::equipped`]): it is drawn without them.
    #[serde(default)]
    pub missing_items: Vec<crate::items::MissingItem>,
}

fn spec_of(p: &mg_preview::Part) -> PartSpec {
    PartSpec {
        model: p.model.clone(),
        attach: p.attach.clone(),
        scale: p.scale,
        textures: p.textures.clone(),
        env_map: p.env_map.clone(),
        animated: p.animated,
        colored: p.colors.is_some(),
    }
}

fn part_of(spec: &PartSpec) -> mg_preview::Part {
    mg_preview::Part {
        model: spec.model.clone(),
        attach: spec.attach.clone(),
        scale: spec.scale,
        textures: spec.textures.clone(),
        colors: spec.colored.then_some([0; 10]),
        env_map: spec.env_map.clone(),
        // (New in Moonglow 3: not carried in a PartSpec yet, so a part is made as it was. `item` marks what a creature wears or
        // holds, which its texture replacements leave alone in 3.3 and later.)
        env_map_default: false,
        material_values: Vec::new(),
        animated: spec.animated,
        local: None,
        item: false,
    }
}

/// The name of a part's file: what it is made of, on which skeleton, in which pose.
pub fn chunk_id(base: &PartSpec, anim: &str, t: f32, part: Option<&PartSpec>) -> String {
    // (v 2: a part's file also carries its particle emitters.)
    let key = serde_json::to_vec(&json!({ "v": 2, "base": base, "anim": anim, "t": t, "part": part })).unwrap_or_default();
    Sha256::digest(&key).iter().take(10).map(|b| format!("{b:02x}")).collect()
}

/// What the character in `gff_bytes` (a .bic or .utc) is made of, standing in `anim` at `t` seconds, a blueprint's equipped items
/// read from `items`.
pub fn describe(game: &GameData, gff_bytes: &[u8], items: &crate::items::Items, anim: &str, t: f32) -> Result<Look> {
    let gff = Gff::read(gff_bytes)?;
    let (mut preview, missing_items) = crate::creature::dressed(game, &gff, items)?;
    // A held weapon's visual effect (its flames, its sparks), which Moonglow does not compose.
    let effects = crate::emitters::weapon_effects(game, &gff.root, &preview);
    preview.parts.extend(effects);
    let base_spec = spec_of(&preview.base);
    let part = |p: &mg_preview::Part| {
        let spec = spec_of(p);
        LookPart { id: chunk_id(&base_spec, anim, t, Some(&spec)), colors: p.colors, spec }
    };
    Ok(Look {
        anim: anim.to_string(),
        t,
        base: LookPart { id: chunk_id(&base_spec, anim, t, None), colors: preview.base.colors, spec: base_spec.clone() },
        parts: preview.parts.iter().map(part).collect(),
        missing_items,
    })
}

/// A part's meshes as a file (see the module), and the textures its meshes name (each with whether it is a PLT).
pub struct Chunk {
    /// The file's bytes (see the module).
    pub bytes: Vec<u8>,
    /// The textures its meshes and emitters name, sorted, each with whether it is a PLT. An emitter's texture is never one.
    pub textures: Vec<(String, bool)>,
}

/// Whether a texture is a PLT or a plain image ([`crate::texture::Kind`]).
pub use crate::texture::Kind as TextureKind;

/// The image a texture name means and whether it is a PLT ([`crate::texture::resolve`], the rule `mg_render` draws with); a coloured
/// part takes a PLT before a plain image of the same name.
pub fn texture_kind(game: &GameData, name: &str, colored: bool) -> Option<(String, TextureKind)> {
    let r = crate::texture::resolve(game, name)?;
    let kind = r.kind(game, colored)?;
    Some((r.image_name(), kind))
}

/// The meshes of one part (or of the base, `part` None) on `base`'s skeleton in `anim` at `t`.
pub fn bake_part(game: &GameData, gpu: &mg_render::Gpu, base: &PartSpec, anim: &str, t: f32, part: Option<&PartSpec>) -> Result<Chunk> {
    let preview = mg_preview::Preview {
        base: part_of(base),
        parts: part.map(part_of).into_iter().collect(),
        idle: Some(anim.to_string()),
        lights: Vec::new(),
    };
    let load = |n: &str| -> Option<Arc<Model>> {
        let d = game.resman.get_named(n, ResType::MDL).ok()?;
        Model::read(&d).ok().map(Arc::new)
    };
    let composed = mg_preview::compose::Composed::new(gpu, &preview, &load).context("the creature has no base model")?;
    let instances = composed.instances(Some(anim), t, Mat4::IDENTITY);
    let inst = instances.get(usize::from(part.is_some())).context("that part has no model")?;
    let gm = &inst.model;
    let pose: &Vec<Mat4> = inst.pose.as_deref().unwrap_or(&gm.rest);
    let (mut meta, mut bin) = (Vec::new(), Vec::<u8>::new());
    let mut texture_info: BTreeMap<String, (bool, bool)> = BTreeMap::new(); // texture -> (has alpha, environment mapped)
    for (j, gmesh) in gm.meshes.iter().enumerate() {
        let Some(m) = gm.mesh_data(j) else { continue };
        let node_now = pose.get(gmesh.node).copied().unwrap_or(Mat4::IDENTITY);
        let bones: Option<Vec<Mat4>> = gmesh.skin.as_ref().map(|s| {
            let to_skin = node_now.inverse();
            s.bones.iter().zip(&s.inverse_bind).map(|(&b, ib)| to_skin * pose.get(b).copied().unwrap_or(Mat4::IDENTITY) * *ib).collect()
        });
        let skin_w = match &m.extra {
            mg_mdl::MeshExtra::Skin(s) if bones.is_some() => Some(&s.weights),
            _ => None,
        };
        let world = inst.transform * node_now;
        let (mut pos, mut nor, mut uv) = (Vec::<f32>::new(), Vec::<f32>::new(), Vec::<f32>::new());
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
            pos.extend_from_slice(&world.transform_point3(p).to_array());
            nor.extend_from_slice(&world.transform_vector3(n).normalize_or_zero().to_array());
            uv.extend_from_slice(&m.uvs[0].get(v).copied().unwrap_or([0.0; 2]));
        }
        let idx: Vec<u32> = m.triangles().flatten().collect();

        // The texture the renderer would pick (renames first; PLT colours are a viewer's business).
        let tex = crate::creature::mesh_texture(game, m.textures[0].as_ref(), gmesh.material.mtr.as_ref())
            .or_else(|| inst.textures.as_ref()?.get("").cloned())
            // A mesh naming no texture takes one named like its model (helmets), if there is one.
            .or_else(|| Assets::texture(&game.resman, &gm.model.name.to_ascii_lowercase()).map(|_| gm.model.name.to_ascii_lowercase()))
            .map(|n| inst.textures.as_ref().and_then(|t| t.get(&n)).cloned().unwrap_or(n).to_ascii_lowercase());
        let resolved = tex.as_deref().and_then(|n| texture_kind(game, n, part.map_or(base.colored, |p| p.colored)));
        let (image, plt) = match &resolved {
            Some((image, kind)) => (Some(image.clone()), *kind == TextureKind::Plt),
            None => (None, false),
        };
        let (mut alpha, mut env) = (false, false);
        if let (Some(image), false) = (&image, plt) {
            let &mut (a, e) = texture_info.entry(image.clone()).or_insert_with(|| match Assets::texture(&game.resman, image) {
                Some(lt) => (lt.texture.has_alpha, lt.txi.envmap().is_some()),
                None => (false, false),
            });
            // An environment-mapped texture's alpha is its reflection mask, not transparency (see creature.rs).
            (alpha, env) = (a && !e, e);
        }
        let at = |b: &Vec<u8>| b.len();
        let po = at(&bin);
        for f in &pos {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        let no = at(&bin);
        for f in &nor {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        let uo = at(&bin);
        for f in &uv {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        let io = at(&bin);
        for i in &idx {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        meta.push(json!({
            "node": gm.model.nodes[gmesh.node].name, "texture": image, "plt": plt, "alpha": alpha, "env": env,
            "materialAlpha": gmesh.material.alpha, "hint": gmesh.material.transparency_hint, "diffuse": gmesh.material.diffuse.to_array(),
            "vertices": pos.len() / 3, "indices": idx.len(), "pos": po, "nor": no, "uv": uo, "idx": io,
        }));
    }
    let mut textures: Vec<(String, bool)> =
        meta.iter().filter_map(|m| Some((m["texture"].as_str()?.to_string(), m["plt"].as_bool()?))).collect();
    // The part's particle emitters (a weapon's flames, see crate::emitters): a viewer runs them; their textures are plain images, named as
    // the game names them (`tex/<name>.png`).
    let mut emitter_textures = Vec::new();
    let emitters = crate::emitters::describe(&gm.model, pose, inst.transform, anim, t, &mut |name: &str| {
        Assets::texture(&game.resman, name)?;
        emitter_textures.push((name.to_string(), false));
        Some(name.to_string())
    });
    textures.extend(emitter_textures);
    textures.sort();
    textures.dedup();
    let mut head = serde_json::to_vec(&json!({ "model": gm.model.name, "meshes": meta, "emitters": emitters }))?;
    while head.len() % 4 != 0 {
        head.push(b' ');
    }
    let mut out = Vec::with_capacity(12 + head.len() + bin.len());
    out.extend_from_slice(b"NWPC");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(head.len())?.to_le_bytes());
    out.extend_from_slice(&head);
    out.extend_from_slice(&bin);
    Ok(Chunk { bytes: out, textures })
}

/// A PLT as a two-channel image the viewer colours itself: grey level in red, layer (0 to 9) in green, rows top first (like every
/// other texture here), as PNG.
pub fn plt_png(game: &GameData, name: &str) -> Result<Vec<u8>> {
    let resref = ResRef::cut(name).context("not a texture name")?;
    let data = game.resman.get(&ResKey::new(resref, ResType::PLT)).with_context(|| format!("no PLT {name}"))?;
    let plt = mg_image::plt::Plt::read(&data)?;
    // PLT rows are stored bottom first: flip them to top first.
    let (w, h) = (plt.width as usize, plt.height as usize);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in (0..h).rev() {
        for x in 0..w {
            let [grey, layer] = plt.pixels[y * w + x];
            rgb.extend_from_slice(&[grey, layer.min(9), 0]);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, plt.width, plt.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&rgb)?;
    }
    Ok(out)
}

/// The palettes PLT layers are coloured from: each an image whose rows are as the game stores them (bottom first), so a colour
/// number n is row `height - 1 - n` of the file, and a grey level g is column `g * width / 256`.
pub fn palette_pngs(game: &GameData) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for name in mg_image::plt::PALETTES {
        if !seen.insert(name) {
            continue;
        }
        let (kind, data) = game
            .resman
            .texture(ResRef::from_str(name).map_err(|_| anyhow::anyhow!("bad palette name {name}"))?)
            .with_context(|| format!("no palette {name}"))?;
        let rgba = mg_image::read(kind, &data)?.to_rgba();
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, rgba.width, rgba.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()?.write_image_data(&rgba.data)?;
        }
        out.push((name.to_string(), png_bytes));
    }
    Ok(out)
}

/// A plain texture as PNG, as `bake_creature` writes it (an environment-mapped texture's alpha is made opaque and its colour taken
/// towards grey by it).
pub fn plain_png(game: &GameData, name: &str) -> Result<Vec<u8>> {
    let lt = Assets::texture(&game.resman, name).with_context(|| format!("no texture {name}"))?;
    let mut rgba = lt.texture.to_rgba();
    if lt.txi.envmap().is_some() {
        for px in rgba.data.as_chunks_mut::<4>().0.iter_mut() {
            let a = f32::from(px[3]) / 255.0;
            for c in &mut px[..3] {
                *c = (f32::from(*c) * a + 0.666_667 * 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            }
            px[3] = 255;
        }
    }
    let img = rgba.top_down();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&img.data)?;
    }
    Ok(out)
}

/// Makes everything a viewer needs for the character in `gff_bytes` (its equipped items read from `items`) that `dir` does not have yet,
/// and says what the character is made of.
/// `dir/chunk/<id>` is a part's meshes, `dir/tex/<name>.png` a plain texture, `dir/plt/<name>.png` a PLT, `dir/palette/<name>.png` a palette;
/// every file is written whole (to a temporary name, then renamed) and is never changed after: they are named for what is in them.
pub fn ensure(
    game: &GameData,
    gpu: &mg_render::Gpu,
    gff_bytes: &[u8],
    items: &crate::items::Items,
    dir: &std::path::Path,
    anim: &str,
    t: f32,
) -> Result<Look> {
    use std::fs;
    let look = describe(game, gff_bytes, items, anim, t)?;
    for sub in ["chunk", "tex", "plt", "palette"] {
        fs::create_dir_all(dir.join(sub))?;
    }
    let write = |path: std::path::PathBuf, bytes: &[u8]| -> Result<()> {
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    };
    if fs::read_dir(dir.join("palette"))?.next().is_none() {
        for (name, bytes) in palette_pngs(game)? {
            write(dir.join("palette").join(format!("{name}.png")), &bytes)?;
        }
    }
    for part in std::iter::once((&look.base, None)).chain(look.parts.iter().map(|p| (p, Some(&p.spec)))) {
        let path = dir.join("chunk").join(&part.0.id);
        let chunk = if path.exists() { None } else { Some(bake_part(game, gpu, &look.base.spec, anim, t, part.1)?) };
        let textures = match &chunk {
            Some(c) => c.textures.clone(),
            None => Vec::new(),
        };
        if let Some(c) = chunk {
            // The textures first: a chunk that exists is one whose textures do too.
            for (name, plt) in &textures {
                let file = dir.join(if *plt { "plt" } else { "tex" }).join(format!("{name}.png"));
                if !file.exists() {
                    write(file, &if *plt { plt_png(game, name)? } else { plain_png(game, name)? })?;
                }
            }
            write(path, &c.bytes)?;
        }
    }
    Ok(look)
}
