// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `nwn-bake area-3d`: areas as glTF for the web (`nwn_bake::area3d`, `docs/area-3d.md`).
//!
//! ```text
//! nwn-bake area-3d <module> <area>... --out <dir>      (or --all for every area of the module)
//! nwn-bake area-3d <area.are | area.are.json | area.git | area.git.json> --out <dir>
//! nwn-bake area-3d <module> [<template area>] --draft <draft.json | -> --out <dir>      (or a lone template area file)
//! ```
//!
//! Writes `<area>.glb`, `<area>.floor` (the walkable floor) and `<area>.json` per area into `--out`, and the textures into
//! `--tex-dir` (default `<out>/tex`), named by their content and shared by every area. `nwn-bake area-3d --help` lists the options. An area whose files are already in the
//! output folder or the cache under its key is not baked again. Progress on stderr; one JSON line per area on stdout at the end;
//! the exit code is 3 when any area failed. With `--draft`, the one area baked is the draft made into an area
//! (`nwn_bake::area::draft`), from the template named (or the lone area file) when there is one.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Args;
use nwn_bake::area::draft::{Draft, DraftSource};
use nwn_bake::area3d::batch::{self, Action, Job};
use nwn_bake::area3d::{self, Options};
use nwn_bake::game::{GameOptions, Haks, Tlk};

use crate::{AreaArgs, GameArgs, HakList, TlkArg};

/// `area-3d`'s arguments.
#[derive(Args)]
pub struct Area3dArgs {
    #[command(flatten)]
    areas: AreaArgs,
    /// The output folder
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
    /// Where the textures go [default: <out>/tex]
    #[arg(long, value_name = "DIR")]
    tex_dir: Option<PathBuf>,
    /// What the .glb puts before a texture's file name, a relative path or a URL [default: --tex-dir relative to --out]
    #[arg(long, value_name = "PREFIX")]
    tex_uri: Option<String>,
    /// Textures at most this many texels wide and high (the first mip level that fits)
    #[arg(long, value_name = "TEXELS", default_value_t = Options::default().max_tex, value_parser = parse_max_tex)]
    max_tex: usize,
    /// WebP quality, 0 to 100 (lossy; alpha kept)
    #[arg(long, value_name = "0..100", default_value_t = Options::default().quality, value_parser = parse_webp_quality)]
    quality: f32,
    /// Bake a draft, a layout that is not a file yet (JSON; `-` for stdin), instead of the areas: the area named, or the lone area
    /// file, is its template; with the module alone, a bare area of the draft's tileset
    #[arg(long, value_name = "FILE", conflicts_with = "all")]
    draft: Option<PathBuf>,
    /// The .git's placeables and doors
    #[arg(long, value_name = "on|off", default_value = "on", value_parser = crate::parse_on_off, action = clap::ArgAction::Set)]
    objects: bool,
    /// Areas baked at a time [default: NWN_BAKE_JOBS, then half the cores]
    #[arg(long, value_name = "N", value_parser = crate::parse_jobs)]
    jobs: Option<usize>,
    /// Keep every key's files here
    #[arg(long, value_name = "DIR")]
    cache: Option<PathBuf>,
    /// Bake even what the keys say is there
    #[arg(long)]
    force: bool,
    /// These haks, first highest, instead of the module's
    #[arg(long, value_name = "A,B,C", value_parser = crate::parse_haks)]
    haks: Option<HakList>,
    /// A hak of the module's list that is not found fails the area
    #[arg(long)]
    strict: bool,
    #[command(flatten)]
    game: GameArgs,
    #[command(flatten)]
    tlk: TlkArg,
}

fn parse_max_tex(v: &str) -> Result<usize, String> {
    v.parse().ok().filter(|&n: &usize| n >= 1).ok_or_else(|| "texels, 1 or more".into())
}

fn parse_webp_quality(v: &str) -> Result<f32, String> {
    v.parse().ok().filter(|q: &f32| (0.0..=100.0).contains(q)).ok_or_else(|| "0 to 100".into())
}

/// `path` relative to `from` (both made absolute), with `/`; None when there is no such path (another drive).
fn relative(from: &Path, path: &Path) -> Option<String> {
    let abs = |p: &Path| -> Option<PathBuf> {
        let p = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().ok()?.join(p) };
        // (Lexically: the folders may not exist yet.)
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                c => out.push(c),
            }
        }
        Some(out)
    };
    let (from, path) = (abs(from)?, abs(path)?);
    let (a, b): (Vec<_>, Vec<_>) = (from.components().collect(), path.components().collect());
    if a.first() != b.first() {
        return None;
    }
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_string(), a.len() - common).collect();
    parts.extend(b[common..].iter().map(|c| c.as_os_str().to_string_lossy().to_string()));
    Some(parts.join("/"))
}

/// The draft in `path` (`-`: stdin), checked.
fn read_draft(path: &Path) -> Result<Draft> {
    let bytes = if path == Path::new("-") {
        let mut b = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin().lock(), &mut b).context("reading the draft from stdin")?;
        b
    } else {
        crate::read(path)?
    };
    Draft::parse(&bytes).with_context(|| format!("the draft {}", path.display()))
}

/// Runs the command; how many areas failed.
pub fn run(a: Area3dArgs) -> Result<usize> {
    let mut o = Options { max_tex: a.max_tex, quality: a.quality, objects: a.objects, ..Options::default() };
    let jobs = crate::jobs(a.jobs)?;
    let user = a.game.user()?;
    let out = a.out;
    let tex_dir = a.tex_dir.unwrap_or_else(|| out.join("tex"));
    o.tex_uri = match a.tex_uri {
        Some(u) if u.is_empty() || u.ends_with('/') => u,
        Some(u) => format!("{u}/"),
        None => match relative(&out, &tex_dir) {
            Some(r) => format!("{r}/"),
            None => crate::misuse("area-3d", "--tex-dir is on another drive than --out: give --tex-uri"),
        },
    };

    let started = Instant::now();
    let draft = a.draft.as_deref().map(read_draft).transpose()?;
    let (source, areas) = match &draft {
        Some(_) => a.areas.resolve_template("area-3d")?,
        None => a.areas.resolve("area-3d")?,
    };
    // (The draft made into its area before the game is opened: a draft that does not fit its template fails at once.)
    let draft = match draft {
        Some(d) => {
            let template = areas.first().map(|r| source.area(r)).transpose()?;
            let area = d.area(template.as_ref()).context("the draft")?;
            Some(DraftSource { area })
        }
        None => None,
    };
    let areas = draft.as_ref().map_or(areas, |d| vec![d.area.resref.clone()]);
    let hak_list = a.haks.map_or_else(|| source.haks().to_vec(), |h| h.0);
    let tlk = Tlk::given_or_module(a.tlk.given(), source.tlk().map(str::to_string));
    let game = nwn_bake::game::open(&GameOptions { root: a.game.root, user, haks: Haks::List(hak_list), tlk: tlk.clone() })?;
    let open_s = started.elapsed().as_secs_f64();
    if !game.missing_haks().is_empty() {
        note!("haks not found: {}", game.missing_haks().join(", "));
    }
    let job = Job { options: o, out, tex_dir, cache: a.cache, force: a.force, strict: a.strict, jobs, tlk: tlk.recorded(&game) };
    let total = areas.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let progress = |r: &batch::AreaResult| {
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let s = &r.seconds;
        match r.action {
            Action::Failed => note!("[{n}/{total}] {}: failed: {}", r.area, r.error.as_deref().unwrap_or("?")),
            Action::Baked => note!(
                "[{n}/{total}] {}: baked, {} bytes, {} textures in {:.2} s (scene {:.2}, placements {:.2}, textures {:.2}, glb {:.2}, write {:.2})",
                r.area,
                r.glb_bytes.unwrap_or(0),
                r.textures.unwrap_or(0),
                s.total,
                s.scene,
                s.placements,
                s.textures,
                s.glb,
                s.write,
            ),
            a => note!("[{n}/{total}] {}: {} ({:.2} s)", r.area, if a == Action::Current { "current" } else { "from the cache" }, s.total),
        }
    };
    let from: &dyn nwn_bake::iso::batch::AreaSource = match &draft {
        Some(d) => d,
        None => &source,
    };
    let (results, written) = batch::run(&game, from, &areas, &job, area3d::webp, &progress);
    let failed = results.iter().filter(|r| r.action == Action::Failed).count();
    let mut stdout = std::io::stdout().lock();
    for r in &results {
        // (A reader that has gone, `| head` say, misses the rest of the lines; the run's result stands.)
        let _ = writeln!(stdout, "{}", serde_json::to_string(r)?);
    }
    note!(
        "game opened in {open_s:.2} s; {}, {failed} failed, {written} texture files written; {:.2} s in all",
        super::areas_text(total),
        started.elapsed().as_secs_f64()
    );
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_texture_folder_is_named_relative_to_the_output_folder() {
        assert_eq!(relative(Path::new("/a/out"), Path::new("/a/out/tex")).as_deref(), Some("tex"));
        assert_eq!(relative(Path::new("/a/out/glb"), Path::new("/a/tex")).as_deref(), Some("../../tex"));
        assert_eq!(relative(Path::new("/a/out/./glb/"), Path::new("/a/out/x/../tex")).as_deref(), Some("../tex"));
    }
}
