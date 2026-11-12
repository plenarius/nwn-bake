// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Many areas at once, and the cache: what `nwn-bake area-3d` does with several areas or `--all`.
//!
//! Each area gets a key before anything is built ([`quick_key`], as the area pictures' batch's "quick key"): the format, the
//! `.glb`s' version ([`super::GLB_VERSION`], not the crate's version or the build's commit, so a new build that bakes the same files
//! keeps every key), the options, the area's `.are` and `.git` (as binary GFF), the haks and talk table the game was opened with,
//! and what the game's files are, as the area pictures' quick key has them: each hak's and the talk table's SHA-256, the base
//! game's key file, and each file of `override/` and `development/` by name, length and time. With `--cache` the digests are kept in
//! `<cache>/files.json`, so a hak is read again only when its length or time changed; without one no hak is read, and its length
//! and time stand for it ([`FileDigests::by_stamp`]). An area is baked only when neither of these has it under its key:
//!
//! - the output folder: its `<area>.json` is of this manifest version and names the key, and its `<area>.glb`, `<area>.floor` and
//!   every texture the manifest lists are there;
//! - the cache (`--cache <dir>`): `<dir>/<key>/<area>.glb`, `.floor` and `.json`, and the textures in `<dir>/tex/`. A hit is copied
//!   to the output folder.
//!
//! What is baked goes to the output folder and the cache. Files already in place are not written again. The areas run on `jobs`
//! threads; every file is the same whatever `jobs` is.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{Encode, FORMAT, GLB_VERSION, Inputs, Manifest, Options, TextureFiles, VERSION, bake, write_atomic, write_new};
use crate::game::Game;
use crate::iso::batch::{AreaSource, FileDigests};
use crate::sync::{Recover, caught};

/// What to bake and where.
#[derive(Clone, Debug)]
pub struct Job {
    /// How to bake each area.
    pub options: Options,
    /// `<area>.glb` and `<area>.json` go here.
    pub out: PathBuf,
    /// The textures go here (shared by every area).
    pub tex_dir: PathBuf,
    /// The cache folder (`--cache`); None for no cache.
    pub cache: Option<PathBuf>,
    /// Bake every area, whatever the keys say.
    pub force: bool,
    /// A hak not found fails the area.
    pub strict: bool,
    /// Areas at a time.
    pub jobs: usize,
    /// The talk table asked for (for the manifests).
    pub tlk: Option<String>,
}

/// The default number of threads: half the cores (at least one), or `NWN_BAKE_JOBS`, as the area pictures' batch.
pub fn default_jobs() -> usize {
    if let Some(n) = std::env::var("NWN_BAKE_JOBS").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n > 0) {
        return n;
    }
    (std::thread::available_parallelism().map_or(2, std::num::NonZero::get) / 2).max(1)
}

/// What became of an area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Its files in the output folder were already those of its key.
    Current,
    /// Its files came from the cache.
    Cached,
    /// It was baked, and its files written to the output folder (and the cache).
    Baked,
    /// It could not be baked: `error` says why.
    Failed,
}

/// One area's result.
#[derive(Clone, Debug, Serialize)]
pub struct AreaResult {
    /// The area's resref, lower case.
    pub area: String,
    /// What became of it.
    pub action: Action,
    /// Why it failed; None when it did not.
    pub error: Option<String>,
    /// Its key ([`quick_key`]); None when it failed before the key was made.
    pub key: Option<String>,
    /// Its manifest in the output folder; None when it failed.
    pub manifest: Option<PathBuf>,
    /// Its `.glb`'s length in bytes; None when it failed.
    pub glb_bytes: Option<u64>,
    /// How many texture files its manifest lists; None when it failed.
    pub textures: Option<usize>,
    /// Textures copied from the cache to the texture folder.
    pub textures_copied: usize,
    /// How long each part took.
    pub seconds: Seconds,
}

/// How long each part of an area's run took, in seconds. A part that did not run is 0.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Seconds {
    /// Reading the area and its key, and looking in the output folder and the cache.
    pub check: f64,
    /// Building the scene ([`super::Timings::scene`]).
    pub scene: f64,
    /// The placements and their check ([`super::Timings::placements`]).
    pub placements: f64,
    /// Encoding and writing new textures ([`super::Timings::textures`]).
    pub textures: f64,
    /// Building the `.glb` ([`super::Timings::glb`]).
    pub glb: f64,
    /// Writing the `.glb` and manifest (and the cache's copies).
    pub write: f64,
    /// The whole area, from the start of the check to the end of the writing.
    pub total: f64,
}

/// A file's length, or None when it is not a file.
fn length(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().filter(std::fs::Metadata::is_file).map(|m| m.len())
}

/// What the game's files are, for the key (see the module): one digest, as the area pictures' quick key makes it.
pub fn game_files(game: &Game, digests: &FileDigests) -> String {
    crate::iso::batch::game_files(game, digests, 4)
}

/// The area's key (see the module): `sha256:` and hex. `files` is [`game_files`].
pub fn quick_key(o: &Options, inputs: &Inputs, files: &str) -> String {
    let what = json!({
        "format": FORMAT, "glb_version": GLB_VERSION, "renderer": "nwn-bake",
        "options": { "max_tex": o.max_tex, "webp_quality": o.quality, "objects": o.objects, "tex_uri": o.tex_uri },
        "inputs": inputs, "files": files,
    });
    format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&what).expect("JSON")))
}

/// Whether `dir` has `resref`'s files under `key` (a manifest of this [`VERSION`], its `.glb` and its floor file), and the textures
/// they name in `tex_dir`; the manifest when it does.
fn has(dir: &Path, tex_dir: &Path, resref: &str, key: &str) -> Option<Manifest> {
    let m: Manifest = serde_json::from_slice(&std::fs::read(dir.join(format!("{resref}.json"))).ok()?).ok()?;
    if m.version != VERSION || m.key.as_deref() != Some(key) || length(&dir.join(&m.glb.file)) != Some(m.glb.bytes) {
        return None;
    }
    let floor = m.floor.as_ref()?;
    if length(&dir.join(&floor.file)) != Some(floor.bytes) {
        return None;
    }
    m.textures.iter().all(|t| length(&tex_dir.join(&t.file)) == Some(t.bytes)).then_some(m)
}

/// `key`'s folder in the cache.
fn cache_dir(cache: &Path, key: &str) -> PathBuf {
    cache.join(key.trim_start_matches("sha256:"))
}

/// Bakes `areas` of `source` (a [`crate::module::ModuleSource`], or one area's files) with `game` as `job` says; `done` is called as each area ends (from its thread). The results are in
/// `areas`' order, with how many texture files were written (made, not copied from the cache). An area whose work panics fails with
/// the panic's message; the others go on.
pub fn run(
    game: &Game,
    source: &dyn AreaSource,
    areas: &[String],
    job: &Job,
    encode: Encode,
    done: &(dyn Fn(&AreaResult) + Sync),
) -> (Vec<AreaResult>, usize) {
    // (The haks the game was opened with: the module's, or the ones given in their place.)
    let haks: Vec<String> = game.haks().to_vec();
    let digests = match &job.cache {
        Some(c) => FileDigests::at(Some(c.join("files.json"))),
        None => FileDigests::by_stamp(),
    };
    let files = game_files(game, &digests);
    // (A digest file that cannot be written costs a re-read next time, nothing more.)
    let _ = digests.save();
    let tex = TextureFiles::new(&job.tex_dir, encode, "webp");
    let results = each(areas.len(), job.jobs, |i| {
        let resref = &areas[i];
        let started = Instant::now();
        let r = caught(|| one(game, source, resref, job, &haks, &files, &tex)).unwrap_or_else(|e| {
            let mut r = AreaResult::failed(resref, format!("{e:#}"));
            r.seconds.total = started.elapsed().as_secs_f64();
            r
        });
        // (A `done` that panics, a progress line written to a closed stderr say, loses that line, not the area or the batch.)
        let _ = caught(|| done(&r));
        r
    });
    (results, tex.written())
}

/// `work(i)` for every `i` below `n`, on `jobs` threads (1 to 64), in `i`'s order. `work` must not panic (the caller catches).
fn each<R: Send>(n: usize, jobs: usize, work: impl Fn(usize) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..jobs.clamp(1, 64).min(n.max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    let r = work(i);
                    *slots[i].locked() = Some(r);
                }
            });
        }
    });
    slots.into_iter().map(|s| s.take().expect("every area ran")).collect()
}

impl AreaResult {
    /// Area `resref` failed with `error`, before anything was known of it.
    fn failed(resref: &str, error: String) -> AreaResult {
        AreaResult {
            area: resref.to_ascii_lowercase(),
            action: Action::Failed,
            error: Some(error),
            key: None,
            manifest: None,
            glb_bytes: None,
            textures: None,
            textures_copied: 0,
            seconds: Seconds::default(),
        }
    }
}

fn one(game: &Game, source: &dyn AreaSource, resref: &str, job: &Job, haks: &[String], files: &str, tex: &TextureFiles) -> AreaResult {
    let started = Instant::now();
    let mut r = AreaResult { error: None, ..AreaResult::failed(resref, String::new()) };
    if let Err(e) = (|| -> Result<()> {
        if job.strict && !game.missing_haks().is_empty() {
            bail!("haks not found: {}", game.missing_haks().join(", "));
        }
        let area = source.area(resref)?;
        r.area = area.resref.clone();
        let inputs = Inputs::of(&area, haks, game.missing_haks(), job.tlk.as_deref())?;
        let key = quick_key(&job.options, &inputs, files);
        r.key = Some(key.clone());
        let manifest_path = job.out.join(format!("{}.json", area.resref));
        let found = |m: &Manifest, r: &mut AreaResult, action| {
            r.action = action;
            r.glb_bytes = Some(m.glb.bytes);
            r.textures = Some(m.textures.len());
            r.manifest = Some(manifest_path.clone());
        };
        if !job.force {
            if let Some(m) = has(&job.out, &job.tex_dir, &area.resref, &key) {
                found(&m, &mut r, Action::Current);
                r.seconds.check = started.elapsed().as_secs_f64();
                return Ok(());
            }
            if let Some(cache) = &job.cache {
                let dir = cache_dir(cache, &key);
                if let Some(m) = has(&dir, &cache.join("tex"), &area.resref, &key) {
                    for t in &m.textures {
                        let to = job.tex_dir.join(&t.file);
                        if length(&to) != Some(t.bytes) {
                            write_new(&to, &std::fs::read(cache.join("tex").join(&t.file))?)?;
                            r.textures_copied += 1;
                        }
                    }
                    write_atomic(&job.out.join(&m.glb.file), &std::fs::read(dir.join(&m.glb.file))?)?;
                    if let Some(f) = &m.floor {
                        write_atomic(&job.out.join(&f.file), &std::fs::read(dir.join(&f.file))?)?;
                    }
                    write_atomic(&manifest_path, &std::fs::read(dir.join(format!("{}.json", area.resref)))?)?;
                    found(&m, &mut r, Action::Cached);
                    r.seconds.check = started.elapsed().as_secs_f64();
                    return Ok(());
                }
            }
        }
        r.seconds.check = started.elapsed().as_secs_f64();
        let baked = bake(game, &area, &job.options, tex, inputs, Some(key.clone()))?;
        let t = &baked.timings;
        (r.seconds.scene, r.seconds.placements, r.seconds.textures, r.seconds.glb) = (t.scene, t.placements, t.textures, t.glb);
        let written = Instant::now();
        let m = &baked.manifest;
        let text = serde_json::to_vec_pretty(m)?;
        // (The manifest last: a reader that finds it finds the files it names.)
        let floor_file = m.floor.as_ref().map_or_else(|| format!("{}.floor", area.resref), |f| f.file.clone());
        write_atomic(&job.out.join(&m.glb.file), &baked.glb)?;
        write_atomic(&job.out.join(&floor_file), &baked.floor)?;
        write_atomic(&manifest_path, &text)?;
        if let Some(cache) = &job.cache {
            let dir = cache_dir(cache, &key);
            for t in &m.textures {
                let to = cache.join("tex").join(&t.file);
                if length(&to) != Some(t.bytes) {
                    write_new(&to, &std::fs::read(tex.dir().join(&t.file)).with_context(|| format!("texture {}", t.file))?)?;
                }
            }
            write_atomic(&dir.join(&m.glb.file), &baked.glb)?;
            write_atomic(&dir.join(&floor_file), &baked.floor)?;
            write_atomic(&dir.join(format!("{}.json", area.resref)), &text)?;
        }
        r.seconds.write = written.elapsed().as_secs_f64();
        found(m, &mut r, Action::Baked);
        Ok(())
    })() {
        r.action = Action::Failed;
        r.error = Some(format!("{e:#}"));
    }
    r.seconds.total = started.elapsed().as_secs_f64();
    r
}
