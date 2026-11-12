// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `nwn-bake` on the command line: one command per subject, parsed by clap. `nwn-bake --help` lists the commands and
//! `nwn-bake <command> --help` gives each one's arguments and options; the README's section for each command and `docs/` say more.
//!
//! The rules every command shares:
//!
//! - **The game:** `--root` (else `NWN_ROOT`, then the usual places) and the user folder, `--user`, else `NWN_BAKE_USER`, else
//!   `NWN_HOME` (else the game's own rule), resolved in one place, [`GameArgs::user`]. The custom talk table is `--tlk`, else
//!   `NWN_BAKE_TLK`, else the module's.
//! - **Areas:** `<module> <area>...`, `<module> --all`, or one area file on its own ([`AreaArgs`]), the same for `area-iso`,
//!   `area-3d` and `area-scene`.
//! - **Exit codes:** 0 when all went well; 1 on an error; 2 on misuse (an unknown command or option, a value that does not parse),
//!   found before the game is opened; 3 when a batch ran and an area in it failed.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use nwn_bake::game::{Game, GameOptions, Haks, Tlk};
use nwn_bake::look;

/// `eprintln!` that drops the line when stderr cannot be written ([`warn`]).
macro_rules! note {
    ($($t:tt)*) => {
        $crate::warn(format_args!($($t)*))
    };
}

mod area3d;

/// Misuse: an unknown command or option, a missing argument, a value that does not parse.
const EXIT_MISUSE: i32 = 2;
/// A batch (`area-iso`, `area-3d`) in which an area failed; the others were done.
const EXIT_AREA_FAILED: i32 = 3;

/// Bakes Neverwinter Nights: EE subjects (characters, items, placeables, areas) into files a web page can show.
#[derive(Parser)]
#[command(
    name = "nwn-bake",
    disable_help_subcommand = true,
    after_help = "Run `nwn-bake <COMMAND> --help` for a command's arguments and options.\n\n\
                  Environment: NWN_ROOT (the install), NWN_BAKE_USER and then NWN_HOME (the user folder), NWN_BAKE_TLK (the talk \
                  table), NWN_BAKE_JOBS (threads for a batch), NWN_BAKE_SOFTWARE=1 (no GPU: Mesa lavapipe).\n\
                  Exit codes: 0 done, 1 an error, 2 misuse, 3 a batch in which an area failed."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

/// Where the game is.
#[derive(Args, Clone, Debug)]
struct GameArgs {
    /// The NWN:EE install, the folder with data/ and lang/ [default: NWN_ROOT, then where the game is usually installed]
    #[arg(long, value_name = "DIR")]
    root: Option<PathBuf>,
    /// The user folder (hak/, tlk/, override/) [default: NWN_BAKE_USER, then NWN_HOME, then the game's own]
    #[arg(long, value_name = "DIR")]
    user: Option<PathBuf>,
}

/// A non-empty environment variable.
fn env(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

impl GameArgs {
    /// The user folder: `--user`, else `NWN_BAKE_USER`, else `NWN_HOME`; None leaves it to the game's rule (`NWN_USER_DIRECTORY`,
    /// then the platform's default). A folder named but not there is an error that says where the name came from.
    fn user(&self) -> Result<Option<PathBuf>> {
        let (path, from) = match (&self.user, env("NWN_BAKE_USER"), env("NWN_HOME")) {
            (Some(p), _, _) => (p.clone(), "--user"),
            (None, Some(p), _) => (PathBuf::from(p), "NWN_BAKE_USER"),
            (None, None, Some(p)) => (PathBuf::from(p), "NWN_HOME"),
            (None, None, None) => return Ok(None),
        };
        anyhow::ensure!(path.is_dir(), "{}: no such folder (the user folder, from {from})", path.display());
        Ok(Some(path))
    }
}

/// The custom talk table.
#[derive(Args, Clone, Debug)]
struct TlkArg {
    /// The custom talk table: a file name in the user folder's tlk/ or hak/ (.tlk may be left out) or a path; not finding it is an
    /// error [default: NWN_BAKE_TLK, then the module's (none when it is not there), then the one .tlk in the user folder]
    #[arg(long, value_name = "NAME|PATH")]
    tlk: Option<String>,
}

impl TlkArg {
    /// `--tlk`, else `NWN_BAKE_TLK`.
    fn given(&self) -> Option<String> {
        self.tlk.clone().or_else(|| std::env::var("NWN_BAKE_TLK").ok()).filter(|t| !t.trim().is_empty())
    }
}

/// The game, for the commands that read characters, items, placeables and textures.
#[derive(Args, Clone, Debug)]
struct CharGame {
    #[command(flatten)]
    game: GameArgs,
    #[command(flatten)]
    tlk: TlkArg,
    /// Open the game with this module's haks, in its order, and its talk table, instead of every .hak in the user folder's hak/
    /// (a .mod, a module folder, a nasher project, a module.ifo or an area file)
    #[arg(long, value_name = "MODULE")]
    module: Option<PathBuf>,
}

/// How a character stands.
#[derive(Args, Clone, Debug)]
struct Pose {
    /// The animation the character is posed in
    #[arg(long, value_name = "NAME", default_value = "pause1")]
    anim: String,
    /// How far into the animation, in seconds
    #[arg(long = "t", value_name = "SECONDS", default_value_t = 0.0)]
    t: f32,
}

/// Where a creature blueprint's equipped items are found.
#[derive(Args, Clone, Debug)]
struct ItemArgs {
    /// A folder of item blueprints (<resref>.uti or nwn_gff's <resref>.uti.json, subfolders too) looked in before the user's
    /// override and haks and the game, for the items a .utc names by resref (a module's own items, which no hak has)
    #[arg(long, value_name = "DIR")]
    items: Option<PathBuf>,
    /// An equipped item not found is an error, not a warning
    #[arg(long)]
    strict_items: bool,
}

impl ItemArgs {
    fn open(&self) -> Result<nwn_bake::items::Items> {
        item_lookup(self.items.as_deref(), self.strict_items)
    }
}

/// The item lookup: `dir`'s blueprints, then the game's; `strict`, an item not found is an error.
fn item_lookup(dir: Option<&Path>, strict: bool) -> Result<nwn_bake::items::Items> {
    let items = match dir {
        Some(d) => nwn_bake::items::Items::folder(d)?,
        None => nwn_bake::items::Items::game(),
    };
    Ok(items.strict(strict))
}

/// A warning a line for each equipped item that was not found.
fn warn_missing_items(missing: &[nwn_bake::items::MissingItem]) {
    for m in missing {
        note!("warning: equipped item not found, drawn without it: {m}");
    }
}

/// The areas a command works on: the same rules for `area-iso`, `area-3d` and `area-scene`.
#[derive(Args, Clone, Debug)]
struct AreaArgs {
    /// The module (a .mod, a module folder, a nasher project, a module.ifo, or an area file for the nearest module.ifo above it),
    /// or one area file on its own (x.are, x.are.json, x.git or x.git.json) for that area
    #[arg(value_name = "MODULE")]
    module: PathBuf,
    /// The areas, by resref
    #[arg(value_name = "AREA")]
    areas: Vec<String>,
    /// Every area of the module, instead of naming them
    #[arg(long, conflicts_with = "areas")]
    all: bool,
    /// A nasher project's areas from this target, instead of the one nasher packs by default
    #[arg(long, value_name = "NAME")]
    target: Option<String>,
}

impl AreaArgs {
    /// The module and the areas to work on, each once (the first spelling kept): those named, every area with `--all`, or the area
    /// of a lone area file. Naming none is misuse of `command`.
    fn resolve(&self, command: &str) -> Result<(nwn_bake::module::ModuleSource, Vec<String>)> {
        let (module, named) = match (self.areas.is_empty(), self.all) {
            (false, _) | (true, true) => (self.module.clone(), self.areas.clone()),
            (true, false) => match area_file(&self.module) {
                Some((are, resref)) => (are, vec![resref]),
                None => misuse(command, "name the areas, give --all, or give one area file on its own"),
            },
        };
        let source = nwn_bake::module::ModuleSource::open_target(&module, self.target.as_deref())
            .with_context(|| format!("opening the module {}", module.display()))?;
        for w in source.warnings() {
            note!("module: {w}");
        }
        let mut areas = if self.all { source.areas() } else { named };
        let mut seen = std::collections::HashSet::new();
        areas.retain(|a| seen.insert(a.to_ascii_lowercase()));
        anyhow::ensure!(!areas.is_empty(), "{}: the module has no areas", module.display());
        Ok((source, areas))
    }
}

impl AreaArgs {
    /// The module and a draft's template (`area-3d --draft`): the one area named, or the lone area file's; no area when the module
    /// is given alone (a draft without a template). Naming more than one is misuse of `command`.
    fn resolve_template(&self, command: &str) -> Result<(nwn_bake::module::ModuleSource, Vec<String>)> {
        if self.areas.len() > 1 {
            misuse(command, "--draft takes one template area, or none");
        }
        if !self.areas.is_empty() || area_file(&self.module).is_some() {
            return self.resolve(command);
        }
        let source = nwn_bake::module::ModuleSource::open_target(&self.module, self.target.as_deref())
            .with_context(|| format!("opening the module {}", self.module.display()))?;
        for w in source.warnings() {
            note!("module: {w}");
        }
        Ok((source, Vec::new()))
    }
}

/// An area file given on its own: the module to open (the `.are` beside a `.git`) and the area's resref. None when `p` is not an
/// area file that is there.
fn area_file(p: &Path) -> Option<(PathBuf, String)> {
    let name = p.file_name()?.to_string_lossy().to_ascii_lowercase();
    let stem = name.strip_suffix(".json").unwrap_or(&name);
    if let Some(r) = stem.strip_suffix(".are") {
        return p.is_file().then(|| (p.to_path_buf(), r.to_string()));
    }
    let r = stem.strip_suffix(".git")?;
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "`name` is lower case already")]
    let json = name.ends_with(".json");
    let are = p.with_file_name(format!("{}{}", &p.file_name()?.to_string_lossy()[..r.len()], if json { ".are.json" } else { ".are" }));
    are.is_file().then(|| (are, r.to_string()))
}

/// `--haks a,b,c`: hak names, first highest.
#[derive(Clone, Debug)]
struct HakList(Vec<String>);

fn parse_haks(v: &str) -> Result<HakList, String> {
    Ok(HakList(v.split(',').map(str::trim).filter(|h| !h.is_empty()).map(str::to_string).collect()))
}

fn parse_on_off(v: &str) -> Result<bool, String> {
    match v {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("on or off".into()),
    }
}

fn parse_jobs(v: &str) -> Result<usize, String> {
    v.parse::<usize>().ok().filter(|&n| n > 0).ok_or_else(|| "a number of threads, 1 or more".into())
}

/// `--jobs`, else `NWN_BAKE_JOBS` (an error when it is not a number, 1 or more), else half the cores.
fn jobs(given: Option<usize>) -> Result<usize> {
    if let Some(n) = given {
        return Ok(n);
    }
    match std::env::var("NWN_BAKE_JOBS").ok().filter(|v| !v.trim().is_empty()) {
        Some(v) => parse_jobs(v.trim()).map_err(|e| anyhow::anyhow!("NWN_BAKE_JOBS={v}: not {e}")),
        None => Ok(nwn_bake::iso::batch::default_jobs()),
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// A character as one scene: scene.json and tex/*.png in OUT_DIR
    Creature {
        /// The character (.bic) or creature blueprint (.utc, or nwn_gff's .utc.json)
        #[arg(value_name = "FILE")]
        input: PathBuf,
        out_dir: PathBuf,
        #[command(flatten)]
        pose: Pose,
        #[command(flatten)]
        items: ItemArgs,
        #[command(flatten)]
        game: CharGame,
    },
    /// A character as a list of parts (JSON on stdout)
    Describe {
        /// The character (.bic) or creature blueprint (.utc, or nwn_gff's .utc.json)
        #[arg(value_name = "FILE")]
        input: PathBuf,
        #[command(flatten)]
        pose: Pose,
        #[command(flatten)]
        items: ItemArgs,
        #[command(flatten)]
        game: CharGame,
    },
    /// `describe`, and writes the parts, textures and palettes PARTS_DIR does not have yet
    Look {
        /// The character (.bic) or creature blueprint (.utc, or nwn_gff's .utc.json)
        #[arg(value_name = "FILE")]
        input: PathBuf,
        parts_dir: PathBuf,
        #[command(flatten)]
        pose: Pose,
        #[command(flatten)]
        items: ItemArgs,
        #[command(flatten)]
        game: CharGame,
    },
    /// One part's meshes, from a spec file: {"anim", "t", "base", "part" (or null)}, each part as describe prints it
    Part {
        spec: PathBuf,
        out: PathBuf,
        #[command(flatten)]
        game: CharGame,
    },
    /// A picture of a creature
    Thumb {
        /// The creature blueprint (.utc, or nwn_gff's .utc.json) or character (.bic)
        #[arg(value_name = "FILE")]
        input: PathBuf,
        /// The PNG to write
        out: PathBuf,
        /// Its side, in pixels
        #[arg(default_value_t = 256)]
        size: u32,
        /// The turn of the camera around the creature, in degrees
        #[arg(default_value_t = 60.0, allow_negative_numbers = true)]
        yaw: f32,
        #[command(flatten)]
        pose: Pose,
        #[command(flatten)]
        items: ItemArgs,
        #[command(flatten)]
        game: CharGame,
    },
    /// A picture of an item on its own
    ItemThumb {
        /// The item blueprint (.uti, or nwn_gff's .uti.json)
        #[arg(value_name = "FILE")]
        input: PathBuf,
        /// The PNG to write
        out: PathBuf,
        /// Its width, in pixels
        #[arg(default_value_t = 256)]
        width: u32,
        /// Its height, in pixels [default: the width]
        height: Option<u32>,
        /// The turn of the camera around the item, in degrees
        #[arg(default_value_t = 0.0, allow_negative_numbers = true)]
        yaw: f32,
        #[command(flatten)]
        game: CharGame,
    },
    /// A picture of a placeables.2da row, textures replaced
    PlaceableThumb {
        /// The placeables.2da row
        row: u32,
        /// The PNG to write
        out: PathBuf,
        /// Its width, in pixels
        #[arg(default_value_t = 256)]
        width: u32,
        /// Its height, in pixels [default: the width]
        height: Option<u32>,
        /// The turn of the camera around the placeable, in degrees
        #[arg(default_value_t = 0.0, allow_negative_numbers = true)]
        yaw: f32,
        /// A texture replaced by another; any number of times
        #[arg(long = "tex", value_name = "OLD=NEW", value_parser = parse_tex)]
        tex: Vec<(String, String)>,
        #[command(flatten)]
        game: CharGame,
    },
    /// Every placeables.2da row with a model: where it comes from, its size, particles, light and textures (JSON)
    Placeables {
        /// The JSON file to write
        out: PathBuf,
        #[command(flatten)]
        game: CharGame,
    },
    /// The textures the model of a placeables.2da row draws with (JSON on stdout)
    Textures {
        /// The placeables.2da row
        row: usize,
        #[command(flatten)]
        game: CharGame,
    },
    /// Whether a texture is a PLT or a plain image, and which image (JSON on stdout)
    Kind {
        texture: String,
        /// 1 when the texture is drawn coloured (a PLT is wanted), else 0
        #[arg(value_parser = ["0", "1"])]
        colored: String,
        #[command(flatten)]
        game: CharGame,
    },
    /// A PLT texture as PNG
    Plt {
        texture: String,
        out: PathBuf,
        #[command(flatten)]
        game: CharGame,
    },
    /// A plain texture as PNG
    Plain {
        texture: String,
        out: PathBuf,
        #[command(flatten)]
        game: CharGame,
    },
    /// The PLT palettes as PNG, one file each in OUT_DIR
    Palettes {
        out_dir: PathBuf,
        #[command(flatten)]
        game: CharGame,
    },
    /// The names the game's item-appearance lists are made from (icons and weapon part models), as JSON
    Resources {
        /// The JSON file to write
        out: PathBuf,
        #[command(flatten)]
        game: CharGame,
    },
    /// Several placeables, models, creatures and items in one picture, at a free camera, from a spec file (docs/scene.md); prints
    /// the camera, each object's bounds and what it left out, and the points' pixel positions (JSON)
    Scene {
        /// The spec (JSON, docs/scene.schema.json), or - for stdin
        spec: PathBuf,
        /// The PNG to write
        out: PathBuf,
        #[command(flatten)]
        items: ItemArgs,
        #[command(flatten)]
        game: CharGame,
    },
    /// A baker that stays: one JSON request a line on stdin, one answer a line on stdout (docs/serve.md)
    Serve {
        #[command(flatten)]
        game: CharGame,
    },
    /// An area's scene, as the pictures are drawn from it: what is in it, timings, the floor, the roof cut and its digest
    AreaScene(AreaSceneArgs),
    /// Pictures of areas, one per view, and each area's manifest (docs/area-iso.md)
    AreaIso(Box<AreaIsoArgs>),
    /// Areas as glTF (.glb) for the web, with a manifest each and their textures in one folder (docs/area-3d.md)
    #[command(name = "area-3d")]
    Area3d(Box<area3d::Area3dArgs>),
}

fn parse_tex(v: &str) -> Result<(String, String), String> {
    let (old, new) = v.split_once('=').ok_or("OLD=NEW")?;
    Ok((old.to_string(), new.to_string()))
}

/// `area-scene`'s arguments.
#[derive(Args)]
struct AreaSceneArgs {
    /// The module (as for area-iso) or one area file on its own
    #[arg(value_name = "MODULE")]
    module: PathBuf,
    /// The area, by resref (left out for an area file on its own)
    #[arg(value_name = "AREA")]
    area: Option<String>,
    /// A nasher project's areas from this target, instead of the one nasher packs by default
    #[arg(long, value_name = "NAME")]
    target: Option<String>,
    /// These haks, first highest, instead of the module's
    #[arg(long, value_name = "A,B,C", value_parser = parse_haks)]
    haks: Option<HakList>,
    /// The .git's placeables and doors
    #[arg(long, value_name = "on|off", default_value = "on", value_parser = parse_on_off, action = clap::ArgAction::Set)]
    objects: bool,
    #[command(flatten)]
    game: GameArgs,
}

/// `area-iso`'s arguments.
#[derive(Args)]
struct AreaIsoArgs {
    #[command(flatten)]
    areas: AreaArgs,
    /// The output folder
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
    /// The views: sw, se, ne, nw (from a corner, 30 degrees up), top (straight down, north up), all (the four corners), or a list
    /// (sw,top)
    #[arg(long, visible_alias = "views", value_name = "VIEWS", default_value = "sw", value_parser = parse_views)]
    view: Views,
    /// Pixels a metre
    #[arg(long, value_name = "PIXELS", default_value_t = nwn_bake::iso::IsoOptions::default().scale, value_parser = parse_scale)]
    scale: f32,
    /// Supersampling: each pixel drawn N by N times and averaged, 1 to 4
    #[arg(long, value_name = "N", default_value_t = nwn_bake::iso::IsoOptions::default().quality,
          value_parser = clap::value_parser!(u8).range(1..=4))]
    supersample: u8,
    /// The pictures' format: webp (lossy, quality 85, alpha kept) or png [default: webp when this build can write it]
    #[arg(long, value_name = "webp|png", value_parser = parse_format)]
    format: Option<nwn_bake::iso::Format>,
    /// The light: day (plain daylight) or area (the area's own sun colours)
    #[arg(long, value_name = "day|area", default_value = "day", value_parser = parse_light)]
    light: nwn_bake::iso::Light,
    /// Shadows from the sun
    #[arg(long, value_name = "on|off", default_value = "on", value_parser = parse_on_off, action = clap::ArgAction::Set)]
    shadows: bool,
    /// The roof: auto (interiors and underground areas cut open over their floor), keep, or metres over the floor
    #[arg(long, value_name = "auto|keep|METRES", default_value = "auto", value_parser = parse_roof, allow_negative_numbers = true)]
    roof: nwn_bake::iso::Roof,
    /// Dark areas lifted (auto) or not (off)
    #[arg(long, value_name = "auto|off", default_value = "auto", value_parser = parse_exposure, action = clap::ArgAction::Set)]
    exposure: bool,
    /// The .git's placeables and doors
    #[arg(long, value_name = "on|off", default_value = "on", value_parser = parse_on_off, action = clap::ArgAction::Set)]
    objects: bool,
    /// What is behind the drawing: transparent or a colour, #rrggbb
    #[arg(long, value_name = "transparent|#rrggbb", value_parser = parse_background)]
    background: Option<nwn_bake::iso::Background>,
    /// Also a 16-bit height picture per view
    #[arg(long)]
    depth: bool,
    /// flat (files side by side) or tree (webp/<view>/<area>.webp and json/<area>.json) [default: tree with --all, else flat]
    #[arg(long, value_name = "flat|tree", value_parser = parse_layout)]
    layout: Option<bool>,
    /// File names for the flat layout; {area}, {view} and {ext} are replaced [default: {area}.{view}.{ext}]
    #[arg(long, value_name = "PATTERN")]
    name: Option<String>,
    /// Areas drawn at a time [default: NWN_BAKE_JOBS, then half the cores]
    #[arg(long, value_name = "N", value_parser = parse_jobs)]
    jobs: Option<usize>,
    /// Keep every key's pictures here, so a change undone costs nothing
    #[arg(long, value_name = "DIR")]
    cache: Option<PathBuf>,
    /// Draw even what the keys say is there
    #[arg(long)]
    force: bool,
    /// Calibration constants (nwn_bake::iso::Look), as JSON; written into the manifest and the keys
    #[arg(long, value_name = "FILE")]
    look: Option<PathBuf>,
    /// These haks, first highest, instead of the module's
    #[arg(long, value_name = "A,B,C", value_parser = parse_haks)]
    haks: Option<HakList>,
    /// A hak of the module's list that is not found fails the area
    #[arg(long)]
    strict: bool,
    #[command(flatten)]
    game: GameArgs,
    #[command(flatten)]
    tlk: TlkArg,
}

/// `--view`: the views, each once, in order.
#[derive(Clone, Debug)]
struct Views(Vec<nwn_bake::iso::View>);

fn parse_views(v: &str) -> Result<Views, String> {
    nwn_bake::iso::View::parse_list(v).map(Views).map_err(|e| format!("{e:#}"))
}

fn parse_scale(v: &str) -> Result<f32, String> {
    v.parse().ok().filter(|s: &f32| s.is_finite() && *s > 0.0).ok_or_else(|| "pixels a metre, more than 0".into())
}

fn parse_format(v: &str) -> Result<nwn_bake::iso::Format, String> {
    nwn_bake::iso::Format::parse(v).map_err(|e| format!("{e:#}"))
}

fn parse_light(v: &str) -> Result<nwn_bake::iso::Light, String> {
    match v {
        "day" => Ok(nwn_bake::iso::Light::Day),
        "area" => Ok(nwn_bake::iso::Light::Area),
        _ => Err("day or area".into()),
    }
}

fn parse_roof(v: &str) -> Result<nwn_bake::iso::Roof, String> {
    use nwn_bake::iso::Roof;
    match v {
        "auto" => Ok(Roof::Auto),
        "keep" => Ok(Roof::Keep),
        m => m.parse().ok().filter(|m: &f32| m.is_finite()).map(Roof::Over).ok_or_else(|| "auto, keep or metres over the floor".into()),
    }
}

fn parse_exposure(v: &str) -> Result<bool, String> {
    match v {
        "auto" => Ok(true),
        "off" => Ok(false),
        _ => Err("auto or off".into()),
    }
}

fn parse_background(v: &str) -> Result<nwn_bake::iso::Background, String> {
    nwn_bake::iso::parse_background(v).map_err(|e| format!("{e:#}"))
}

fn parse_layout(v: &str) -> Result<bool, String> {
    match v {
        "flat" => Ok(false),
        "tree" => Ok(true),
        _ => Err("flat or tree".into()),
    }
}

/// Stops with `message` and `command`'s usage, as clap does for misuse: exit code 2.
fn misuse(command: &str, message: impl std::fmt::Display) -> ! {
    let mut cli = Cli::command();
    cli.build();
    match cli.find_subcommand_mut(command) {
        Some(c) => c.error(clap::error::ErrorKind::MissingRequiredArgument, message).exit(),
        None => cli.error(clap::error::ErrorKind::InvalidSubcommand, message).exit(),
    }
}

/// `nwn-bake 0.2.0`, and the commit when the build knows it.
fn version() -> &'static str {
    let r = nwn_bake::iso::Renderer::this();
    let v = match r.commit {
        Some(c) => format!("{} ({c})", r.version),
        None => r.version.to_string(),
    };
    Box::leak(v.into_boxed_str())
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().collect();
    let mut cli = Cli::command().version(version());
    if args.len() <= 1 {
        // (A bare run is how an installer checks that a new binary starts: the first line stays `Error: usage: nwn-bake`.)
        note!("Error: usage: nwn-bake <COMMAND> [ARGS] [OPTIONS]\n\n{}", cli.render_help());
        std::process::exit(EXIT_MISUSE);
    }
    let matches = cli.get_matches_from(args);
    let parsed = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    match run(parsed.command) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            note!("Error: {e:?}");
            std::process::exit(1);
        }
    }
}

/// Reads `path`, saying which file failed.
fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

/// A character or blueprint (.bic, .utc, .uti) as GFF bytes: binary as it is, nwn_gff's JSON (a name ending in `.json`) converted.
fn read_blueprint(path: &Path) -> Result<Vec<u8>> {
    if !path.to_string_lossy().to_ascii_lowercase().ends_with(".json") {
        return read(path);
    }
    let gff = nwn_bake::module::read_gff(path)?;
    gff.to_bytes().with_context(|| format!("converting {}", path.display()))
}

/// A scene's spec from a file, or from stdin for `-`.
fn read_scene(path: &Path) -> Result<nwn_bake::scene::Spec> {
    let bytes = if path.as_os_str() == "-" {
        let mut b = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut b).context("reading the spec from stdin")?;
        b
    } else {
        read(path)?
    };
    serde_json::from_slice(&bytes).with_context(|| format!("{}: not a scene spec (docs/scene.md)", path.display()))
}

/// Writes `path`, saying which file failed.
fn write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// The game for a character command: every hak in the user folder's `hak/` by name, or with `--module` that module's, in its order.
fn open_game(g: &CharGame) -> Result<Game> {
    let (haks, tlk) = match &g.module {
        None => (Haks::AllInUser, Tlk::given_or_module(g.tlk.given(), None)),
        Some(m) => {
            let m = nwn_bake::module::ModuleSource::open(m).with_context(|| format!("opening the module {}", m.display()))?;
            (Haks::List(m.haks().to_vec()), Tlk::given_or_module(g.tlk.given(), m.tlk().map(str::to_string)))
        }
    };
    let game = nwn_bake::game::open(&GameOptions { root: g.game.root.clone(), user: g.game.user()?, haks, tlk })?;
    if !game.missing_haks().is_empty() {
        note!("haks not found: {}", game.missing_haks().join(", "));
    }
    Ok(game)
}

fn gpu() -> Result<nwn_bake::creature::Gpu> {
    nwn_bake::creature::gpu().context("no GPU and no software adapter (Mesa lavapipe) either")
}

/// Writes a line to stderr. A progress line or a warning that cannot be written (stderr closed: `2>&-`, or a pipe whose reader has
/// gone) is dropped: `eprintln!` would panic, and a panic in a batch's thread would fail the area it was reporting.
fn warn(line: std::fmt::Arguments<'_>) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// Runs a command; the exit code.
fn run(command: Cmd) -> Result<i32> {
    match command {
        Cmd::AreaIso(a) => return area_iso(*a).map(|failed| if failed > 0 { EXIT_AREA_FAILED } else { 0 }),
        Cmd::Area3d(a) => return area3d::run(*a).map(|failed| if failed > 0 { EXIT_AREA_FAILED } else { 0 }),
        Cmd::AreaScene(a) => return area_scene(&a).map(|()| 0),
        Cmd::Serve { game } => return serve(&game).map(|()| 0),
        _ => {}
    }
    match command {
        Cmd::Creature { input, out_dir, pose, items, game } => {
            let items = items.open()?;
            let game = open_game(&game)?;
            let baked = nwn_bake::creature::bake_creature(&game, &gpu()?, &read_blueprint(&input)?, &items, &out_dir, &pose.anim, pose.t)?;
            warn_missing_items(&baked.missing_items);
            if !baked.missing_models.is_empty() {
                note!("missing models: {:?}", baked.missing_models);
            }
            note!("{} meshes, {} emitters, {} textures", baked.meshes, baked.emitters, baked.textures);
        }
        Cmd::Thumb { input, out, size, yaw, pose, items, game } => {
            let items = items.open()?;
            let game = open_game(&game)?;
            let drawn =
                nwn_bake::creature::thumbnail(&game, &gpu()?, &read_blueprint(&input)?, &items, &out, size, &pose.anim, pose.t, yaw)?;
            warn_missing_items(&drawn.missing_items);
            if !drawn.missing_models.is_empty() {
                note!("missing models: {:?}", drawn.missing_models);
            }
        }
        Cmd::ItemThumb { input, out, width, height, yaw, game } => {
            let game = open_game(&game)?;
            let missing =
                nwn_bake::creature::item_thumbnail(&game, &gpu()?, &read_blueprint(&input)?, &out, width, height.unwrap_or(width), yaw)?;
            if !missing.is_empty() {
                note!("missing models: {missing:?}");
            }
        }
        Cmd::PlaceableThumb { row, out, width, height, yaw, tex, game } => {
            let game = open_game(&game)?;
            let (missing, particles) =
                nwn_bake::creature::placeable_thumbnail(&game, &gpu()?, row, &tex, &out, width, height.unwrap_or(width), yaw)?;
            if particles {
                note!("with particles");
            }
            if !missing.is_empty() {
                note!("missing models: {missing:?}");
            }
        }
        Cmd::Scene { spec, out, items, game } => {
            let spec = read_scene(&spec)?;
            // (Misuse in the spec is found before the game is opened, which takes seconds.)
            spec.check()?;
            let items = items.open()?;
            let game = open_game(&game)?;
            let drawn = nwn_bake::scene::render(&game, &gpu()?, &spec, &items, &out)?;
            for (i, o) in drawn.objects.iter().enumerate() {
                for m in &o.missing_items {
                    note!("warning: object {i}: equipped item not found, drawn without it: {m}");
                }
                if !o.missing_models.is_empty() {
                    note!("object {i}: missing models: {:?}", o.missing_models);
                }
            }
            println!("{}", serde_json::to_string(&drawn)?);
        }
        Cmd::Placeables { out, game } => {
            let game = open_game(&game)?;
            write(&out, serde_json::to_vec(&nwn_bake::placeables_index(&game)?)?)?;
        }
        Cmd::Textures { row, game } => {
            let game = open_game(&game)?;
            let model = nwn_bake::placeable_model(&game, row)?;
            println!("{}", serde_json::json!({ "model": model, "textures": nwn_bake::model_textures(&game, &model)? }));
        }
        Cmd::Describe { input, pose, items, game } => {
            let items = items.open()?;
            let game = open_game(&game)?;
            let look = look::describe(&game, &read_blueprint(&input)?, &items, &pose.anim, pose.t)?;
            warn_missing_items(&look.missing_items);
            println!("{}", serde_json::to_string(&look)?);
        }
        Cmd::Look { input, parts_dir, pose, items, game } => {
            let items = items.open()?;
            let game = open_game(&game)?;
            let look = look::ensure(&game, &gpu()?, &read_blueprint(&input)?, &items, &parts_dir, &pose.anim, pose.t)?;
            warn_missing_items(&look.missing_items);
            println!("{}", serde_json::to_string(&look)?);
        }
        Cmd::Part { spec, out, game } => {
            #[derive(serde::Deserialize)]
            struct Spec {
                base: look::PartSpec,
                part: Option<look::PartSpec>,
                anim: String,
                t: f32,
            }
            let spec_file = spec;
            let spec: Spec =
                serde_json::from_slice(&read(&spec_file)?).with_context(|| format!("{}: not a part spec", spec_file.display()))?;
            let game = open_game(&game)?;
            let chunk = look::bake_part(&game, &gpu()?, &spec.base, &spec.anim, spec.t, spec.part.as_ref())?;
            write(&out, chunk.bytes)?;
        }
        Cmd::Kind { texture, colored, game } => {
            let game = open_game(&game)?;
            match look::texture_kind(&game, &texture, colored == "1") {
                Some((image, kind)) => println!("{}", serde_json::json!({ "image": image, "kind": kind })),
                None => bail!("texture {texture}: not found"),
            }
        }
        Cmd::Resources { out, game } => {
            // The names a list of item appearances is built from: the icons (TGA, PLT and DDS named `i...`) and the models of weapon
            // parts (`_b_`, `_m_`, `_t_`), for a tool or server that offers the appearances the game has. Written as JSON, one list a type.
            use mg_core::ResType;
            let game = open_game(&game)?;
            let names = |t: ResType, keep: &dyn Fn(&str) -> bool| -> Vec<String> {
                game.resman.list(t).iter().map(|r| r.to_string().to_lowercase()).filter(|n| keep(n)).collect()
            };
            let list = serde_json::json!({
                "tga": names(ResType::TGA, &|n| n.starts_with('i')),
                "plt": names(ResType::PLT, &|n| n.starts_with('i')),
                "dds": names(ResType::DDS, &|n| n.starts_with('i')),
                "mdl": names(ResType::MDL, &|n| ["_b_", "_m_", "_t_"].iter().any(|k| n.contains(k))),
            });
            write(&out, serde_json::to_vec(&list)?)?;
        }
        Cmd::Plt { texture, out, game } => write(&out, look::plt_png(&*open_game(&game)?, &texture)?)?,
        Cmd::Plain { texture, out, game } => write(&out, look::plain_png(&*open_game(&game)?, &texture)?)?,
        Cmd::Palettes { out_dir, game } => {
            let game = open_game(&game)?;
            std::fs::create_dir_all(&out_dir).with_context(|| format!("making {}", out_dir.display()))?;
            for (name, bytes) in look::palette_pngs(&game)? {
                write(&out_dir.join(format!("{name}.png")), bytes)?;
            }
        }
        Cmd::AreaIso(_) | Cmd::Area3d(_) | Cmd::AreaScene(_) | Cmd::Serve { .. } => unreachable!("handled above"),
    }
    Ok(0)
}

/// `serve`: see `docs/serve.md`.
fn serve(g: &CharGame) -> Result<()> {
    use std::io::{BufRead, Write};
    // A baker that stays: the game and the device are opened once. One request per line on stdin, `{"in": <bic>, "dir": <parts folder>,
    // "anim": "pause1", "t": 0}`; one answer per line on stdout, the `look` result or `{"error": ...}`. Ends when stdin does.
    // Also pictures (`thumb`: a creature, an item with `item`, a placeables.2da row with `placeable` and `textures`) and a placeable's
    // textures (`{"textures_of": row}`), and areas: `{"area_iso": {..}}` and `{"area_iso_key": {..}}` (`nwn_bake::iso::batch::AreaRequest`,
    // `docs/area-iso.md`, `docs/serve.md`), and scenes (`{"scene": {..}, "out": ..}`, `docs/scene.md`). Areas, `textures_of` and
    // `placeables_index` need no GPU.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Req {
        #[serde(rename = "in", default)]
        input: String,
        #[serde(default)]
        dir: String,
        anim: Option<String>,
        t: Option<f32>,
        /// With these, the request is a picture, not a look: `in` is the creature (.utc), `thumb` the PNG to write, `size` its side in pixels.
        /// With `item` too, `in` is an item (.uti) and the picture is of that item on its own.
        thumb: Option<String>,
        #[serde(default)]
        item: bool,
        size: Option<u32>,
        /// An item's picture may be lower than it is wide (`size` is the width).
        height: Option<u32>,
        yaw: Option<f32>,
        /// With `thumb`: a picture of placeables.2da row `placeable` (no `in`), with `textures` ([old, new] pairs) replaced.
        placeable: Option<u32>,
        #[serde(default)]
        textures: Vec<(String, String)>,
        /// On its own: the textures of placeables.2da row `textures_of`'s model, `{"model": .., "textures": [..]}`.
        textures_of: Option<usize>,
        /// On its own: every placeables.2da row with a model, described, written to this file as JSON (`{"ok": true, "rows": n}`).
        placeables_index: Option<String>,
        /// An area's pictures (`nwn_bake::iso::batch::AreaRequest`): answers its manifest, with `action` and `manifest_path`.
        area_iso: Option<nwn_bake::iso::batch::AreaRequest>,
        /// An area's keys, and whether the cache and the output folder have its pictures.
        area_iso_key: Option<nwn_bake::iso::batch::AreaRequest>,
        /// A look or a creature's picture: a folder of item blueprints looked in first for the items a .utc names (`--items`).
        items: Option<String>,
        /// With it, an equipped item not found fails the request (`--strict-items`).
        #[serde(default)]
        strict_items: bool,
        /// Several objects in one picture (`nwn_bake::scene::Spec`, `docs/scene.md`), written to `out`.
        scene: Option<nwn_bake::scene::Spec>,
        /// With `scene`: the PNG to write.
        out: Option<String>,
    }
    let user = g.game.user()?;
    // (The area requests open the game themselves, with each module's haks, and need no user folder. Without one, `serve` still
    // starts, and the requests that need the character game answer why it could not be opened; with one, that is an error now.)
    let game = match open_game(g) {
        Ok(game) => Ok(game),
        Err(e) if user.is_none() && g.module.is_none() => {
            note!("serve: only area requests will work: {e:#}");
            Err(format!("{e:#}"))
        }
        Err(e) => return Err(e),
    };
    // (Areas, placeables_index and textures_of need no GPU: without one, only the requests that do answer with the error.)
    let gpu = gpu();
    let areas = nwn_bake::iso::batch::Service::new(g.game.root.clone(), user);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    writeln!(out, "{}", serde_json::json!({ "ready": true }))?;
    out.flush()?;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = (|| -> Result<String> {
            let r: Req = serde_json::from_str(&line)?;
            if let Some(a) = &r.area_iso {
                return Ok(areas.area_iso(a)?.to_string());
            }
            if let Some(a) = &r.area_iso_key {
                return Ok(areas.area_iso_key(a)?.to_string());
            }
            if r.input.is_empty()
                && r.textures_of.is_none()
                && r.placeables_index.is_none()
                && (r.placeable.is_none() || r.thumb.is_none())
                && r.scene.is_none()
            {
                bail!("unknown request: give `in`, `textures_of`, `placeables_index`, `scene`, `area_iso` or `area_iso_key`");
            }
            if let Some(spec) = &r.scene {
                spec.check()?;
                let out_png = r.out.as_deref().context("a scene request needs `out`, the PNG to write")?;
                let game = game.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
                let gpu = gpu.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))?;
                let items = item_lookup(r.items.as_deref().map(Path::new), r.strict_items)?;
                let drawn = nwn_bake::scene::render(game, gpu, spec, &items, Path::new(out_png))?;
                // (Spliced, not through a `Value`: that would print each f32 with an f64's digits, 1.0040481090545654.)
                let answer = serde_json::to_string(&drawn)?;
                return Ok(format!("{{\"ok\":true,{}", answer.strip_prefix('{').unwrap_or(&answer)));
            }
            let game = game.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
            if let Some(out_json) = &r.placeables_index {
                let index = nwn_bake::placeables_index(game)?;
                write(Path::new(out_json), serde_json::to_vec(&index)?)?;
                return Ok(serde_json::json!({ "ok": true, "rows": index.len() }).to_string());
            }
            if let Some(row) = r.textures_of {
                let model = nwn_bake::placeable_model(game, row)?;
                return Ok(serde_json::json!({ "model": model, "textures": nwn_bake::model_textures(game, &model)? }).to_string());
            }
            // (Only the pictures and looks below need it: the requests above work on a machine with no GPU.)
            let gpu = gpu.as_ref().map_err(|e| anyhow::anyhow!("{e:#}"))?;
            if let (Some(out_png), Some(row)) = (&r.thumb, r.placeable) {
                let size = r.size.unwrap_or(256);
                let (missing, particles) = nwn_bake::creature::placeable_thumbnail(
                    game,
                    gpu,
                    row,
                    &r.textures,
                    Path::new(out_png),
                    size,
                    r.height.unwrap_or(size),
                    r.yaw.unwrap_or(0.0),
                )?;
                return Ok(serde_json::json!({ "ok": true, "missing": missing, "particles": particles }).to_string());
            }
            if let Some(out_png) = &r.thumb {
                if r.item {
                    let missing = nwn_bake::creature::item_thumbnail(
                        game,
                        gpu,
                        &read_blueprint(Path::new(&r.input))?,
                        Path::new(out_png),
                        r.size.unwrap_or(256),
                        r.height.unwrap_or(r.size.unwrap_or(256)),
                        r.yaw.unwrap_or(0.0),
                    )?;
                    return Ok(serde_json::json!({ "ok": true, "missing": missing }).to_string());
                }
                let drawn = nwn_bake::creature::thumbnail(
                    game,
                    gpu,
                    &read_blueprint(Path::new(&r.input))?,
                    &item_lookup(r.items.as_deref().map(Path::new), r.strict_items)?,
                    Path::new(out_png),
                    r.size.unwrap_or(256),
                    r.anim.as_deref().unwrap_or("pause1"),
                    r.t.unwrap_or(0.0),
                    r.yaw.unwrap_or(60.0),
                )?;
                return Ok(
                    serde_json::json!({ "ok": true, "missing": drawn.missing_models, "missing_items": drawn.missing_items }).to_string()
                );
            }
            let look = look::ensure(
                game,
                gpu,
                &read_blueprint(Path::new(&r.input))?,
                &item_lookup(r.items.as_deref().map(Path::new), r.strict_items)?,
                Path::new(&r.dir),
                r.anim.as_deref().unwrap_or("pause1"),
                r.t.unwrap_or(0.0),
            )?;
            Ok(serde_json::to_string(&look)?)
        })();
        match answer {
            Ok(a) => writeln!(out, "{a}")?,
            Err(e) => writeln!(out, "{}", serde_json::json!({ "error": format!("{e:#}") }))?,
        }
        out.flush()?;
    }
    Ok(())
}

/// `area-scene`: see the command's help.
fn area_scene(a: &AreaSceneArgs) -> Result<()> {
    use nwn_bake::area::{RoofRule, SceneOptions};
    let (module, resref) = match &a.area {
        Some(r) => (a.module.clone(), r.clone()),
        None => area_file(&a.module).unwrap_or_else(|| misuse("area-scene", "name the area, or give one area file on its own")),
    };
    let started = Instant::now();
    let source = nwn_bake::module::ModuleSource::open_target(&module, a.target.as_deref())
        .with_context(|| format!("opening the module {}", module.display()))?;
    let area = source.area(&resref)?;
    let module_s = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let haks = a.haks.clone().map_or_else(|| source.haks().to_vec(), |h| h.0);
    // (The scene has no names in it: no talk table.)
    let game =
        nwn_bake::game::open(&GameOptions { root: a.game.root.clone(), user: a.game.user()?, haks: Haks::List(haks), tlk: Tlk::None })?;
    let game_s = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let scene = nwn_bake::area::scene(&game, &area, &SceneOptions { objects: a.objects, ..Default::default() })?;
    let scene_s = started.elapsed().as_secs_f64();
    let stats = scene.probe.stats();
    let rule = RoofRule::default();
    let cut = stats.roof_cut(&rule);
    let applies = scene.roof_cut(&rule).is_some();
    let digest = scene.digest();
    let (n, t) = (&scene.counts, &scene.timings);
    note!(
        "{} ({}, {}x{} tiles, {:?}{}): {} tile triangles from {} models, {} from {} placeables and {} doors, {} water, {} stray left out; {} textures ({} missing)",
        scene.resref,
        scene.tileset,
        scene.tiles[0],
        scene.tiles[1],
        scene.kind(),
        if scene.tileset_interior { ", interior tileset" } else { "" },
        n.tile_triangles,
        n.tile_models,
        n.object_triangles,
        n.placeables,
        n.doors,
        n.water_triangles,
        n.stray,
        n.textures,
        n.textures_missing,
    );
    note!(
        "time: module {module_s:.2} s, game {game_s:.2} s, scene {scene_s:.2} s (area {:.2}, tiles {:.2}, objects {:.2}, textures {:.2}, probe {:.2})",
        t.area.as_secs_f64(),
        t.tiles.as_secs_f64(),
        t.objects.as_secs_f64(),
        t.textures.as_secs_f64(),
        t.probe.as_secs_f64(),
    );
    note!(
        "floor: {} samples, {:.2}..{:.2} m, {:.1}% at the top, {:.1}% drawn floors 2 m or more below it (95th percentile {}), median {:.2} m of the {:.1}% not black",
        stats.samples,
        stats.lowest,
        stats.roof,
        stats.top_share * 100.0,
        stats.floor_share * 100.0,
        stats.floor_p95.map_or("-".to_string(), |p| format!("{p:.2} m")),
        stats.median,
        stats.drawn_share * 100.0,
    );
    let (walkable, blocked) = scene.probe.floor_counts();
    note!(
        "walkable floor: {walkable} of {} samples ({:.1}%), {blocked} of them blocked by placeables",
        stats.samples,
        walkable as f32 / stats.samples.max(1) as f32 * 100.0,
    );
    note!(
        "roof: {} at {:.2} m ({})",
        if applies { "cut" } else { "kept (a surface area); it would be cut" },
        cut.at,
        if cut.slab { "a cave dug out of rock: over its highest floor" } else { "over the floor" },
    );
    for note in &scene.notes {
        note!("note: {note}");
    }
    note!("digest {digest}");
    println!(
        "{}",
        serde_json::json!({
            "area": scene.resref, "tileset": scene.tileset, "tiles": scene.tiles, "kind": scene.kind(), "tileset_interior": scene.tileset_interior,
            "counts": scene.counts, "light": scene.light,
            "seconds": { "module": module_s, "game": game_s, "scene": scene_s, "area": t.area.as_secs_f64(), "tiles": t.tiles.as_secs_f64(),
                         "objects": t.objects.as_secs_f64(), "textures": t.textures.as_secs_f64(), "probe": t.probe.as_secs_f64() },
            "floor": stats, "walkable_floor": { "samples": walkable, "blocked": blocked }, "roof_cut": cut, "roof_cut_applies": applies, "notes": scene.notes, "digest": digest,
        })
    );
    Ok(())
}

/// `area-iso`: see `docs/area-iso.md`. Returns how many areas failed.
fn area_iso(a: AreaIsoArgs) -> Result<usize> {
    use std::io::Write;

    use nwn_bake::iso::batch::{self, Action, FileDigests, Job};
    use nwn_bake::iso::{Format, IsoOptions, OutOptions};
    let mut o = IsoOptions {
        views: a.view.0,
        scale: a.scale,
        quality: a.supersample,
        light: a.light,
        shadows: a.shadows,
        roof: a.roof,
        exposure: a.exposure,
        depth: a.depth,
        ..IsoOptions::default()
    };
    if let Some(b) = a.background {
        o.background = b;
    }
    if let Some(look) = &a.look {
        o.look = serde_json::from_slice(&read(look)?).with_context(|| format!("{}: not a look", look.display()))?;
    }
    let format = a.format.unwrap_or_else(Format::best);
    // (A batch writes a folder per kind and view; named areas side by side.)
    let all = a.areas.all;
    let mut out = if a.layout.unwrap_or(all) { OutOptions::tree(a.out, format) } else { OutOptions { format, ..OutOptions::new(a.out) } };
    if let Some(n) = a.name {
        out.name = n;
    }
    if o.views.len() > 1 && !out.name.contains("{view}") {
        misuse("area-iso", "with several views, --name needs {view} in it");
    }
    let jobs = jobs(a.jobs)?;
    let user = a.game.user()?;

    let started = Instant::now();
    let (source, areas) = a.areas.resolve("area-iso")?;
    let module_s = started.elapsed().as_secs_f64();
    let hak_list = a.haks.map_or_else(|| source.haks().to_vec(), |h| h.0);
    let tlk = Tlk::given_or_module(a.tlk.given(), source.tlk().map(str::to_string));
    let game = nwn_bake::game::open(&GameOptions { root: a.game.root, user, haks: Haks::List(hak_list), tlk: tlk.clone() })?;
    if !game.missing_haks().is_empty() {
        note!("haks not found: {}", game.missing_haks().join(", "));
    }
    let cache = a.cache;
    let job = Job {
        iso: o,
        objects: a.objects,
        out,
        cache: cache.clone(),
        force: a.force,
        strict: a.strict,
        jobs,
        index: all,
        tlk: tlk.recorded(&game),
    };
    // (Quick keys need every hak's digest: worth it for a batch or with a cache, which keeps the digests between runs.)
    let files = (all || cache.is_some()).then(|| FileDigests::at(cache.as_ref().map(|c| c.join("v1").join("files.json"))));
    let open_s = started.elapsed().as_secs_f64();
    let total = areas.len();
    let count = std::sync::atomic::AtomicUsize::new(0);
    note!(
        "{}: {}, {jobs} at a time (module opened in {module_s:.2} s, game in {:.2} s)",
        a.areas.module.display(),
        areas_text(total),
        open_s - module_s
    );
    let report = |r: &batch::AreaResult| {
        let n = count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let s = &r.seconds;
        match (&r.action, &r.manifest) {
            (Action::Failed, _) | (_, None) => note!("[{n}/{total}] {}: failed: {}", r.resref, r.error.as_deref().unwrap_or("?")),
            (action, Some(m)) => {
                let views = m["views"].as_array().map_or_default(|v| {
                    v.iter()
                        .map(|v| format!("{} {}x{}", v["view"].as_str().unwrap_or("?"), v["width"], v["height"]))
                        .collect::<Vec<_>>()
                        .join(", ")
                });
                note!(
                    "[{n}/{total}] {}: {} ({views}); check {:.2} s, scene {:.2} s, drawing {:.2} s, writing {:.2} s",
                    r.resref,
                    serde_json::to_value(action).expect("an action is a plain enum, always JSON").as_str().unwrap_or("?"),
                    s.check,
                    s.scene,
                    s.render,
                    s.write,
                );
                if *action == Action::Rendered {
                    for n in m["notes"].as_array().into_iter().flatten() {
                        note!("  note: {}", n.as_str().unwrap_or_default());
                    }
                }
            }
        }
    };
    let results = batch::run(&game, &source, &areas, &job, files.as_ref(), &report)?;
    if let Some((n, bytes)) = files.as_ref().map(FileDigests::read).filter(|&(n, _)| n > 0) {
        let kept = if cache.is_some() { "; kept with the cache" } else { "" };
        note!("keys: read {n} of the game's files ({:.1} GB) for their digests{kept}", bytes as f64 / 1e9);
    }
    let mut counts = std::collections::BTreeMap::new();
    let mut stdout = std::io::stdout().lock();
    for r in &results {
        // (A reader that has gone, `| head` say, misses the rest of the lines; the run's result stands.)
        let _ = writeln!(stdout, "{}", r.line());
        *counts.entry(serde_json::to_value(r.action)?.as_str().unwrap_or("?").to_string()).or_insert(0) += 1;
    }
    let failed = results.iter().filter(|r| r.action == Action::Failed).count();
    note!(
        "{} in {:.1} s: {}",
        areas_text(results.len()),
        started.elapsed().as_secs_f64(),
        counts.iter().map(|(k, v)| format!("{v} {k}")).collect::<Vec<_>>().join(", "),
    );
    Ok(failed)
}

/// "1 area", "2 areas".
fn areas_text(n: usize) -> String {
    if n == 1 { "1 area".to_string() } else { format!("{n} areas") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn misuse_is_found_before_the_game_is_opened() {
        let parse = |args: &[&str]| Cli::try_parse_from(std::iter::once("nwn-bake").chain(args.iter().copied()));
        for bad in [
            &["nope"][..],
            &["creature"],
            &["thumb", "a.utc", "b.png", "big"],
            &["area-iso", "m.mod", "a", "--out", "o", "--supersample", "9"],
            &["area-iso", "m.mod", "a", "--out", "o", "--scale", "-1"],
            &["area-iso", "m.mod", "a", "--all", "--out", "o"],
            &["area-iso", "m.mod", "a", "--out", "o", "--quality", "2"],
            &["area-3d", "m.mod", "a", "--out", "o", "--quality", "101"],
            &["area-3d", "m.mod", "a", "--out", "o", "--jobs", "0"],
        ] {
            let e = parse(bad).err().unwrap_or_else(|| panic!("{bad:?} parsed"));
            assert_eq!(e.exit_code(), EXIT_MISUSE, "{bad:?}: {e}");
        }
        assert!(parse(&["thumb", "a.utc", "b.png", "128", "-30"]).is_ok(), "a negative yaw is a number");
        assert!(parse(&["scene", "-", "b.png", "--items", "uti", "--strict-items"]).is_ok());
        assert!(parse(&["scene", "a.json"]).is_err(), "a scene needs its PNG");
        assert!(parse(&["area-iso", "m.mod", "a", "--out", "o", "--views", "all,top", "--roof", "-2"]).is_ok());
        let Cmd::AreaIso(a) = parse(&["area-iso", "m.mod", "a", "--out", "o", "--shadows", "off", "--exposure", "off"]).unwrap().command
        else {
            panic!("not area-iso")
        };
        assert_eq!((a.shadows, a.exposure, a.objects), (false, false, true), "on|off options take their value");
    }

    #[test]
    fn an_area_file_alone_names_its_area() {
        let dir = std::env::temp_dir().join(format!("nwn-bake-area-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Town.are.json"), "{}").unwrap();
        assert_eq!(area_file(&dir.join("Town.are.json")), Some((dir.join("Town.are.json"), "town".into())));
        assert_eq!(area_file(&dir.join("Town.git.json")), Some((dir.join("Town.are.json"), "town".into())));
        assert_eq!(area_file(&dir.join("other.are")), None);
        assert_eq!(area_file(&dir.join("module.mod")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
