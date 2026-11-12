// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing: the pictures and the manifest.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::{Background, Depth, IsoOptions, Light, Look, Picture, Rendered, Roof, View};
use crate::area::{Kind, Scene};

/// The manifest's format name (see `docs/area-iso.md`).
pub const FORMAT: &str = "nwn-bake/area-iso";
/// The manifest's version: raised when a field changes in a way a reader must know.
pub const VERSION: u32 = 1;

/// The pictures' version: part of both cache keys (`docs/area-iso.md`, "Keys") in place of the build's commit, so a new build makes
/// no area stale unless it draws differently. **Raise it by one with any change that alters a picture or a manifest** (the look, the
/// scene, the camera, the encoders, a manifest field), then write the fixture's digests again (`docs/testing.md`, "The pictures'
/// version"): the test `pictures_version` fails while the fixture's pictures or manifests differ from the digests recorded for this
/// number. Every area is drawn again after a raise.
pub const PICTURES_VERSION: u32 = 1;

/// The picture files' format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// PNG, lossless.
    Png,
    /// Lossy WebP at [`WEBP_QUALITY`], its alpha kept (built with the `webp` feature).
    Webp,
}

/// The WebP pictures' quality (0..100, libwebp's lossy `quality`; the alpha is kept losslessly).
pub const WEBP_QUALITY: f32 = 85.0;

impl Format {
    /// The file extension: `png` or `webp`.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Webp => "webp",
        }
    }

    /// WebP when this build can write it, else PNG.
    pub fn best() -> Format {
        if cfg!(feature = "webp") { Format::Webp } else { Format::Png }
    }

    /// `png` or `webp` (the latter only when this build can write it).
    pub fn parse(s: &str) -> Result<Format> {
        match s {
            "png" => Ok(Format::Png),
            "webp" if cfg!(feature = "webp") => Ok(Format::Webp),
            "webp" => bail!("this nwn-bake is built without WebP (the `webp` feature); use --format png"),
            _ => bail!("the format is webp or png, not {s:?}"),
        }
    }
}

/// Where and how to write: file names as patterns in which `{area}`, `{view}` and `{ext}` are replaced.
#[derive(Clone, Debug)]
pub struct OutOptions {
    /// The folder the file names are relative to.
    pub dir: PathBuf,
    /// The pictures.
    pub name: String,
    /// The depth pictures (always PNG).
    pub depth_name: String,
    /// Each area's manifest.
    pub manifest_name: String,
    /// The pictures' format.
    pub format: Format,
}

impl OutOptions {
    /// Everything side by side in `dir`, PNG: `{area}.{view}.{ext}`, `{area}.{view}.depth.png`, `{area}.json`.
    pub fn new(dir: impl Into<PathBuf>) -> OutOptions {
        OutOptions {
            dir: dir.into(),
            name: "{area}.{view}.{ext}".into(),
            depth_name: "{area}.{view}.depth.png".into(),
            manifest_name: "{area}.json".into(),
            format: Format::Png,
        }
    }

    /// A folder per kind of file and per view, as a batch writes it: `{ext}/{view}/{area}.{ext}`, `depth/{view}/{area}.png`,
    /// `json/{area}.json` (with `manifest.jsonl` and `index.json` beside them, [`batch`](super::batch)).
    pub fn tree(dir: impl Into<PathBuf>, format: Format) -> OutOptions {
        OutOptions {
            dir: dir.into(),
            name: "{ext}/{view}/{area}.{ext}".into(),
            depth_name: "depth/{view}/{area}.png".into(),
            manifest_name: "json/{area}.json".into(),
            format,
        }
    }

    fn fill(pattern: &str, area: &str, view: Option<View>, ext: &str) -> String {
        pattern.replace("{area}", area).replace("{view}", view.map_or("", View::name)).replace("{ext}", ext)
    }

    /// A view's picture's file name, relative to `dir`.
    pub fn picture(&self, area: &str, view: View) -> String {
        Self::fill(&self.name, area, Some(view), self.format.ext())
    }

    /// A view's depth picture's file name, relative to `dir`.
    pub fn depth(&self, area: &str, view: View) -> String {
        Self::fill(&self.depth_name, area, Some(view), "png")
    }

    /// The area's manifest's file name, relative to `dir`.
    pub fn manifest(&self, area: &str) -> String {
        Self::fill(&self.manifest_name, area, None, "json")
    }
}

/// What the scene was made from, for the manifest.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Inputs {
    /// `sha256:` of the `.are` as binary GFF, hex (so a `.mod` and its nasher JSON give the same).
    pub are: String,
    /// `sha256:` of the `.git` as binary GFF; None when the area has none.
    pub git: Option<String>,
    /// The haks the game was opened with, first highest.
    pub haks: Vec<String>,
    /// The haks asked for that were found nowhere.
    pub missing_haks: Vec<String>,
    /// The custom talk table asked for.
    pub tlk: Option<String>,
}

impl Inputs {
    /// The inputs of `area`: the digests of its `.are` and `.git`, with the haks, the haks not found and the talk table as given.
    /// Fails when a GFF can't be written back as binary.
    pub fn of(area: &crate::module::AreaFiles, haks: &[String], missing_haks: &[String], tlk: Option<&str>) -> Result<Inputs> {
        let digest = |g: &mg_gff::Gff| -> Result<String> {
            use sha2::Digest;
            let bytes = g.to_bytes().map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(format!("sha256:{:x}", sha2::Sha256::digest(&bytes)))
        };
        Ok(Inputs {
            are: digest(&area.are).with_context(|| format!("{}.are", area.resref))?,
            git: area.git.as_ref().map(digest).transpose().with_context(|| format!("{}.git", area.resref))?,
            haks: haks.to_vec(),
            missing_haks: missing_haks.to_vec(),
            tlk: tlk.map(str::to_string),
        })
    }
}

/// The manifest (`<area>.json`), version 1. See `docs/area-iso.md`.
#[derive(Clone, Debug, Serialize)]
pub struct Manifest {
    /// Always [`FORMAT`].
    pub format: &'static str,
    /// Always [`VERSION`].
    pub version: u32,
    /// What drew the pictures.
    pub renderer: Renderer,
    /// The area: its names, size and kind.
    pub area: AreaInfo,
    /// What the scene was made from.
    pub inputs: Inputs,
    /// The digest of every resource the scene read ([`crate::game::Game::resources_digest`]). Null when the pictures were drawn
    /// without keys.
    pub resources: Option<String>,
    /// The exact key ([`batch`](super::batch); `docs/area-iso.md`, "Keys"): the part both keys share (the renderer, the options, the
    /// area and its inputs), the area's name and `resources`. Null when the pictures were drawn without keys.
    pub key: Option<String>,
    /// The quick key ([`batch`](super::batch); `docs/area-iso.md`, "Keys"): the part both keys share and what the game's files are.
    /// Null when the run made no quick keys (named areas with no cache).
    pub quick_key: Option<String>,
    /// The options in effect.
    pub options: OptionsInfo,
    /// Where the roof was cut (metres), if it was.
    pub roof_cut_m: Option<f32>,
    /// Each view drawn, in the order asked for.
    pub views: Vec<ViewInfo>,
    /// What could not be read or looked wrong: the scene's notes, the haks not found, then the pictures' notes.
    pub notes: Vec<String>,
}

/// What drew the pictures. Only `name` and `pictures` are part of the keys; `version` (the crate's) and `commit` (the build's) are
/// there for information.
#[derive(Clone, Debug, Serialize)]
pub struct Renderer {
    /// Always `nwn-bake`.
    pub name: &'static str,
    /// The crate's version, from `Cargo.toml`.
    pub version: &'static str,
    /// The git commit it was built from (`NWN_BAKE_COMMIT` at build time, as release builds set it); None for a local build.
    pub commit: Option<&'static str>,
    /// [`PICTURES_VERSION`].
    pub pictures: u32,
}

impl Renderer {
    /// This program.
    pub fn this() -> Renderer {
        Renderer {
            name: "nwn-bake",
            version: env!("CARGO_PKG_VERSION"),
            commit: option_env!("NWN_BAKE_COMMIT").filter(|c| !c.is_empty()),
            pictures: PICTURES_VERSION,
        }
    }
}

/// The area, as the manifest describes it.
#[derive(Clone, Debug, Serialize)]
pub struct AreaInfo {
    /// Its resref, lower case.
    pub resref: String,
    /// As the game shows it (None when the area has no name).
    pub name: Option<String>,
    /// Its tileset's resref, lower case.
    pub tileset: String,
    /// Tiles along x (east) and y (north).
    pub tiles: [u32; 2],
    /// Its size in metres along x and y: the tiles times 10 m.
    pub size_m: [f32; 2],
    /// Surface, interior or underground ([`Scene::kind`]): whether the roof is cut.
    pub kind: Kind,
}

/// The options as they were in effect.
#[derive(Clone, Debug, Serialize)]
pub struct OptionsInfo {
    /// [`IsoOptions::scale`].
    pub scale: f32,
    /// [`IsoOptions::quality`].
    pub quality: u8,
    /// [`IsoOptions::light`].
    pub light: Light,
    /// [`IsoOptions::shadows`].
    pub shadows: bool,
    /// `"auto"`, `"keep"` or metres over the floor.
    pub roof: serde_json::Value,
    /// `"auto"` or `"off"`.
    pub exposure: &'static str,
    /// Whether the `.git`'s placeables and doors are drawn ([`crate::area::SceneOptions::objects`]).
    pub objects: bool,
    /// `"transparent"` or `"#rrggbb"`.
    pub background: String,
    /// `"default"`, or the whole [`Look`] when it is not the default.
    pub look: serde_json::Value,
    /// The pictures' format: `"webp"` or `"png"`, and the WebP's quality.
    pub format: &'static str,
    /// The WebP quality ([`WEBP_QUALITY`]); left out for PNG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webp_quality: Option<f32>,
    /// Whether there are depth pictures.
    pub depth: bool,
}

impl OptionsInfo {
    /// The options in effect for pictures drawn with `o`, of a scene with or without its objects, written as `format`.
    pub fn of(o: &IsoOptions, objects: bool, format: Format) -> OptionsInfo {
        OptionsInfo {
            scale: o.scale,
            quality: o.quality,
            light: o.light,
            shadows: o.shadows,
            roof: match o.roof {
                Roof::Auto => "auto".into(),
                Roof::Keep => "keep".into(),
                Roof::Over(m) => shortest(&m),
            },
            exposure: if o.exposure { "auto" } else { "off" },
            objects,
            background: match o.background {
                Background::Transparent => "transparent".into(),
                Background::Colour(c) => format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]),
            },
            look: if o.look == Look::default() { "default".into() } else { shortest(&o.look) },
            format: format.ext(),
            webp_quality: (format == Format::Webp).then_some(WEBP_QUALITY),
            depth: o.depth,
        }
    }
}

/// `value` as JSON with its `f32`s in their shortest form (0.35, not the 0.3499999940395355 that `serde_json::to_value` widens
/// it to): through text, which writes an `f32` as `f32`.
fn shortest(value: &impl Serialize) -> serde_json::Value {
    serde_json::to_string(value).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// One view in the manifest: its file and its transform.
#[derive(Clone, Debug, Serialize)]
pub struct ViewInfo {
    /// The view.
    pub view: View,
    /// The picture's file name, relative to the output folder.
    pub file: String,
    /// The depth picture's file name ([`Depth`]); None without depth pictures.
    pub depth: Option<String>,
    /// The heights of the depth picture's 0 and 65535 ([`Depth::range`]); left out without a depth picture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_range: Option<[f32; 2]>,
    /// The picture file's length in bytes; None until it is written.
    pub bytes: Option<u64>,
    /// [`Transform::width`](super::Transform::width).
    pub width: u32,
    /// [`Transform::height`](super::Transform::height).
    pub height: u32,
    /// [`Transform::scale`](super::Transform::scale).
    pub scale: f32,
    /// [`Transform::elevation_deg`](super::Transform::elevation_deg).
    pub elevation_deg: f32,
    /// [`Transform::rotation`](super::Transform::rotation).
    pub rotation: [[i32; 2]; 2],
    /// [`Transform::offset`](super::Transform::offset).
    pub offset: [f32; 2],
    /// [`Transform::matrix`](super::Transform::matrix).
    pub matrix: [[f32; 4]; 2],
}

/// The manifest for `rendered` (nothing written).
pub fn manifest(scene: &Scene, name: Option<String>, inputs: Inputs, rendered: &Rendered, o: &IsoOptions, out: &OutOptions) -> Manifest {
    let mut notes = scene.notes.clone();
    notes.extend(inputs.missing_haks.iter().map(|h| format!("hak {h} not found")));
    notes.extend(rendered.notes.iter().cloned());
    Manifest {
        format: FORMAT,
        version: VERSION,
        renderer: Renderer::this(),
        area: AreaInfo {
            resref: scene.resref.clone(),
            name,
            tileset: scene.tileset.clone(),
            tiles: scene.tiles,
            size_m: scene.size(),
            kind: scene.kind(),
        },
        inputs,
        resources: None,
        key: None,
        quick_key: None,
        options: OptionsInfo::of(o, scene.objects, out.format),
        roof_cut_m: rendered.roof_cut.map(|c| c.at),
        views: rendered
            .pictures
            .iter()
            .map(|p| {
                let t = &p.transform;
                ViewInfo {
                    view: p.view,
                    file: out.picture(&scene.resref, p.view),
                    depth: p.depth.as_ref().map(|_| out.depth(&scene.resref, p.view)),
                    depth_range: p.depth.as_ref().map(|d| d.range),
                    bytes: None,
                    width: p.width,
                    height: p.height,
                    scale: t.scale,
                    elevation_deg: t.elevation_deg,
                    rotation: t.rotation,
                    offset: t.offset,
                    matrix: t.matrix,
                }
            })
            .collect(),
        notes,
    }
}

/// A picture's file name in `out.dir`.
pub fn file_name(out: &OutOptions, area: &str, view: View) -> String {
    out.picture(area, view)
}

/// Writes the pictures (and depth pictures) and the manifest to `out.dir`, each file whole or not at all; returns the manifest.
pub fn write(
    scene: &Scene,
    name: Option<String>,
    inputs: Inputs,
    rendered: &Rendered,
    o: &IsoOptions,
    out: &OutOptions,
) -> Result<Manifest> {
    let mut m = manifest(scene, name, inputs, rendered, o, out);
    let opaque = !matches!(o.background, Background::Transparent);
    for (p, v) in rendered.pictures.iter().zip(m.views.iter_mut()) {
        let bytes = encode(p, out.format, opaque)?;
        v.bytes = Some(bytes.len() as u64);
        write_file(&out.dir.join(&v.file), &bytes)?;
        if let (Some(d), Some(file)) = (&p.depth, &v.depth) {
            write_file(&out.dir.join(file), &encode_depth(d, p.width, p.height)?)?;
        }
    }
    write_file(&out.dir.join(out.manifest(&scene.resref)), &serde_json::to_vec_pretty(&m)?)?;
    Ok(m)
}

/// Writes `bytes` to `path` (its folders made), through a temporary file beside it, so the file is never seen half written.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    let tmp = path.with_file_name(format!(".{}.part", path.file_name().map_or_default(|n| n.to_string_lossy())));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

/// A picture as a file's bytes in `format`: PNG (RGB when `opaque`, else RGBA) or lossy WebP at [`WEBP_QUALITY`] with its alpha.
pub fn encode(p: &Picture, format: Format, opaque: bool) -> Result<Vec<u8>> {
    match format {
        Format::Png => {
            let mut out = Vec::new();
            {
                let mut enc = png::Encoder::new(&mut out, p.width, p.height);
                enc.set_depth(png::BitDepth::Eight);
                if opaque {
                    enc.set_color(png::ColorType::Rgb);
                    let rgb: Vec<u8> = p.rgba.as_chunks::<4>().0.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
                    enc.write_header()?.write_image_data(&rgb)?;
                } else {
                    enc.set_color(png::ColorType::Rgba);
                    enc.write_header()?.write_image_data(&p.rgba)?;
                }
            }
            Ok(out)
        }
        Format::Webp => webp(p),
    }
}

#[cfg(feature = "webp")]
fn webp(p: &Picture) -> Result<Vec<u8>> {
    // (libwebp's defaults otherwise, as Pillow's: method 4, the alpha lossless; one thread, so the same picture gives the same file.)
    let enc = webp::Encoder::from_rgba(&p.rgba, p.width, p.height);
    let mut config = webp::WebPConfig::new().map_err(|()| anyhow::anyhow!("libwebp: no configuration"))?;
    config.quality = WEBP_QUALITY;
    config.method = 4;
    config.alpha_quality = 100;
    let data = enc.encode_advanced(&config).map_err(|e| anyhow::anyhow!("libwebp: {e:?}"))?;
    Ok(data.to_vec())
}

#[cfg(not(feature = "webp"))]
fn webp(_: &Picture) -> Result<Vec<u8>> {
    bail!("this nwn-bake is built without WebP (the `webp` feature)")
}

/// A depth picture as a 16-bit grey PNG.
pub fn encode_depth(d: &Depth, width: u32, height: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Sixteen);
        let bytes: Vec<u8> = d.values.iter().flat_map(|v| v.to_be_bytes()).collect();
        enc.write_header()?.write_image_data(&bytes)?;
    }
    Ok(out)
}

/// A picture as PNG: RGB when it is opaque, else RGBA.
pub fn write_png(path: &Path, p: &Picture, opaque: bool) -> Result<()> {
    write_file(path, &encode(p, Format::Png, opaque)?)
}

/// `#rrggbb` or `transparent`.
pub fn parse_background(s: &str) -> Result<Background> {
    if s == "transparent" {
        return Ok(Background::Transparent);
    }
    let hex = s.strip_prefix('#').unwrap_or(s);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("--background is transparent or #rrggbb, not {s:?}");
    }
    let v = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    Ok(Background::Colour([v(0), v(2), v(4)]))
}
