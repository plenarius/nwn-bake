// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Many areas at once, and the cache: what `nwn-bake area-iso` does with several areas or `--all`, and `serve`'s `area_iso`.
//!
//! Every area gets two keys (`docs/area-iso.md`, "Keys"):
//!
//! - the **quick key** needs no scene: the renderer's name and the pictures' version ([`super::PICTURES_VERSION`], not the build's
//!   commit), the options, the area's `.are` and `.git` (as binary GFF), the haks and talk table it was opened with, and what the
//!   game's files are (each hak's SHA-256, each folder's listing, the base game's key file; [`KeyContext`]). It changes whenever any
//!   hak changes.
//! - the **exact key** needs the scene (a second or so) but no drawing: the same, with the digest of every resource the scene
//!   looked up ([`crate::area::Scene::reads`], [`Game::resources_digest`]) in place of the game's files. A hak change that does not
//!   touch what the area reads leaves it as it was.
//!
//! An area is drawn only when neither key finds its pictures: not in the output folder (its manifest there names the key and its
//! files are there), not in the cache (`--cache`: a folder per exact key, and small files from quick keys to exact keys). Pictures
//! that are already in the output folder are not written again, so their files keep their times.
//!
//! The work runs on `jobs` threads. Each area is first checked (and its scene built when it must be); an area that must be drawn
//! puts its views at the front of the queue, so the threads draw the views of the areas already started before starting more
//! areas. Every picture and file is the same whatever `jobs` is and whichever thread drew it.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Drawn, Inputs, IsoOptions, OptionsInfo, OutOptions, Prepared, Renderer, View};
use crate::area::{Scene, SceneOptions};
use crate::game::{Game, Tlk};
use crate::module::{AreaFiles, ModuleSource};
use crate::sync::{Recover, caught};

/// What to draw and where.
#[derive(Clone, Debug)]
pub struct Job {
    /// How to draw each area.
    pub iso: IsoOptions,
    /// The `.git`'s placeables and doors.
    pub objects: bool,
    /// Where and how to write the pictures and manifests.
    pub out: OutOptions,
    /// The cache folder, if any.
    pub cache: Option<PathBuf>,
    /// Draw every area, whatever the keys say (the cache and the output folder are still written).
    pub force: bool,
    /// A hak not found fails the area.
    pub strict: bool,
    /// Areas at a time (threads).
    pub jobs: usize,
    /// Also write `manifest.jsonl` and `index.json` in the output folder, for every area of the run.
    pub index: bool,
    /// The custom talk table's name, for the manifests: the one asked for, when it was read ([`Tlk::recorded`]).
    pub tlk: Option<String>,
}

/// The default number of threads: half the cores (at least one), or `NWN_BAKE_JOBS`: a batch is a background job, often beside a
/// game server, a website and other work.
pub fn default_jobs() -> usize {
    if let Some(n) = std::env::var("NWN_BAKE_JOBS").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n > 0) {
        return n;
    }
    (std::thread::available_parallelism().map_or(2, std::num::NonZero::get) / 2).max(1)
}

/// What became of an area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Its pictures in the output folder were already those of its key.
    Current,
    /// Its pictures came from the cache.
    Cached,
    /// It was drawn.
    Rendered,
    /// It could not be drawn: `error` says why.
    Failed,
}

/// One area's result.
#[derive(Clone, Debug)]
pub struct AreaResult {
    /// The area's resref, lower case.
    pub resref: String,
    /// What became of it.
    pub action: Action,
    /// Why it failed; None when it did not.
    pub error: Option<String>,
    /// The manifest as it is in the output folder (None when the area failed).
    pub manifest: Option<Value>,
    /// Its path.
    pub manifest_path: PathBuf,
    /// Seconds: checking the keys (reading the area, the quick key), the scene and the exact key, drawing (all views, each on its
    /// thread, added up), writing.
    pub seconds: Seconds,
    /// Each view drawn: (view, seconds, the exposure's gain).
    pub views: Vec<(View, f64, f32)>,
}

/// How long each part of an area's run took, in seconds. A part that did not run is 0.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Seconds {
    /// Checking the keys: reading the area, the quick key, and looking in the output folder and the cache (the scene apart).
    pub check: f64,
    /// Building the scene and working out the exact key.
    pub scene: f64,
    /// Drawing: every view's time, added up (each view draws on its own thread).
    pub render: f64,
    /// Writing the pictures and the manifest, to the output folder and the cache.
    pub write: f64,
}

impl AreaResult {
    /// The line `area-iso` prints for it.
    pub fn line(&self) -> Value {
        match &self.manifest {
            Some(m) => json!({
                "area": self.resref, "status": "ok", "action": self.action, "manifest": self.manifest_path,
                "key": m["key"], "quick_key": m["quick_key"], "roof_cut_m": m["roof_cut_m"],
                "views": self.views.iter().map(|(v, s, e)| json!({ "view": v, "seconds": s, "exposure": e })).collect::<Vec<_>>(),
                "seconds": self.seconds,
            }),
            None => json!({ "area": self.resref, "status": "failed", "action": self.action, "error": self.error, "seconds": self.seconds }),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Keys.

/// SHA-256 of files, remembered by (path, size, modification time) so a file is read again only when it changed; kept in the
/// cache folder (`files.json`) between runs.
#[derive(Default)]
pub struct FileDigests {
    known: Mutex<BTreeMap<String, (u64, i128, String)>>,
    path: Option<PathBuf>,
    changed: Mutex<bool>,
    /// Files read for their digest in this run, and their bytes.
    read: Mutex<(usize, u64)>,
    /// Never read a file: its length and time stand for its content ([`FileDigests::by_stamp`]).
    by_stamp: bool,
}

impl FileDigests {
    /// Remembered in `path` (read now, written by [`FileDigests::save`]); in memory only when None.
    pub fn at(path: Option<PathBuf>) -> FileDigests {
        let known = path.as_deref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        FileDigests { known: Mutex::new(known), path, ..FileDigests::default() }
    }

    /// No file read: a file's "digest" is its length and modification time. For a run with nowhere to keep the digests, where
    /// reading every hak each time would cost more than it saves (17 GB of haks: about a minute). A touched hak then counts as
    /// changed.
    pub fn by_stamp() -> FileDigests {
        FileDigests { by_stamp: true, ..FileDigests::default() }
    }

    /// How many files were read for their digests since this was made, and how many bytes: a first run with many gigabytes of haks
    /// takes minutes, which a caller may want to say.
    pub fn read(&self) -> (usize, u64) {
        *self.read.locked()
    }

    /// `sha256:` and hex of `path`'s bytes; None when it is not a readable file.
    pub fn digest(&self, path: &Path) -> Option<String> {
        let meta = std::fs::metadata(path).ok().filter(std::fs::Metadata::is_file)?;
        let mtime = mtime_ns(&meta);
        if self.by_stamp {
            return Some(format!("stamp:{}:{mtime}", meta.len()));
        }
        // (By the file itself: a hak linked into another user folder is the same hak.)
        let name = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().into_owned();
        if let Some((size, t, d)) = self.known.locked().get(&name)
            && *size == meta.len()
            && *t == mtime
        {
            return Some(d.clone());
        }
        let mut h = Sha256::new();
        let mut f = std::fs::File::open(path).ok()?;
        std::io::copy(&mut f, &mut h).ok()?;
        let d = format!("sha256:{:x}", h.finalize());
        {
            let mut read = self.read.locked();
            read.0 += 1;
            read.1 += meta.len();
        }
        self.known.locked().insert(name, (meta.len(), mtime, d.clone()));
        *self.changed.locked() = true;
        Some(d)
    }

    /// Writes what is remembered, when anything changed.
    pub fn save(&self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if !std::mem::take(&mut *self.changed.locked()) {
            return Ok(());
        }
        super::write_file(path, &serde_json::to_vec(&*self.known.locked())?)
    }
}

fn mtime_ns(meta: &std::fs::Metadata) -> i128 {
    meta.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i128)
}

/// What the keys of a run share: the renderer, the options, and (for the quick key) what the game's files are.
pub struct KeyContext {
    common: String,
    /// The game's files, for the quick key: None when quick keys are not used.
    game_files: Option<String>,
}

/// The renderer as the keys see it: its name and the pictures' version ([`super::PICTURES_VERSION`]). Not the crate's version or
/// the build's commit: a new build that draws the same pictures keeps every key, so nothing is drawn again.
pub fn renderer_id() -> String {
    let r = Renderer::this();
    format!("{} pictures {}", r.name, r.pictures)
}

impl KeyContext {
    /// For `job` on `game`. With `files`, quick keys are made too: each of the game's layers by its label and kind, and for a hak
    /// (or any one file) its SHA-256, for a folder (`override/`, `development/`) each file's name, size and time, for the base
    /// game its key file's SHA-256; the custom talk table's SHA-256.
    pub fn new(game: &Game, job: &Job, files: Option<&FileDigests>) -> KeyContext {
        let mut common = Sha256::new();
        common.update(b"nwn-bake/area-iso keys 1\0");
        common.update(super::VERSION.to_le_bytes());
        common.update(renderer_id().as_bytes());
        common.update([0]);
        let options = json!({ "options": OptionsInfo::of(&job.iso, job.objects, job.out.format), "views": job.iso.views });
        common.update(options.to_string().as_bytes());
        let common = format!("{:x}", common.finalize());
        let game_files = files.map(|f| game_files(game, f, job.jobs.clamp(1, 4)));
        KeyContext { common, game_files }
    }

    /// Whether quick keys are made.
    pub fn quick_keys(&self) -> bool {
        self.game_files.is_some()
    }

    fn start(&self, what: &str, resref: &str, inputs: &Inputs) -> Sha256 {
        let mut h = Sha256::new();
        h.update(what.as_bytes());
        h.update([0]);
        h.update(self.common.as_bytes());
        h.update([0]);
        h.update(resref.as_bytes());
        h.update([0]);
        h.update(serde_json::to_string(inputs).unwrap_or_default().as_bytes());
        h.update([0]);
        h
    }

    /// The quick key of an area (None when quick keys are not used).
    pub fn quick(&self, resref: &str, inputs: &Inputs) -> Option<String> {
        let files = self.game_files.as_ref()?;
        let mut h = self.start("quick", resref, inputs);
        h.update(files.as_bytes());
        Some(format!("sha256:{:x}", h.finalize()))
    }

    /// The exact key of an area: its inputs, its name as the manifest gives it, and `resources`
    /// ([`Game::resources_digest`] of its scene's reads).
    pub fn exact(&self, resref: &str, inputs: &Inputs, title: Option<&str>, resources: &str) -> String {
        let mut h = self.start("exact", resref, inputs);
        h.update(title.unwrap_or("\u{0}none").as_bytes());
        h.update([0]);
        h.update(resources.as_bytes());
        format!("sha256:{:x}", h.finalize())
    }
}

/// One digest of what the game's files are (see [`KeyContext::new`]; area-3d's key uses it too). The files not known yet are read
/// on `threads` threads (a first run reads every hak once: a few minutes for a module with many gigabytes of haks;
/// [`FileDigests::read`] says how much was read).
pub(crate) fn game_files(game: &Game, files: &FileDigests, threads: usize) -> String {
    let root = &game.install().root;
    let mut wanted: Vec<PathBuf> =
        game.data.resman.layers().iter().filter_map(|l| l.container.watched()).filter(|p| p.is_file()).map(Path::to_path_buf).collect();
    wanted.push(root.join("data/nwn_base.key"));
    wanted.extend(game.tlk_path().map(Path::to_path_buf));
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                while let Some(p) = wanted.get(next.fetch_add(1, Ordering::Relaxed)) {
                    files.digest(p);
                }
            });
        }
    });
    let mut h = Sha256::new();
    h.update(b"nwn-bake/game-files 1\0");
    for layer in game.data.resman.layers() {
        h.update(layer.label.as_bytes());
        h.update([0]);
        h.update(format!("{:?}", layer.class).as_bytes());
        h.update([0]);
        match layer.container.watched() {
            Some(p) if p.is_file() => h.update(files.digest(p).unwrap_or_default().as_bytes()),
            Some(p) if p.is_dir() => h.update(folder_listing(p).as_bytes()),
            _ => {}
        }
        h.update([0]);
    }
    h.update(files.digest(&root.join("data/nwn_base.key")).unwrap_or_default().as_bytes());
    h.update([0]);
    if let Some(t) = game.tlk_path() {
        h.update(files.digest(t).unwrap_or_default().as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// Each file in folder `p` (not its subfolders): name, size, time.
fn folder_listing(p: &Path) -> String {
    let mut files: Vec<(String, u64, i128)> = std::fs::read_dir(p)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let meta = std::fs::metadata(e.path()).ok().filter(std::fs::Metadata::is_file)?;
            Some((e.file_name().to_string_lossy().into_owned(), meta.len(), mtime_ns(&meta)))
        })
        .collect();
    files.sort();
    let mut h = Sha256::new();
    for (n, s, t) in files {
        h.update(format!("{n}\0{s}\0{t}\0").as_bytes());
    }
    format!("{:x}", h.finalize())
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The cache.

/// The cache folder: `exact/<ab>/<key>/` (the pictures, `<view>.<ext>` and `<view>.depth.png`, and `manifest.json`) and
/// `quick/<ab>/<key>` (the exact key a quick key was found to have). Keys are their hex, without `sha256:`.
pub struct Cache {
    dir: PathBuf,
}

fn hex(key: &str) -> &str {
    key.strip_prefix("sha256:").unwrap_or(key)
}

impl Cache {
    /// The cache in `dir`. Its files go in `dir/v1/`: the cache layout's version.
    pub fn new(dir: &Path) -> Cache {
        Cache { dir: dir.join("v1") }
    }

    fn exact_dir(&self, key: &str) -> PathBuf {
        let k = hex(key);
        self.dir.join("exact").join(&k[..2.min(k.len())]).join(k)
    }

    fn quick_file(&self, key: &str) -> PathBuf {
        let k = hex(key);
        self.dir.join("quick").join(&k[..2.min(k.len())]).join(k)
    }

    /// The manifest kept under exact key `key`, when the cache has it whole.
    pub fn get(&self, key: &str) -> Option<Value> {
        let dir = self.exact_dir(key);
        let m: Value = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).ok()?).ok()?;
        files_there(&m, &dir).then_some(m)
    }

    /// The exact key quick key `quick` was found to have.
    pub fn exact_of(&self, quick: &str) -> Option<String> {
        let k = std::fs::read_to_string(self.quick_file(quick)).ok()?;
        let k = k.trim();
        (k.starts_with("sha256:") && k.len() == 71).then(|| k.to_string())
    }

    /// Notes that quick key `quick` has exact key `exact`.
    pub fn link(&self, quick: &str, exact: &str) -> Result<()> {
        if self.exact_of(quick).as_deref() == Some(exact) {
            return Ok(());
        }
        super::write_file(&self.quick_file(quick), exact.as_bytes())
    }

    /// Keeps an area's files (`files`: name in the cache, bytes) and its manifest (file names as in the cache) under `key`.
    fn put(&self, key: &str, manifest: &Value, files: &[(String, Arc<Vec<u8>>)]) -> Result<()> {
        let dir = self.exact_dir(key);
        if dir.join("manifest.json").is_file() {
            return Ok(());
        }
        static N: AtomicUsize = AtomicUsize::new(0);
        let tmp = self.dir.join("tmp").join(format!("{}-{}-{}", hex(key), std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&tmp).with_context(|| format!("making {}", tmp.display()))?;
        for (name, bytes) in files {
            let path = tmp.join(name);
            std::fs::write(&path, bytes.as_slice()).with_context(|| format!("writing {}", path.display()))?;
        }
        let path = tmp.join("manifest.json");
        std::fs::write(&path, serde_json::to_vec_pretty(manifest)?).with_context(|| format!("writing {}", path.display()))?;
        let parent = dir.parent().expect("an exact key's folder is inside the cache");
        std::fs::create_dir_all(parent).with_context(|| format!("making {}", parent.display()))?;
        if std::fs::rename(&tmp, &dir).is_err() {
            // (Another process kept it first.)
            let _ = std::fs::remove_dir_all(&tmp);
        }
        Ok(())
    }
}

/// Whether every file a manifest names is in `dir` with the size it says.
fn files_there(m: &Value, dir: &Path) -> bool {
    let Some(views) = m["views"].as_array() else { return false };
    !views.is_empty()
        && views.iter().all(|v| {
            let size = |f: &Value| f.as_str().and_then(|f| std::fs::metadata(dir.join(f)).ok()).map(|m| m.len());
            let pic = size(&v["file"]);
            pic.is_some() && (v["bytes"].is_null() || v["bytes"].as_u64() == pic) && (v["depth"].is_null() || size(&v["depth"]).is_some())
        })
}

// ---------------------------------------------------------------------------------------------------------------------------------
// One area.

/// Where areas are read from: a module ([`ModuleSource`]) or an area's own files ([`AreaPaths`]).
pub trait AreaSource: Sync {
    /// The files of area `resref`. Fails when the source has no such area or its files can't be read.
    fn area(&self, resref: &str) -> Result<AreaFiles>;
}

impl AreaSource for ModuleSource {
    fn area(&self, resref: &str) -> Result<AreaFiles> {
        ModuleSource::area(self, resref)
    }
}

/// One area's files by path: its `.are` and its `.git` (binary or nwn_gff's JSON; no `.git` is an area with nothing placed).
pub struct AreaPaths {
    /// The `.are` file.
    pub are: PathBuf,
    /// The `.git` file; None for an area with nothing placed.
    pub git: Option<PathBuf>,
}

impl AreaSource for AreaPaths {
    fn area(&self, resref: &str) -> Result<AreaFiles> {
        let a = area_from_files(&self.are, self.git.as_deref())?;
        if !a.resref.eq_ignore_ascii_case(resref) {
            bail!("{} is area {}, not {resref}", self.are.display(), a.resref);
        }
        Ok(a)
    }
}

/// What one area's work shares: the job, the game, the module, the keys and the cache.
pub struct Batch<'a> {
    /// What to draw and where.
    pub job: &'a Job,
    /// The game the areas are drawn from.
    pub game: &'a Game,
    /// Where the areas are read from.
    pub source: &'a dyn AreaSource,
    /// What every area's keys share.
    pub keys: KeyContext,
    /// The cache; None for no cache.
    pub cache: Option<Cache>,
    /// Threads each scene decodes its textures on.
    pub scene_threads: usize,
}

/// What an area's check found: done, or a scene to draw.
enum Checked {
    Done(AreaResult),
    Draw(Box<ToDraw>),
}

/// An area to draw: its prepared scene, and what its manifest needs.
struct ToDraw {
    resref: String,
    /// The scene without its triangles, water and textures (the prepared copy has them).
    scene: Scene,
    title: Option<String>,
    inputs: Inputs,
    resources: String,
    key: String,
    quick: Option<String>,
    prepared: Prepared,
    seconds: Seconds,
    /// Each view as it is drawn (in the job's order).
    drawn: Mutex<Vec<Option<DrawnView>>>,
    /// Why views were not drawn.
    errors: Mutex<Vec<String>>,
    left: AtomicUsize,
}

/// A view drawn and its files' bytes (the picture's pixels dropped).
struct DrawnView {
    drawn: Drawn,
    picture: Arc<Vec<u8>>,
    depth: Option<Arc<Vec<u8>>>,
}

impl Batch<'_> {
    /// The manifest of `resref` in the output folder, when there is one whose files are all there.
    fn in_out(&self, resref: &str) -> Option<Value> {
        let path = self.job.out.dir.join(self.job.out.manifest(resref));
        let m: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        files_there(&m, &self.job.out.dir).then_some(m)
    }

    fn result(&self, resref: &str, action: Action, manifest: Option<Value>, error: Option<String>, seconds: Seconds) -> AreaResult {
        AreaResult {
            resref: resref.to_string(),
            action,
            error,
            manifest,
            manifest_path: self.job.out.dir.join(self.job.out.manifest(resref)),
            seconds,
            views: Vec::new(),
        }
    }

    /// Writes the manifest `m` (made for exact key `key`) as the output folder's, with quick key `quick`, the files named as the
    /// output folder names them.
    fn write_out_manifest(&self, resref: &str, mut m: Value, quick: Option<&str>) -> Result<Value> {
        m["quick_key"] = quick.map_or(Value::Null, |q| Value::String(q.into()));
        if let Some(views) = m["views"].as_array_mut() {
            for v in views {
                let Some(view) = v["view"].as_str().and_then(|n| View::parse_list(n).ok()).and_then(|l| l.first().copied()) else {
                    continue;
                };
                v["file"] = json!(self.job.out.picture(resref, view));
                if !v["depth"].is_null() {
                    v["depth"] = json!(self.job.out.depth(resref, view));
                }
            }
        }
        super::write_file(&self.job.out.dir.join(self.job.out.manifest(resref)), &serde_json::to_vec_pretty(&m)?)?;
        Ok(m)
    }

    /// Puts the cache's files of exact key `key` (its manifest `cached`) in the output folder: linked where they can be, else copied.
    fn export(&self, resref: &str, key: &str, cached: &Value, quick: Option<&str>) -> Result<Value> {
        let cache = self.cache.as_ref().context("no cache")?;
        let dir = cache.exact_dir(key);
        for v in cached["views"].as_array().into_iter().flatten() {
            let view = v["view"]
                .as_str()
                .and_then(|n| View::parse_list(n).ok())
                .and_then(|l| l.first().copied())
                .context("a view with no name")?;
            let mut pairs = vec![(v["file"].as_str().context("a view with no file")?.to_string(), self.job.out.picture(resref, view))];
            if let Some(d) = v["depth"].as_str() {
                pairs.push((d.to_string(), self.job.out.depth(resref, view)));
            }
            for (from, to) in pairs {
                place(&dir.join(from), &self.job.out.dir.join(to))?;
            }
        }
        self.write_out_manifest(resref, cached.clone(), quick)
    }

    /// Checks area `resref`: done already (in the output folder or the cache), failed, or to be drawn.
    fn check(&self, resref: &str) -> Checked {
        let started = Instant::now();
        let mut seconds = Seconds::default();
        match self.check_inner(resref, started, &mut seconds) {
            Ok(c) => c,
            Err(e) => {
                seconds.check = started.elapsed().as_secs_f64() - seconds.scene;
                Checked::Done(self.result(resref, Action::Failed, None, Some(format!("{e:#}")), seconds))
            }
        }
    }

    fn check_inner(&self, resref: &str, started: Instant, seconds: &mut Seconds) -> Result<Checked> {
        let job = self.job;
        let game = self.game;
        if job.strict && !game.missing_haks().is_empty() {
            bail!("haks not found: {}", game.missing_haks().join(", "));
        }
        let area = self.source.area(resref)?;
        let resref = area.resref.clone();
        let title = area.are.root.locstring("Name").and_then(|n| game.locstring(n)).filter(|n| !n.trim().is_empty());
        let inputs = Inputs::of(&area, game.haks(), game.missing_haks(), job.tlk.as_deref())?;
        let quick = self.keys.quick(&resref, &inputs);
        let out = if job.force { None } else { self.in_out(&resref) };
        if let (Some(q), Some(m)) = (&quick, &out)
            && m["quick_key"].as_str() == Some(q)
            && m["key"].is_string()
        {
            seconds.check = started.elapsed().as_secs_f64();
            return Ok(Checked::Done(self.result(&resref, Action::Current, out, None, *seconds)));
        }
        if let (Some(q), Some(cache), false) = (&quick, &self.cache, job.force)
            && let Some(k) = cache.exact_of(q)
            && let Some(cached) = cache.get(&k)
        {
            let m = match &out {
                Some(m) if m["key"].as_str() == Some(&k) => self.write_out_manifest(&resref, m.clone(), Some(q))?,
                _ => self.export(&resref, &k, &cached, Some(q))?,
            };
            seconds.check = started.elapsed().as_secs_f64();
            let action = if out.as_ref().is_some_and(|o| o["key"].as_str() == Some(&k)) { Action::Current } else { Action::Cached };
            return Ok(Checked::Done(self.result(&resref, action, Some(m), None, *seconds)));
        }
        seconds.check = started.elapsed().as_secs_f64();

        let t = Instant::now();
        let mut scene = crate::area::scene(game, &area, &SceneOptions { objects: job.objects, threads: self.scene_threads })?;
        let resources = game.resources_digest(&scene.reads);
        let key = self.keys.exact(&resref, &inputs, title.as_deref(), &resources);
        seconds.scene = t.elapsed().as_secs_f64();
        drop(area);
        if !job.force {
            if let Some(m) = &out
                && m["key"].as_str() == Some(&key)
            {
                let m = if m["quick_key"].as_str() == quick.as_deref() {
                    m.clone()
                } else {
                    self.write_out_manifest(&resref, m.clone(), quick.as_deref())?
                };
                if let (Some(c), Some(q)) = (&self.cache, &quick) {
                    c.link(q, &key)?;
                }
                return Ok(Checked::Done(self.result(&resref, Action::Current, Some(m), None, *seconds)));
            }
            if let Some(cache) = &self.cache
                && let Some(cached) = cache.get(&key)
            {
                let m = self.export(&resref, &key, &cached, quick.as_deref())?;
                if let Some(q) = &quick {
                    cache.link(q, &key)?;
                }
                return Ok(Checked::Done(self.result(&resref, Action::Cached, Some(m), None, *seconds)));
            }
        }
        let prepared = super::prepare(&scene, &job.iso)?;
        // (What the manifest needs of the scene is kept; the prepared copy has what is drawn.)
        scene.triangles = Vec::new();
        scene.water = Vec::new();
        scene.textures = Vec::new();
        let views = job.iso.views.len();
        Ok(Checked::Draw(Box::new(ToDraw {
            resref,
            scene,
            title,
            inputs,
            resources,
            key,
            quick,
            prepared,
            seconds: *seconds,
            drawn: Mutex::new((0..views).map(|_| None).collect()),
            errors: Mutex::new(Vec::new()),
            left: AtomicUsize::new(views),
        })))
    }

    /// Draws view number `i` of `d` and makes its files.
    fn draw_view(&self, d: &ToDraw, i: usize) -> Result<()> {
        let view = self.job.iso.views[i];
        let mut drawn = d.prepared.draw(view);
        let opaque = !matches!(self.job.iso.background, super::Background::Transparent);
        let picture = Arc::new(super::encode(&drawn.picture, self.job.out.format, opaque)?);
        let depth = match &drawn.picture.depth {
            Some(dp) => Some(Arc::new(super::encode_depth(dp, drawn.picture.width, drawn.picture.height)?)),
            None => None,
        };
        // (The pixels are in the files now.)
        drawn.picture.rgba = Vec::new();
        if let Some(dp) = &mut drawn.picture.depth {
            dp.values = Vec::new();
        }
        d.drawn.locked()[i] = Some(DrawnView { drawn, picture, depth });
        Ok(())
    }

    /// Writes a drawn area: the output folder, then the cache.
    fn finish(&self, d: &ToDraw) -> AreaResult {
        let ToDraw { resref, scene, title, inputs, resources, key, quick, prepared, seconds, .. } = d;
        let (mut seconds, key, quick) = (*seconds, key.clone(), quick.clone());
        let t = Instant::now();
        let views: Vec<DrawnView> = match std::mem::take(&mut *d.drawn.locked()).into_iter().collect::<Option<Vec<_>>>() {
            Some(v) => v,
            None => {
                let why = d.errors.locked().join("; ");
                return self.result(resref, Action::Failed, None, Some(format!("a view was not drawn: {why}")), seconds);
            }
        };
        seconds.render = views.iter().map(|v| v.drawn.picture.time.as_secs_f64()).sum();
        let times: Vec<(View, f64, f32)> =
            views.iter().map(|v| (v.drawn.picture.view, v.drawn.picture.time.as_secs_f64(), v.drawn.picture.exposure)).collect();
        let files: Vec<(Arc<Vec<u8>>, Option<Arc<Vec<u8>>>)> = views.iter().map(|v| (v.picture.clone(), v.depth.clone())).collect();
        let rendered = prepared.assemble(views.into_iter().map(|v| v.drawn).collect());
        let written = (|| -> Result<Value> {
            let job = self.job;
            let mut m = super::manifest(scene, title.clone(), inputs.clone(), &rendered, &job.iso, &job.out);
            m.resources = Some(resources.clone());
            m.key = Some(key.clone());
            m.quick_key = quick.clone();
            for (v, (pic, _)) in m.views.iter_mut().zip(&files) {
                v.bytes = Some(pic.len() as u64);
            }
            for (v, (pic, depth)) in m.views.iter().zip(&files) {
                super::write_file(&job.out.dir.join(&v.file), pic)?;
                if let (Some(name), Some(bytes)) = (&v.depth, depth) {
                    super::write_file(&job.out.dir.join(name), bytes)?;
                }
            }
            // (Through text, so its f32s keep their shortest form: 230.27417, not 230.27416992187500.)
            let mut value: Value = serde_json::from_slice(&serde_json::to_vec(&m)?)?;
            super::write_file(&job.out.dir.join(job.out.manifest(resref)), &serde_json::to_vec_pretty(&value)?)?;
            if let Some(cache) = &self.cache {
                // (In the cache the files are named by their view only, and the quick key is not part of the pictures.)
                let mut kept = value.clone();
                kept["quick_key"] = Value::Null;
                let mut cache_files = Vec::new();
                for (v, (pic, depth)) in kept["views"].as_array_mut().expect("a manifest written above has views").iter_mut().zip(&files) {
                    let name = v["view"].as_str().unwrap_or("view").to_string();
                    v["file"] = json!(format!("{name}.{}", job.out.format.ext()));
                    cache_files.push((format!("{name}.{}", job.out.format.ext()), pic.clone()));
                    if let Some(d) = depth {
                        v["depth"] = json!(format!("{name}.depth.png"));
                        cache_files.push((format!("{name}.depth.png"), d.clone()));
                    }
                }
                cache.put(&key, &kept, &cache_files)?;
                if let Some(q) = &quick {
                    cache.link(q, &key)?;
                }
            }
            value["quick_key"] = quick.map_or(Value::Null, Value::String);
            Ok(value)
        })();
        seconds.write = t.elapsed().as_secs_f64();
        match written {
            Ok(m) => AreaResult { views: times, ..self.result(resref, Action::Rendered, Some(m), None, seconds) },
            Err(e) => self.result(resref, Action::Failed, None, Some(format!("{e:#}")), seconds),
        }
    }
}

/// Puts file `from` at `to` (replacing it): a hard link where it can, else a copy; through a temporary name beside `to`.
fn place(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    let tmp = to.with_file_name(format!(".{}.part", to.file_name().map_or_default(|n| n.to_string_lossy())));
    let _ = std::fs::remove_file(&tmp);
    if std::fs::hard_link(from, &tmp).is_err() {
        std::fs::copy(from, &tmp).with_context(|| format!("copying {} to {}", from.display(), tmp.display()))?;
    }
    std::fs::rename(&tmp, to).with_context(|| format!("writing {}", to.display()))
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The run.

enum Task {
    Area(usize),
    View(Arc<ToDraw>, usize),
}

/// A task taken from the queue: when it is dropped, at the task's end or in a panic's unwinding, the count of tasks in flight goes
/// down, and when nothing is left the waiting threads are woken to end.
struct InFlight<'a> {
    queue: &'a Mutex<(VecDeque<Task>, usize)>,
    wake: &'a Condvar,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut q = self.queue.locked();
        q.1 -= 1;
        if q.1 == 0 && q.0.is_empty() {
            self.wake.notify_all();
        }
    }
}

/// Draws `areas` of `source` as `job` says, on `job.jobs` threads; `report` is called with each area's result as it is done (from
/// the thread that finished it). Returns the results in `areas`' order. With `job.index`, writes `manifest.jsonl` and `index.json`.
pub fn run(
    game: &Game,
    source: &dyn AreaSource,
    areas: &[String],
    job: &Job,
    files: Option<&FileDigests>,
    report: &(dyn Fn(&AreaResult) + Sync),
) -> Result<Vec<AreaResult>> {
    std::fs::create_dir_all(&job.out.dir).with_context(|| format!("making {}", job.out.dir.display()))?;
    let jobs = job.jobs.max(1);
    let batch = Batch {
        job,
        game,
        source,
        keys: KeyContext::new(game, job, files),
        cache: job.cache.as_deref().map(Cache::new),
        scene_threads: if areas.len() > 1 { 2.min(jobs) } else { jobs.min(8) },
    };
    let results: Mutex<Vec<Option<AreaResult>>> = Mutex::new(areas.iter().map(|_| None).collect());
    let queue: Mutex<(VecDeque<Task>, usize)> = Mutex::new(((0..areas.len()).map(Task::Area).collect(), 0));
    let wake = Condvar::new();
    // Which area each drawing belongs to, for its result's place.
    let place_of: Mutex<HashMap<String, usize>> = Mutex::new(HashMap::new());
    let done = |i: usize, r: AreaResult| {
        // (A report that panics, a progress line written to a closed stderr say, loses that line, not the area or the batch.)
        let _ = caught(|| report(&r));
        results.locked()[i] = Some(r);
    };
    let failed =
        |resref: &str, e: anyhow::Error, seconds: Seconds| batch.result(resref, Action::Failed, None, Some(format!("{e:#}")), seconds);
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let task = {
                        let mut q = queue.locked();
                        loop {
                            if let Some(t) = q.0.pop_front() {
                                q.1 += 1;
                                break Some(t);
                            }
                            if q.1 == 0 {
                                break None;
                            }
                            q = wake.wait(q).unwrap_or_else(PoisonError::into_inner);
                        }
                    };
                    let Some(task) = task else {
                        wake.notify_all();
                        return;
                    };
                    // (However the task ends, panic included, it leaves the count; else the other threads would wait for ever.)
                    let _in_flight = InFlight { queue: &queue, wake: &wake };
                    match task {
                        Task::Area(i) => match caught(|| batch.check(&areas[i])) {
                            Ok(Checked::Done(r)) => done(i, r),
                            Ok(Checked::Draw(d)) => {
                                let views = d.prepared.views().len();
                                place_of.locked().insert(d.resref.clone(), i);
                                let shared: Arc<ToDraw> = Arc::from(d);
                                let mut q = queue.locked();
                                for v in (0..views).rev() {
                                    q.0.push_front(Task::View(shared.clone(), v));
                                }
                                wake.notify_all();
                            }
                            Err(e) => done(i, failed(&areas[i], e, Seconds::default())),
                        },
                        Task::View(d, v) => {
                            if let Err(e) = caught(|| batch.draw_view(&d, v)).and_then(|r| r) {
                                d.errors.locked().push(format!("view {}: {e:#}", batch.job.iso.views[v].name()));
                            }
                            // (The last view of an area to be drawn writes it.)
                            if d.left.fetch_sub(1, Ordering::AcqRel) == 1 {
                                let i = place_of.locked().remove(&d.resref).unwrap_or(0);
                                let r = caught(|| batch.finish(&d)).unwrap_or_else(|e| failed(&d.resref, e, d.seconds));
                                done(i, r);
                            }
                        }
                    }
                }
            });
        }
    });
    if let Some(f) = files {
        f.save()?;
    }
    let results: Vec<AreaResult> = results.take().into_iter().map(|r| r.expect("an area was left out")).collect();
    if job.index {
        write_index(job, &results)?;
    }
    Ok(results)
}

/// `manifest.jsonl` (one line per area and view, in resref and view order, for a service or a website to read) and `index.json` (each
/// area's status and keys), from the manifests in the output folder.
pub fn write_index(job: &Job, results: &[AreaResult]) -> Result<()> {
    let mut sorted: Vec<&AreaResult> = results.iter().collect();
    sorted.sort_by(|a, b| a.resref.cmp(&b.resref));
    let mut lines = String::new();
    let mut index = Vec::new();
    for r in sorted {
        let manifest_file = job.out.manifest(&r.resref);
        // (An area that failed now keeps the pictures it had, when it had any.)
        let m: Option<Value> = r
            .manifest
            .clone()
            .or_else(|| std::fs::read(job.out.dir.join(&manifest_file)).ok().and_then(|b| serde_json::from_slice(&b).ok()));
        match &m {
            Some(m) => {
                for line in lines_of(m) {
                    lines.push_str(&line.to_string());
                    lines.push('\n');
                }
            }
            None => {
                for v in &job.iso.views {
                    lines.push_str(
                        &json!({ "resref": r.resref, "view": v, "version": super::VERSION, "status": "failed", "error": r.error })
                            .to_string(),
                    );
                    lines.push('\n');
                }
            }
        }
        index.push(json!({
            "resref": r.resref,
            "status": if r.action == Action::Failed { "failed" } else { "ok" },
            "manifest": m.as_ref().map(|_| manifest_file),
            "key": m.as_ref().map_or(Value::Null, |m| m["key"].clone()),
            "quick_key": m.as_ref().map_or(Value::Null, |m| m["quick_key"].clone()),
            "error": r.error,
        }));
    }
    // (Only when they change: a reader may watch manifest.jsonl's time to know when to read it again.)
    write_if_changed(&job.out.dir.join("manifest.jsonl"), lines.as_bytes())?;
    let index = json!({ "format": "nwn-bake/area-iso-index", "version": 1, "areas": index });
    write_if_changed(&job.out.dir.join("index.json"), &serde_json::to_vec_pretty(&index)?)?;
    Ok(())
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if std::fs::read(path).is_ok_and(|had| had == bytes) {
        return Ok(());
    }
    super::write_file(path, bytes)
}

/// The `manifest.jsonl` lines of an area's manifest: one per view, each with the fields a reader needs first (`resref`, `view`, `path`,
/// `width`, `height`, `bytes`, `status`, `transform` with `matrix`), the manifest's own fields, and the view's entry (`view_entry`).
pub fn lines_of(m: &Value) -> Vec<Value> {
    let mut base = serde_json::Map::new();
    for k in ["format", "version", "renderer", "area", "inputs", "resources", "key", "quick_key", "options", "roof_cut_m", "notes"] {
        base.insert(k.into(), m[k].clone());
    }
    m["views"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            let mut line = serde_json::Map::new();
            line.insert("resref".into(), m["area"]["resref"].clone());
            line.insert("view".into(), v["view"].clone());
            line.insert("path".into(), v["file"].clone());
            line.insert("width".into(), v["width"].clone());
            line.insert("height".into(), v["height"].clone());
            line.insert("bytes".into(), v["bytes"].clone());
            line.insert("status".into(), json!("ok"));
            line.insert("error".into(), Value::Null);
            let mut t = serde_json::Map::new();
            for k in ["view", "elevation_deg", "scale", "width", "height", "rotation", "offset", "matrix"] {
                t.insert(k.into(), v[k].clone());
            }
            line.insert("transform".into(), Value::Object(t));
            for (k, val) in &base {
                line.insert(k.clone(), val.clone());
            }
            line.insert("view_entry".into(), v.clone());
            Value::Object(line)
        })
        .collect()
}

/// An area's files read by path, for `serve`'s requests that give them instead of a module: `are` and `git` (binary or nwn_gff's
/// JSON; `git` may be left out).
pub fn area_from_files(are: &Path, git: Option<&Path>) -> Result<AreaFiles> {
    let name = are.file_name().map_or_default(|n| n.to_string_lossy().to_ascii_lowercase());
    let stem = name.strip_suffix(".json").unwrap_or(&name);
    let resref = stem.strip_suffix(".are").unwrap_or(stem).to_string();
    Ok(AreaFiles { resref, are: crate::module::read_gff(are)?, git: git.map(crate::module::read_gff).transpose()? })
}

// ---------------------------------------------------------------------------------------------------------------------------------
// serve's requests.

/// `serve`'s `area_iso` and `area_iso_key` requests (`docs/area-iso.md`, "serve"): an area, of a module (`module`) or by its files
/// (`are`, `git`, and `haks` and `tlk`, else the module's of the nearest `module.ifo`), the options (as `area-iso`'s, the
/// defaults the same), and where to write (`out`, laid out as `--all` lays it out unless `layout` is `flat`) and keep (`cache`).
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaRequest {
    /// The module (any form [`ModuleSource::open`] takes); give it with `area`, or give `are` instead.
    pub module: Option<PathBuf>,
    /// The area's resref: required with `module`; with `are`, checked against the file's.
    pub area: Option<String>,
    /// The area's `.are` (binary or nwn_gff's JSON), in place of `module`.
    pub are: Option<PathBuf>,
    /// The area's `.git`; by default the `.git` (or `.git.json`) beside the `.are`.
    pub git: Option<PathBuf>,
    /// The haks, first highest; by default the module's.
    pub haks: Option<Vec<String>>,
    /// The custom talk table, a name or a path; by default the module's.
    pub tlk: Option<String>,
    /// A list (`["sw", "top"]`) or as `--view` takes it (`"all,top"`).
    pub views: Option<Value>,
    /// Pixels a metre ([`IsoOptions::scale`]), more than 0.
    pub scale: Option<f32>,
    /// Supersampling, 1 to 4 ([`IsoOptions::quality`]).
    pub quality: Option<u8>,
    /// `"day"` or `"area"` ([`super::Light`]).
    pub light: Option<String>,
    /// Draw shadows ([`IsoOptions::shadows`]).
    pub shadows: Option<bool>,
    /// `"auto"`, `"keep"` or metres.
    pub roof: Option<Value>,
    /// `"auto"` or `"off"` (or true or false).
    pub exposure: Option<Value>,
    /// Draw the `.git`'s placeables and doors (true when left out).
    pub objects: Option<bool>,
    /// `"transparent"` or a colour (`"#rrggbb"`).
    pub background: Option<String>,
    /// Also write depth pictures ([`IsoOptions::depth`]).
    pub depth: Option<bool>,
    /// `"webp"` or `"png"`; WebP by default when this build can write it.
    pub format: Option<String>,
    /// Calibration constants that differ from the default ([`super::Look`]'s fields).
    pub look: Option<Value>,
    /// The output folder; without it nothing is written and only the keys are worked out.
    pub out: Option<PathBuf>,
    /// `tree` (the default: `webp/<view>/<area>.webp`, `json/<area>.json`) or `flat` (side by side).
    pub layout: Option<String>,
    /// The cache folder.
    pub cache: Option<PathBuf>,
    /// Draw the area whatever its keys say.
    #[serde(default)]
    pub force: bool,
    /// A hak not found fails the area.
    #[serde(default)]
    pub strict: bool,
}

impl AreaRequest {
    /// The job it asks for (with no `out`, a folder that is never written: only keys are asked for).
    pub fn job(&self, tlk: Option<String>) -> Result<Job> {
        let mut o = IsoOptions::default();
        if let Some(v) = &self.views {
            o.views = match v {
                Value::String(s) => View::parse_list(s)?,
                Value::Array(a) => {
                    View::parse_list(&a.iter().map(|x| x.as_str().unwrap_or("?").to_string()).collect::<Vec<_>>().join(","))?
                }
                _ => bail!("views is a list of views"),
            };
        }
        if let Some(s) = self.scale {
            anyhow::ensure!(s.is_finite() && s > 0.0, "scale is pixels a metre, more than 0");
            o.scale = s;
        }
        if let Some(q) = self.quality {
            anyhow::ensure!((1..=4).contains(&q), "quality is 1 to 4");
            o.quality = q;
        }
        if let Some(l) = &self.light {
            o.light = match l.as_str() {
                "day" => super::Light::Day,
                "area" => super::Light::Area,
                _ => bail!("light is day or area"),
            };
        }
        if let Some(sh) = self.shadows {
            o.shadows = sh;
        }
        if let Some(r) = &self.roof {
            o.roof = match r {
                Value::String(s) if s == "auto" => super::Roof::Auto,
                Value::String(s) if s == "keep" => super::Roof::Keep,
                Value::Number(n) => super::Roof::Over(n.as_f64().unwrap_or(0.0) as f32),
                _ => bail!("roof is auto, keep or metres"),
            };
        }
        if let Some(e) = &self.exposure {
            o.exposure = match e {
                Value::Bool(b) => *b,
                Value::String(s) if s == "auto" => true,
                Value::String(s) if s == "off" => false,
                _ => bail!("exposure is auto or off"),
            };
        }
        if let Some(b) = &self.background {
            o.background = super::parse_background(b)?;
        }
        o.depth = self.depth.unwrap_or(false);
        if let Some(l) = &self.look {
            o.look = serde_json::from_value(l.clone()).context("look: not a look")?;
        }
        let format = match &self.format {
            Some(f) => super::Format::parse(f)?,
            None => super::Format::best(),
        };
        let dir = self.out.clone().unwrap_or_default();
        let out = match self.layout.as_deref() {
            None | Some("tree") => OutOptions::tree(dir, format),
            Some("flat") => OutOptions { format, ..OutOptions::new(dir) },
            Some(l) => bail!("layout is tree or flat, not {l:?}"),
        };
        Ok(Job {
            iso: o,
            objects: self.objects.unwrap_or(true),
            out,
            cache: self.cache.clone(),
            force: self.force,
            strict: self.strict,
            jobs: 1,
            index: false,
            tlk,
        })
    }
}

/// Answers `serve`'s area requests. The game is opened once per hak list and talk table and kept (the four used last), each in the
/// one caches' budget [`Service::cache`]; the module is read again for each request, so a change to its files is seen at once; the
/// haks' digests are kept per cache folder.
pub struct Service {
    /// The game install; found as the game finds it when None.
    pub root: Option<PathBuf>,
    /// The user folder (`hak/`, `tlk/`, `override/`).
    pub user: Option<PathBuf>,
    /// Threads per request (views drawn at once).
    pub jobs: usize,
    /// The caches' budget every game it opens shares ([`crate::game::open_in`]): the limits hold for all of them together. Its own,
    /// at the default limits, unless set; `serve` gives it the one its character game has.
    pub cache: crate::game::CacheBudget,
    games: Mutex<Vec<((Vec<String>, Tlk), Arc<Game>)>>,
    files: Mutex<HashMap<Option<PathBuf>, Arc<FileDigests>>>,
}

impl Service {
    /// A service for the install `root` and the user folder `user`, drawing on [`default_jobs`] threads a request.
    pub fn new(root: Option<PathBuf>, user: Option<PathBuf>) -> Service {
        Service {
            root,
            user,
            jobs: default_jobs().max(1),
            cache: crate::game::CacheBudget::default(),
            games: Default::default(),
            files: Default::default(),
        }
    }

    fn game(&self, haks: Vec<String>, tlk: Tlk) -> Result<Arc<Game>> {
        let key = (haks, tlk);
        if let Some((_, g)) = self.games.locked().iter().find(|(k, _)| *k == key) {
            return Ok(g.clone());
        }
        let g = Arc::new(crate::game::open_in(
            &crate::game::GameOptions {
                root: self.root.clone(),
                user: self.user.clone(),
                haks: crate::game::Haks::List(key.0.clone()),
                tlk: key.1.clone(),
            },
            &self.cache,
        )?);
        let mut games = self.games.locked();
        // (A few hak lists at most; the oldest goes past four.)
        if games.len() >= 4 {
            games.remove(0);
        }
        games.push((key, g.clone()));
        Ok(g)
    }

    fn digests(&self, cache: Option<&Path>) -> Arc<FileDigests> {
        let key = cache.map(Path::to_path_buf);
        self.files
            .locked()
            .entry(key.clone())
            .or_insert_with(|| Arc::new(FileDigests::at(key.map(|c| c.join("v1").join("files.json")))))
            .clone()
    }

    /// What a request reads its area from, the area's resref, the haks and the talk table.
    fn source(&self, r: &AreaRequest) -> Result<(Box<dyn AreaSource>, String, Vec<String>, Tlk)> {
        match (&r.module, &r.are) {
            (Some(m), None) => {
                let area = r.area.clone().context("give the area")?.to_ascii_lowercase();
                let source = ModuleSource::open(m)?;
                let haks = r.haks.clone().unwrap_or_else(|| source.haks().to_vec());
                let tlk = Tlk::given_or_module(r.tlk.clone(), source.tlk().map(str::to_string));
                Ok((Box::new(source), area, haks, tlk))
            }
            (None, Some(are)) => {
                // (The haks and talk table, when not given, of the nearest module.ifo above the area, as for an area file.)
                let near = ModuleSource::open(are)?;
                let haks = r.haks.clone().unwrap_or_else(|| near.haks().to_vec());
                let tlk = Tlk::given_or_module(r.tlk.clone(), near.tlk().map(str::to_string));
                let paths = AreaPaths { are: are.clone(), git: r.git.clone().or_else(|| sibling_git(are)) };
                let area = area_from_files(&paths.are, None)?.resref;
                if let Some(a) = &r.area {
                    anyhow::ensure!(a.eq_ignore_ascii_case(&area), "{} is area {area}, not {a}", are.display());
                }
                Ok((Box::new(paths), area, haks, tlk))
            }
            _ => bail!("give module (and area), or are (and git)"),
        }
    }

    /// `area_iso`: the area drawn unless the output folder or the cache has its pictures; answers its manifest, with `action`
    /// (`current`, `cached` or `rendered`) and `manifest_path` added.
    pub fn area_iso(&self, r: &AreaRequest) -> Result<Value> {
        anyhow::ensure!(r.out.is_some(), "give out, the folder to write to");
        let (source, area, haks, tlk) = self.source(r)?;
        let game = self.game(haks, tlk.clone())?;
        let mut job = r.job(tlk.recorded(&game))?;
        // (Each view on a thread of its own, as many at once as there are cores.)
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        job.jobs = job.iso.views.len().clamp(1, cores);
        let files = self.digests(job.cache.as_deref());
        let results = run(&game, source.as_ref(), std::slice::from_ref(&area), &job, Some(&files), &|_| {})?;
        let r = results.into_iter().next().context("no result")?;
        match r.manifest {
            Some(mut m) => {
                m["action"] = serde_json::to_value(r.action)?;
                m["manifest_path"] = json!(r.manifest_path);
                Ok(m)
            }
            None => bail!("{}", r.error.unwrap_or_else(|| "failed".into())),
        }
    }

    /// `area_iso_key`: the area's quick and exact keys (the exact one builds its scene, nothing is drawn), whether the cache has
    /// pictures for it, and whether the output folder (when `out` is given) has them.
    pub fn area_iso_key(&self, r: &AreaRequest) -> Result<Value> {
        let (source, area, haks, tlk) = self.source(r)?;
        let game = self.game(haks, tlk.clone())?;
        let job = r.job(tlk.recorded(&game))?;
        let files = self.digests(job.cache.as_deref());
        let batch = Batch {
            job: &job,
            game: &game,
            source: source.as_ref(),
            keys: KeyContext::new(&game, &job, Some(&files)),
            cache: job.cache.as_deref().map(Cache::new),
            scene_threads: self.jobs.min(8),
        };
        let a = batch.source.area(&area)?;
        let title = a.are.root.locstring("Name").and_then(|n| game.locstring(n)).filter(|n| !n.trim().is_empty());
        let inputs = Inputs::of(&a, game.haks(), game.missing_haks(), job.tlk.as_deref())?;
        let quick = batch.keys.quick(&a.resref, &inputs);
        let scene = crate::area::scene(&game, &a, &SceneOptions { objects: job.objects, threads: batch.scene_threads })?;
        let resources = game.resources_digest(&scene.reads);
        let key = batch.keys.exact(&a.resref, &inputs, title.as_deref(), &resources);
        let _ = files.save();
        let cached = batch.cache.as_ref().is_some_and(|c| c.get(&key).is_some());
        let current = r.out.is_some() && batch.in_out(&a.resref).is_some_and(|m| m["key"].as_str() == Some(&key));
        Ok(json!({ "area": a.resref, "quick_key": quick, "key": key, "resources": resources, "cached": cached, "current": current }))
    }
}

/// `x.git` beside `x.are`, `x.git.json` beside `x.are.json`.
fn sibling_git(are: &Path) -> Option<PathBuf> {
    let name = are.file_name()?.to_string_lossy().into_owned();
    let lower = name.to_ascii_lowercase();
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "`lower` is lower case already")]
    let git = if lower.ends_with(".are.json") {
        format!("{}.git.json", &name[..name.len() - ".are.json".len()])
    } else if lower.ends_with(".are") {
        format!("{}.git", &name[..name.len() - ".are".len()])
    } else {
        return None;
    };
    Some(are.with_file_name(git)).filter(|p| p.is_file())
}
