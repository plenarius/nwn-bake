// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Opening the game: the install, the user folder laid over it, a module's haks in the module's order, and the custom talk table.
//!
//! One way for everything that reads the game (characters, placeables, areas and the tools built on this crate). Hak order matters:
//! as in the game, the first hak in a module's `Mod_HakList` wins ([`mg_resman::ResMan::add_haks`] gives the first listed the highest
//! priority).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, ensure};
use mg_resman::GameInstall;
use mg_rules::GameData;

use crate::sync::Recover;

/// Which haks to lay over the game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Haks {
    /// None: the install and the user folder's `override/` and `development/` only.
    #[default]
    None,
    /// Every `user/hak/*.hak`, by name (what the character baker has always done, for a user folder that holds one module's haks).
    AllInUser,
    /// A module's `Mod_HakList`, first highest; each looked up in the user folder's `hak/`, then the install's `data/hk/`.
    List(Vec<String>),
}

/// Which custom talk table (the module's own, `Mod_CustomTlk`) to read. A file that is chosen and cannot be read as a talk table
/// is an error in every case: the game is never opened with a talk table other than the one asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Tlk {
    /// None: only the game's `dialog.tlk`.
    None,
    /// The one `.tlk` in the user folder's `tlk/` or `hak/` that is not the game's own, when there is exactly one; else none.
    #[default]
    Auto,
    /// A name in the user folder's `tlk/` or `hak/` (`.tlk` may be left out, as a module names it), or a path. Not finding it is
    /// an error: someone asked for it.
    Named(String),
    /// A module's `Mod_CustomTlk`, looked up as `Named`; when it is not there, none (a module's pictures do not need its names).
    Module(String),
}

impl Tlk {
    /// `Named` for a name that is not blank, else `Auto`.
    pub fn named_or_auto(name: Option<String>) -> Tlk {
        match name.filter(|n| !n.trim().is_empty()) {
            Some(n) => Tlk::Named(n),
            None => Tlk::Auto,
        }
    }

    /// `Named` for `given` (a name someone asked for), else `Module` for the module's `Mod_CustomTlk`, else `Auto`. Blank names
    /// count as none.
    pub fn given_or_module(given: Option<String>, module: Option<String>) -> Tlk {
        let blank = |n: &String| n.trim().is_empty();
        match (given.filter(|n| !blank(n)), module.filter(|n| !blank(n))) {
            (Some(n), _) => Tlk::Named(n),
            (None, Some(m)) => Tlk::Module(m),
            (None, None) => Tlk::Auto,
        }
    }

    /// The name a manifest records for this choice, once `game` is opened with it: the name asked for, when its file was read; None
    /// for `None`, for `Auto` and for a module's talk table that is not there.
    pub fn recorded(&self, game: &Game) -> Option<String> {
        match self {
            Tlk::Named(n) | Tlk::Module(n) if game.tlk_path().is_some() => Some(n.clone()),
            _ => None,
        }
    }
}

/// How to open the game.
#[derive(Clone, Debug, Default)]
pub struct GameOptions {
    /// The install; found as the game finds it (`NWN_ROOT`, then the usual places) when None.
    pub root: Option<PathBuf>,
    /// The user folder (`hak/`, `tlk/`, `override/`); found as the game finds it (`NWN_HOME`, then the usual place) when None.
    pub user: Option<PathBuf>,
    /// Which haks to lay over the game.
    pub haks: Haks,
    /// Which custom talk table to read.
    pub tlk: Tlk,
}

/// The game, opened: its data, the haks that were asked for and found nowhere, and what is read from it more than once (flattened
/// models, decoded textures, the digests of resources), shared by everything that uses this `Game`.
pub struct Game {
    /// The game's data: its resource manager (the install, with the haks over it) and its talk tables.
    pub data: GameData,
    install: GameInstall,
    /// The haks asked for, first highest.
    haks: Vec<String>,
    missing: Vec<String>,
    /// The custom talk table read, if any.
    tlk: Option<PathBuf>,
    flats: Cache<crate::Flat>,
    textures: Cache<crate::texture::Mipped>,
    budget: CacheBudget,
    /// SHA-256 of what a lookup of the resources finds ([`Game::resources_digest`]); None when it finds nothing.
    digests: Mutex<HashMap<(String, &'static str), Option<[u8; 32]>>>,
}

/// What a cache entry weighs, roughly in bytes, for [`Game::set_cache_limits`].
pub trait Weigh {
    /// Roughly how many bytes this keeps in memory.
    fn weight(&self) -> usize;
}

impl Weigh for crate::Flat {
    fn weight(&self) -> usize {
        self.positions.len() * 12
            + self.uvs.len() * 8
            + self.groups.iter().map(|g| g.0.len() + 24).sum::<usize>()
            + self.footprint.len() * 24
            + 128
    }
}

impl Weigh for crate::texture::Mipped {
    fn weight(&self) -> usize {
        self.levels.iter().map(|l| l.rgba.len()).sum::<usize>() + 128
    }
}

/// How much the caches of flattened models and decoded textures may keep between uses, roughly in bytes. What is in use is kept by
/// whoever uses it, whatever the limits; they bound only what is kept for the next time it is asked for. They never change what is
/// drawn or a key: an entry dropped is made again, the same, when it is asked for again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheLimits {
    /// Flattened models.
    pub flats: usize,
    /// Decoded textures.
    pub textures: usize,
}

impl Default for CacheLimits {
    /// [`FLAT_CACHE`] and [`TEXTURE_CACHE`].
    fn default() -> CacheLimits {
        CacheLimits { flats: FLAT_CACHE, textures: TEXTURE_CACHE }
    }
}

impl CacheLimits {
    /// `total` bytes, split as the defaults are: a quarter for models, the rest for textures. 0 keeps as little as can be (the
    /// entry made last of each kind).
    pub fn total(total: usize) -> CacheLimits {
        let flats = total / 4;
        CacheLimits { flats, textures: total - flats }
    }

    /// `mib` MiB (2^20 bytes) in all, split as [`CacheLimits::total`] does; None when that many bytes do not fit in a `usize`.
    pub fn total_mib(mib: u64) -> Option<CacheLimits> {
        usize::try_from(mib).ok().and_then(|m| m.checked_mul(1 << 20)).map(CacheLimits::total)
    }

    /// The two limits together, in bytes.
    pub fn sum(&self) -> usize {
        self.flats.saturating_add(self.textures)
    }
}

/// The caches' budget: one pool of flattened models and one of decoded textures, each with its limit, shared by every [`Game`]
/// opened with it ([`open_in`]). The limits bound what all those `Game`s keep together, the entries used least recently dropped
/// first, whichever `Game` they belong to; so a program that opens several (`serve` opens one per hak list) keeps no more than one
/// would. [`open`] gives each `Game` a budget of its own, with the default limits. Cloning it shares it.
#[derive(Clone)]
pub struct CacheBudget {
    flats: Arc<Pool<crate::Flat>>,
    textures: Arc<Pool<crate::texture::Mipped>>,
}

impl std::fmt::Debug for CacheBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheBudget").field("limits", &self.limits()).field("kept", &self.kept()).finish()
    }
}

impl Default for CacheBudget {
    /// A budget with the default limits ([`CacheLimits::default`]).
    fn default() -> CacheBudget {
        CacheBudget::new(CacheLimits::default())
    }
}

impl CacheBudget {
    /// A budget with these limits, shared by no `Game` yet.
    pub fn new(limits: CacheLimits) -> CacheBudget {
        CacheBudget { flats: Arc::new(Pool::new(limits.flats)), textures: Arc::new(Pool::new(limits.textures)) }
    }

    /// The limits.
    pub fn limits(&self) -> CacheLimits {
        CacheLimits { flats: self.flats.locked().limit, textures: self.textures.locked().limit }
    }

    /// Changes the limits for every `Game` that shares this budget; what is kept beyond them is dropped now.
    pub fn set_limits(&self, limits: CacheLimits) {
        let mut f = self.flats.locked();
        f.limit = limits.flats;
        f.trim(None);
        drop(f);
        let mut t = self.textures.locked();
        t.limit = limits.textures;
        t.trim(None);
    }

    /// What the pools keep now, roughly in bytes (models, textures).
    pub fn kept(&self) -> CacheLimits {
        CacheLimits { flats: self.flats.locked().weight, textures: self.textures.locked().weight }
    }

    /// Drops everything kept, for every `Game` that shares this budget (whoever holds an entry keeps it).
    pub fn clear(&self) {
        self.flats.locked().retain(|_| false);
        self.textures.locked().retain(|_| false);
    }
}

/// One kind of cache, for every `Game` of a budget: results by `Game` and lower-case name; a failure is kept as its message, so it is
/// not tried again. When what is kept weighs more than the limit, the entries used least recently are dropped (whoever holds one
/// keeps it; it is made again when asked for again), so a batch of a whole module does not keep every model and texture it ever read.
struct Pool<T> {
    inner: Mutex<PoolInner<T>>,
}

struct PoolInner<T> {
    map: HashMap<(u64, String), Entry<T>>,
    tick: u64,
    weight: usize,
    limit: usize,
}

struct Entry<T> {
    value: Result<Arc<T>, String>,
    used: u64,
    weight: usize,
}

impl<T> Pool<T> {
    fn new(limit: usize) -> Pool<T> {
        Pool { inner: Mutex::new(PoolInner { map: HashMap::new(), tick: 0, weight: 0, limit }) }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, PoolInner<T>> {
        self.inner.locked()
    }
}

impl<T> PoolInner<T> {
    /// Keeps the entries `keep` says yes to.
    fn retain(&mut self, mut keep: impl FnMut(&(u64, String)) -> bool) {
        let mut dropped = 0;
        self.map.retain(|k, e| {
            let kept = keep(k);
            if !kept {
                dropped += e.weight;
            }
            kept
        });
        self.weight -= dropped;
    }

    /// When what is kept weighs more than the limit, drops the entries used least recently but `spare` down to nine tenths of it
    /// (so this is not done again at the next entry).
    fn trim(&mut self, spare: Option<&(u64, String)>) {
        if self.weight <= self.limit {
            return;
        }
        let mut by_age: Vec<(u64, (u64, String))> =
            self.map.iter().filter(|(k, _)| Some(*k) != spare).map(|(k, e)| (e.used, k.clone())).collect();
        by_age.sort_unstable();
        let goal = self.limit / 10 * 9;
        for (_, k) in by_age {
            if self.weight <= goal {
                break;
            }
            if let Some(e) = self.map.remove(&k) {
                self.weight -= e.weight;
            }
        }
    }
}

/// A `Game`'s share of a pool: its entries, under an id no other `Game` has. Dropping it drops them.
struct Cache<T> {
    pool: Arc<Pool<T>>,
    id: u64,
}

impl<T> Cache<T> {
    fn new(pool: Arc<Pool<T>>) -> Cache<T> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Cache { pool, id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
    }

    fn clear(&self) {
        let id = self.id;
        self.pool.locked().retain(|k| k.0 != id);
    }
}

impl<T> Drop for Cache<T> {
    fn drop(&mut self) {
        self.clear();
    }
}

/// `name`'s entry in `cache`, made by `make` the first time. (Made outside the lock: two threads asking at once may both make it,
/// and the first one kept wins. The result is the same either way.)
fn cached<T: Weigh>(cache: &Cache<T>, name: &str, make: impl FnOnce(&str) -> Result<T>) -> Result<Arc<T>> {
    let key = (cache.id, name.to_ascii_lowercase());
    {
        let mut c = cache.pool.locked();
        c.tick += 1;
        let tick = c.tick;
        if let Some(e) = c.map.get_mut(&key) {
            e.used = tick;
            return e.value.clone().map_err(anyhow::Error::msg);
        }
    }
    let made = make(&key.1).map(Arc::new).map_err(|e| format!("{e:#}"));
    let mut c = cache.pool.locked();
    c.tick += 1;
    let tick = c.tick;
    if let Some(e) = c.map.get_mut(&key) {
        e.used = tick;
        return e.value.clone().map_err(anyhow::Error::msg);
    }
    let weight = made.as_ref().map_or(64, |v| v.weight());
    c.weight += weight;
    c.map.insert(key.clone(), Entry { value: made.clone(), used: tick, weight });
    c.trim(Some(&key));
    made.map_err(anyhow::Error::msg)
}

impl std::ops::Deref for Game {
    type Target = GameData;
    fn deref(&self) -> &GameData {
        &self.data
    }
}

/// How much the cache of flattened models keeps by default, roughly in bytes: 1 GiB.
pub const FLAT_CACHE: usize = 1 << 30;
/// How much the cache of decoded textures keeps by default, roughly in bytes: 3 GiB.
pub const TEXTURE_CACHE: usize = 3 << 30;

/// Opens the game as `o` says, with caches of its own at the default limits ([`CacheLimits::default`]).
pub fn open(o: &GameOptions) -> Result<Game> {
    open_in(o, &CacheBudget::default())
}

/// Opens the game as `o` says, its caches kept in `budget`'s pools, within its limits together with every other `Game` that shares
/// it.
pub fn open_in(o: &GameOptions, budget: &CacheBudget) -> Result<Game> {
    let mut install = match &o.root {
        Some(root) => {
            anyhow::ensure!(root.is_dir(), "{}: no such folder (the game install)", root.display());
            GameInstall::new(root.clone(), o.user.clone().or_else(GameInstall::detect_user_dir), "en")
        }
        None => {
            let mut gi = GameInstall::detect().context("no game install found: give its folder with --root, or set NWN_ROOT")?;
            if o.user.is_some() {
                gi.user_dir = o.user.clone();
            }
            gi
        }
    };
    install.language = language(&install.root)?;
    let mut data = GameData::open(&install).with_context(|| format!("opening the game in {}", install.root.display()))?;
    let names: Vec<String> = match &o.haks {
        Haks::None => Vec::new(),
        Haks::AllInUser => all_in_user(install.user_dir.as_deref())?,
        Haks::List(list) => list.clone(),
    };
    let missing = data.resman.add_haks(&install, &names)?;
    let user = install.user_dir.as_deref();
    let tlk = match &o.tlk {
        Tlk::None => None,
        Tlk::Auto => user.and_then(|u| crate::custom_tlk_path(u, None)),
        Tlk::Named(name) => Some(find_tlk(user, name).with_context(|| match user {
            Some(u) => {
                format!("talk table {name}: not found in {} or {}, and not a file", u.join("tlk").display(), u.join("hak").display())
            }
            None => format!("talk table {name}: not a file, and there is no user folder to look in"),
        })?),
        Tlk::Module(name) => find_tlk(user, name),
    };
    // (Only a file that was read is kept: `tlk_path` goes into the keys and the manifests.)
    if let Some(p) = &tlk {
        let bytes = std::fs::read(p).with_context(|| format!("reading the talk table {}", p.display()))?;
        let read = mg_tlk::Tlk::read(&bytes).map_err(|e| anyhow::anyhow!("{}: not a talk table: {e}", p.display()))?;
        data.set_custom_tlk(Some(read));
    }
    Ok(Game {
        data,
        install,
        haks: names,
        missing,
        tlk,
        flats: Cache::new(budget.flats.clone()),
        textures: Cache::new(budget.textures.clone()),
        budget: budget.clone(),
        digests: Default::default(),
    })
}

impl Game {
    /// Haks that were asked for and found nowhere (the game is opened without them).
    pub fn missing_haks(&self) -> &[String] {
        &self.missing
    }

    /// The haks asked for, first highest (found or not).
    pub fn haks(&self) -> &[String] {
        &self.haks
    }

    /// The custom talk table's file, when one was read.
    pub fn tlk_path(&self) -> Option<&Path> {
        self.tlk.as_deref()
    }

    /// The install and user folder the game was opened from.
    pub fn install(&self) -> &GameInstall {
        &self.install
    }

    /// How much the caches of flattened models and decoded textures may keep, roughly in bytes ([`FLAT_CACHE`] and
    /// [`TEXTURE_CACHE`] by default). Only the weight kept between uses: what is in use is kept by whoever uses it. The limits are
    /// the budget's ([`Game::cache_budget`]), so they change for every `Game` that shares it.
    pub fn set_cache_limits(&mut self, flats: usize, textures: usize) {
        self.budget.set_limits(CacheLimits { flats, textures });
    }

    /// The budget this `Game`'s caches are kept in ([`open_in`]).
    pub fn cache_budget(&self) -> &CacheBudget {
        &self.budget
    }

    /// Lays more haks under the ones already there (each lower than every hak before it); returns those found nowhere (also kept in
    /// [`Game::missing_haks`]).
    pub fn add_haks(&mut self, names: &[String]) -> Result<Vec<String>> {
        let missing = self.data.resman.add_haks(&self.install, names)?;
        // (What was read may now come from another layer.)
        self.flats.clear();
        self.textures.clear();
        self.digests.inner_mut().clear();
        self.haks.extend(names.iter().cloned());
        self.missing.extend(missing.iter().cloned());
        Ok(missing)
    }
}

impl Game {
    /// Model `resref` flattened at rest ([`crate::flatten`]), read once per `Game`.
    pub fn flat(&self, resref: &str) -> Result<Arc<crate::Flat>> {
        cached(&self.flats, resref, |r| crate::flatten(&self.data, r))
    }

    /// The image texture name `name` means, decoded with its mips ([`crate::texture::mipped`]), read once per `Game`.
    pub fn mipped(&self, name: &str) -> Result<Arc<crate::texture::Mipped>> {
        cached(&self.textures, name, |n| crate::texture::mipped(&self.data, n))
    }

    /// The SHA-256 of what looking resource `name` up as `kind` finds, None when it finds nothing; worked out once per `Game`.
    /// `kind` is a [`ResKind`]'s name.
    fn lookup_digest(&self, name: &str, kind: ResKind) -> Option<[u8; 32]> {
        use sha2::Digest;
        let key = (name.to_ascii_lowercase(), kind.name());
        if let Some(d) = self.digests.locked().get(&key) {
            return *d;
        }
        let found: Option<[u8; 32]> = match kind {
            ResKind::Texture => mg_core::ResRef::cut(&key.0).and_then(|r| self.data.resman.texture(r)).map(|(t, bytes)| {
                let mut h = sha2::Sha256::new();
                h.update(t.extension().unwrap_or("?").as_bytes());
                h.update([0]);
                h.update(&bytes);
                h.finalize().into()
            }),
            _ => self.data.resman.get_named(&key.0, kind.restype()).ok().map(|bytes| sha2::Sha256::digest(&bytes).into()),
        };
        self.digests.locked().insert(key, found);
        found
    }

    /// One digest of `reads` ([`crate::area::Scene::reads`]): for each, in order, its name, its kind and the SHA-256 of what the
    /// game finds for it, or that it finds nothing. The same lookups finding the same bytes give the same digest, whichever hak or
    /// folder they come from. `sha256:` and hex.
    pub fn resources_digest(&self, reads: &[(String, ResKind)]) -> String {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(b"nwn-bake/resources 1\0");
        for (name, kind) in reads {
            h.update(name.as_bytes());
            h.update([0]);
            h.update(kind.name().as_bytes());
            h.update([0]);
            match self.lookup_digest(name, *kind) {
                Some(d) => {
                    h.update([1]);
                    h.update(d);
                }
                None => h.update([0]),
            }
        }
        format!("sha256:{:x}", h.finalize())
    }
}

/// How a scene looks a resource up: by its type, or as a texture (a DDS or TGA, whichever the game's rule finds first,
/// [`mg_resman::ResMan::texture`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResKind {
    /// A tileset (`.set`).
    Set,
    /// A 2da table.
    TwoDa,
    /// A model (`.mdl`).
    Mdl,
    /// A tile's walkmesh (`.wok`).
    Wok,
    /// A placeable's or door's walkmesh (`.pwk`).
    Pwk,
    /// A material (`.mtr`).
    Mtr,
    /// A texture: a DDS or a TGA, whichever [`mg_resman::ResMan::texture`] finds first.
    Texture,
}

impl ResKind {
    /// The kind's name in the resource digests (`set`, `2da`, `mdl`, `wok`, `pwk`, `mtr`, `texture`). It is part of what is hashed, so
    /// changing one changes every digest.
    pub fn name(self) -> &'static str {
        match self {
            ResKind::Set => "set",
            ResKind::TwoDa => "2da",
            ResKind::Mdl => "mdl",
            ResKind::Wok => "wok",
            ResKind::Pwk => "pwk",
            ResKind::Mtr => "mtr",
            ResKind::Texture => "texture",
        }
    }

    fn restype(self) -> mg_core::ResType {
        use mg_core::ResType;
        match self {
            ResKind::Set => ResType::SET,
            ResKind::TwoDa => ResType::TWODA,
            ResKind::Mdl => ResType::MDL,
            ResKind::Wok => ResType::WOK,
            ResKind::Pwk => ResType::PWK,
            ResKind::Mtr => ResType::MTR,
            // (Looked up by `ResMan::texture`, never by type.)
            ResKind::Texture => ResType::DDS,
        }
    }
}

/// The custom talk table `name` means: a path that is absolute, a file in the user folder's `tlk/` or `hak/` (with `.tlk` added
/// when the name has no extension, as `Mod_CustomTlk` gives it), else a path relative to the working folder. None when none is a file.
fn find_tlk(user: Option<&Path>, name: &str) -> Option<PathBuf> {
    let as_path = Path::new(name);
    if as_path.is_absolute() {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }
    user.and_then(|u| crate::custom_tlk_path(u, Some(name))).or_else(|| as_path.is_file().then(|| as_path.to_path_buf()))
}

/// The language folder the install is read in: `en` when the install has `lang/en/data/dialog.tlk`, else the first `lang/<xx>`
/// (by name) that has one, so a French or German install opens in its own language.
fn language(root: &Path) -> Result<String> {
    let has = |lang: &str| root.join("lang").join(lang).join("data").join("dialog.tlk").is_file();
    if has("en") {
        return Ok("en".to_string());
    }
    let mut langs: Vec<String> = std::fs::read_dir(root.join("lang"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|l| has(l))
        .collect();
    langs.sort();
    langs.into_iter().next().with_context(|| {
        // (Moonglow tells the usual mistakes apart: the original game, a folder inside the game's, one above it.)
        let why = GameInstall::why_not(root).unwrap_or_else(|| "no lang/<language>/data/dialog.tlk in it".to_string());
        format!("{}: not an NWN:EE install: {why}", root.display())
    })
}

/// Every `*.hak` in `user/hak`, by name; none when the user folder has no `hak/`.
fn all_in_user(user: Option<&Path>) -> Result<Vec<String>> {
    let user = user.context("no user folder: give --user, or set NWN_BAKE_USER or NWN_HOME")?;
    ensure!(user.is_dir(), "the user folder {} is not a folder", user.display());
    let entries = match std::fs::read_dir(user.join("hak")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", user.join("hak").display())),
    };
    let mut haks: Vec<String> = entries
        .filter_map(|e| {
            let p = e.ok()?.path();
            p.extension()?.eq_ignore_ascii_case("hak").then(|| p.file_stem().map(|s| s.to_string_lossy().into_owned())).flatten()
        })
        .collect();
    haks.sort();
    Ok(haks)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cache entry that weighs what it says.
    struct Lump(usize);

    impl Weigh for Lump {
        fn weight(&self) -> usize {
            self.0
        }
    }

    #[test]
    fn one_budget_bounds_every_cache_in_it_together() {
        let pool = Arc::new(Pool::<Lump>::new(100));
        let (a, b) = (Cache::new(pool.clone()), Cache::new(pool.clone()));
        let made = std::cell::Cell::new(0);
        let get = |c: &Cache<Lump>, name: &str| {
            cached(c, name, |_| {
                made.set(made.get() + 1);
                Ok(Lump(30))
            })
            .unwrap()
        };
        // The same name in two caches is two entries; three fit, the fourth drops the oldest, whichever cache it is in.
        get(&a, "x");
        get(&b, "x");
        get(&a, "y");
        assert_eq!((pool.locked().weight, made.get()), (90, 3));
        get(&b, "y");
        let weight = pool.locked().weight;
        assert!(weight <= 100, "{weight}");
        assert!(!pool.locked().map.contains_key(&(a.id, "x".to_string())), "a's x was used least recently");
        get(&a, "x");
        assert_eq!(made.get(), 5, "made again when asked for again");
        // A cache dropped takes its entries with it.
        drop(a);
        let p = pool.locked();
        assert_eq!(p.weight, p.map.len() * 30);
        assert!(p.map.keys().all(|k| k.0 == b.id));
        drop(p);
    }

    #[test]
    fn a_zero_limit_keeps_the_entry_made_last_only() {
        let pool = Arc::new(Pool::<Lump>::new(0));
        let c = Cache::new(pool.clone());
        for name in ["a", "b", "c"] {
            cached(&c, name, |_| Ok(Lump(1000))).unwrap();
            assert_eq!(pool.locked().map.len(), 1);
        }
        assert!(pool.locked().map.contains_key(&(c.id, "c".to_string())));
    }

    #[test]
    fn the_limits_split_a_quarter_for_models_and_refuse_what_does_not_fit() {
        assert_eq!(CacheLimits::total_mib(4096), Some(CacheLimits::default()));
        assert_eq!(CacheLimits::total_mib(0), Some(CacheLimits { flats: 0, textures: 0 }));
        assert_eq!(CacheLimits::total(10), CacheLimits { flats: 2, textures: 8 });
        assert_eq!(CacheLimits::total_mib(u64::MAX), None);
        let budget = CacheBudget::new(CacheLimits::total(4));
        budget.set_limits(CacheLimits::default());
        assert_eq!(budget.limits(), CacheLimits::default());
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nwn-bake-game-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_user_folder_without_hak_has_no_haks() {
        let user = dir("nohak");
        assert!(all_in_user(Some(&user)).unwrap().is_empty());
        assert!(all_in_user(Some(&user.join("missing"))).is_err());
    }

    #[test]
    fn a_hak_is_found_whatever_the_case_of_its_extension() {
        let user = dir("haks");
        std::fs::create_dir_all(user.join("hak")).unwrap();
        for f in ["b.hak", "A.HAK", "c.Hak", "notes.txt"] {
            std::fs::write(user.join("hak").join(f), b"").unwrap();
        }
        assert_eq!(all_in_user(Some(&user)).unwrap(), ["A", "b", "c"]);
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn the_install_is_read_in_english_else_in_the_language_it_has() {
        let root = dir("lang");
        let tlk = |lang: &str| {
            std::fs::create_dir_all(root.join("lang").join(lang).join("data")).unwrap();
            std::fs::write(root.join("lang").join(lang).join("data/dialog.tlk"), b"").unwrap();
        };
        assert!(language(&root).unwrap_err().to_string().contains("not an NWN:EE install"));
        tlk("fr");
        tlk("de");
        assert_eq!(language(&root).unwrap(), "de");
        tlk("en");
        assert_eq!(language(&root).unwrap(), "en");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_talk_table_given_wins_over_the_module_s_and_blank_is_none() {
        assert_eq!(Tlk::given_or_module(Some("a".into()), Some("m".into())), Tlk::Named("a".into()));
        assert_eq!(Tlk::given_or_module(Some(" ".into()), Some("m".into())), Tlk::Module("m".into()));
        assert_eq!(Tlk::given_or_module(None, None), Tlk::Auto);
    }
}
