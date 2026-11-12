// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where areas come from: a module in any of the forms a builder keeps one, read the same way whatever the form.
//!
//! - a `.mod` (or any ERF of areas: with no `module.ifo` there is no hak list, so the caller gives the haks);
//! - a folder of `.are`/`.git`/`module.ifo` files;
//! - a folder of nwn_gff's JSON files with no `nasher.cfg` (`x.are.json`, `x.git.json`, `module.ifo.json`, any
//!   `<resref>.<gff type>.json`), in it and its subfolders, binary files among them read as they are;
//! - a nasher project (a folder with `nasher.cfg`, its GFF files kept as nwn_gff's JSON, such as `areas/core/*.are.json`);
//! - one area file (`x.are` or `x.are.json`), its `.git` beside it and the nearest `module.ifo(.json)` above it for the haks and the
//!   talk table: the cheap way to read one area of a large project;
//! - a `module.ifo` or `module.ifo.json` on its own: the hak list and the talk table, no areas.
//!
//! A nasher project is read lazily: its module target's source files are listed (as nasher's `include` and `exclude` say) and each
//! area's `.are` and `.git` is read when it is asked for, so opening a project of 1,300 areas and about 40,000 source files takes a
//! second, not the minutes converting every source takes. A project kept as NWNT goes through Moonglow's whole project. The module
//! target is the one nasher packs by default (the first, when it makes a module); [`ModuleSource::open_target`] names another, such
//! as a target that lists areas the packed module leaves out.
//!
//! `.mod`s and folders of binary files go through [`mg_module::Module::open`] (which reads only a folder's top
//! level, and only binary files). JSON is read by Moonglow ([`mg_module::nasher::Conversion`], which is
//! `mg_gff::json` with nasher's code page), so no `nwn_gff` program is needed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mg_core::{ResRef, ResType};
use mg_gff::Gff;
use mg_module::Module;
use mg_module::nasher::Conversion;
use mg_resman::ResKey;

/// A module to read areas from. See the module documentation for the forms it takes.
pub struct ModuleSource {
    label: String,
    haks: Vec<String>,
    tlk: Option<String>,
    from: From,
    warnings: Vec<String>,
}

enum From {
    /// A `.mod` or other ERF, a folder, or a folder of JSON files.
    Module(Box<Module>),
    /// A nasher project: its module target's area files, listed when opened and read when asked for.
    Project(ProjectAreas),
    /// One area's files (or none: a `module.ifo` given on its own).
    Files { area: Option<(String, PathBuf, Option<PathBuf>)>, conv: Conversion },
}

/// A nasher project's areas: the module target's source files, by resref. Opening a whole project converts every one of its sources
/// (blueprints, scripts, dialogs: on a large project, about 40,000 files and four minutes); areas need only these.
struct ProjectAreas {
    are: BTreeMap<String, PathBuf>,
    git: BTreeMap<String, PathBuf>,
    /// The areas the module's area list names, in its order (those with an `.are`).
    listed: Vec<String>,
    conv: Conversion,
}

/// A nasher project's areas and its `module.ifo`, read lazily; None when its sources are not kept as JSON (NWNT, which only
/// Moonglow's whole project reads).
fn project_areas(root: &Path, target: Option<&str>) -> Result<Option<(ProjectAreas, Option<Gff>, Vec<String>)>> {
    use mg_module::nasher::{Package, Settings};
    let package = Package::read(root).map_err(|e| anyhow::anyhow!("{}: {e}", root.display()))?;
    let settings = Settings::read(root);
    if !settings.gff_format.eq_ignore_ascii_case("json") {
        if let Some(name) = target {
            bail!("{}: target {name}: only a project kept as JSON is read by target", root.display());
        }
        return Ok(None);
    }
    let conv = Conversion::of(&settings);
    let target = match target {
        Some(name) => package.target(name).with_context(|| format!("{}: no target {name}", root.display()))?,
        None => package.module_target().with_context(|| format!("{}: no target packs a module", root.display()))?,
    };
    let files = target.source_files(root).map_err(|e| anyhow::anyhow!("{}: {e}", root.display()))?;
    let (mut are, mut git): (BTreeMap<String, PathBuf>, BTreeMap<String, PathBuf>) = (BTreeMap::new(), BTreeMap::new());
    let (mut ifo, mut warnings) = (None, Vec::new());
    for path in files {
        let name = path.file_name().map_or_default(|n| n.to_string_lossy().to_ascii_lowercase());
        let plain = name.strip_suffix(".json").unwrap_or(&name);
        let (map, resref) = if let Some(r) = plain.strip_suffix(".are") {
            (&mut are, r)
        } else if let Some(r) = plain.strip_suffix(".git") {
            (&mut git, r)
        } else {
            if plain == "module.ifo" && ifo.is_none() {
                ifo = Some(path);
            }
            continue;
        };
        if ResRef::from_str(resref).is_err() {
            continue;
        }
        // (In two folders nasher packs one of them, in no fixed order: the first here, as Moonglow reads a project.)
        match map.get(resref) {
            Some(had) => {
                warnings.push(format!("{resref} is in the project twice: {} is read, not {}", shown(root, had), shown(root, &path)));
            }
            None => _ = map.insert(resref.to_string(), path),
        }
    }
    let ifo = ifo.map(|p| read_gff_with(&p, &conv)).transpose()?;
    let mut listed = Vec::new();
    if let Some(info) = &ifo {
        let mut m = Module::new();
        if m.set_info(info).is_ok() {
            for a in m.areas().unwrap_or_default() {
                let a = a.to_string().to_ascii_lowercase();
                if are.contains_key(&a) && !listed.contains(&a) {
                    listed.push(a);
                }
            }
        }
    }
    Ok(Some((ProjectAreas { are, git, listed, conv }, ifo, warnings)))
}

fn shown(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

/// An area's files: its `.are` and its `.git` (None when it has none).
#[derive(Clone, Debug)]
pub struct AreaFiles {
    /// Lower case.
    pub resref: String,
    /// The area's `.are`: its tiles, tileset and settings.
    pub are: Gff,
    /// The area's `.git`: its placeables, doors and other objects. None when the area has none.
    pub git: Option<Gff>,
}

impl ModuleSource {
    /// Opens `path`, which is any of the forms in the module documentation.
    pub fn open(path: &Path) -> Result<ModuleSource> {
        ModuleSource::open_target(path, None)
    }

    /// Opens `path` as [`open`](Self::open) does; a nasher project through its target `target` (by name) instead of the one nasher
    /// packs by default. Its areas are that target's area files, its haks and talk table those of the `module.ifo` it lists. An error
    /// when `target` is named and `path` is no nasher project kept as JSON, or the project has no such target.
    pub fn open_target(path: &Path, target: Option<&str>) -> Result<ModuleSource> {
        let name = path.file_name().map_or_default(|n| n.to_string_lossy().to_ascii_lowercase());
        let label = path.display().to_string();
        let project_root = if name == "nasher.cfg" && path.is_file() {
            path.parent().map(Path::to_path_buf)
        } else {
            path.join("nasher.cfg").is_file().then(|| path.to_path_buf())
        };
        if let Some(target) = target
            && project_root.is_none()
        {
            bail!("{}: target {target}: not a nasher project", path.display());
        }
        #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "`name` is lower case already")]
        let is_area = name.ends_with(".are") || name.ends_with(".are.json");
        if path.is_file() && is_area {
            let stem = name.strip_suffix(".json").unwrap_or(&name);
            let resref = stem.strip_suffix(".are").unwrap_or(stem).to_string();
            ResRef::from_str(&resref).with_context(|| format!("{}: not an area's name", path.display()))?;
            let git = sibling_git(path).filter(|g| g.is_file());
            let (haks, tlk) = nearest_info(path)?;
            let conv = conversion_for(path);
            return Ok(ModuleSource {
                label,
                haks,
                tlk,
                from: From::Files { area: Some((resref, path.to_path_buf(), git)), conv },
                warnings: Vec::new(),
            });
        }
        if path.is_file() && (name == "module.ifo" || name == "module.ifo.json") {
            let (haks, tlk) = info_of(&read_gff_with(path, &conversion_for(path))?);
            return Ok(ModuleSource {
                label,
                haks,
                tlk,
                from: From::Files { area: None, conv: conversion_for(path) },
                warnings: Vec::new(),
            });
        }
        if let Some(root) = project_root
            && let Some((areas, ifo, warnings)) = project_areas(&root, target)?
        {
            let (haks, tlk) = ifo.as_ref().map_or_default(info_of);
            return Ok(ModuleSource { label, haks, tlk, from: From::Project(areas), warnings });
        }
        let json_folder = if path.is_dir() && !path.join("nasher.cfg").is_file() { json_folder(path)? } else { None };
        let (m, warnings) = match json_folder {
            // (The caller says them: [`ModuleSource::warnings`].)
            Some((m, warnings)) => (m, warnings),
            None => (Module::open(path).map_err(|e| anyhow::anyhow!("opening {}: {e}", path.display()))?, Vec::new()),
        };
        let (haks, tlk) = match m.info() {
            Ok(info) => info_of(&info),
            // (A folder that keeps its module.ifo as nwn_gff's JSON and is no nasher project.)
            Err(_) if path.is_dir() && path.join("module.ifo.json").is_file() => {
                info_of(&read_gff_with(&path.join("module.ifo.json"), &conversion_for(path))?)
            }
            // (A plain ERF of areas: no module.ifo, no haks, no talk table.)
            Err(_) => (Vec::new(), None),
        };
        Ok(ModuleSource { label, haks, tlk, from: From::Module(Box::new(m)), warnings })
    }

    /// What was odd about the source when it was opened (a resource found twice in a folder of JSON files, and which was read).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The path it was opened from, for messages.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The module's haks (`Mod_HakList`), first highest, as the game lays them.
    pub fn haks(&self) -> &[String] {
        &self.haks
    }

    /// The module's custom talk table (`Mod_CustomTlk`, without `.tlk`), when it has one.
    pub fn tlk(&self) -> Option<&str> {
        self.tlk.as_deref()
    }

    /// The areas, by resref (lower case): those the module's area list names, in its order, then any other `.are` it has. For a
    /// single area file, that area.
    pub fn areas(&self) -> Vec<String> {
        match &self.from {
            // (A plain ERF of areas has no area list: every .are in it.)
            From::Module(m) if !m.contains(&Module::info_key()) => m.keys_of(ResType::ARE).map(|k| k.resref.to_string()).collect(),
            From::Module(m) => {
                mg_module::areas::every(m).iter().filter(|a| m.contains(&ResKey::new(**a, ResType::ARE))).map(ToString::to_string).collect()
            }
            From::Files { area, .. } => area.iter().map(|(r, _, _)| r.clone()).collect(),
            // (Those the area list names, in its order, then the others by name, as Moonglow lists a module's.)
            From::Project(p) => p.listed.iter().cloned().chain(p.are.keys().filter(|a| !p.listed.contains(a)).cloned()).collect(),
        }
    }

    /// Area `resref`'s files.
    pub fn area(&self, resref: &str) -> Result<AreaFiles> {
        let lower = resref.to_ascii_lowercase();
        match &self.from {
            From::Module(m) => {
                let r = ResRef::from_str(&lower).with_context(|| format!("{resref}: not an area's name"))?;
                let get =
                    |t: ResType| m.gff(&ResKey::new(r, t)).transpose().with_context(|| format!("{}: {}", self.label, ResKey::new(r, t)));
                let are = get(ResType::ARE)?.with_context(|| format!("{} has no area {lower}", self.label))?;
                Ok(AreaFiles { resref: lower, are, git: get(ResType::GIT)? })
            }
            From::Files { area: Some((r, are, git)), conv } if *r == lower => Ok(AreaFiles {
                resref: lower,
                are: read_gff_with(are, conv)?,
                git: git.as_deref().map(|g| read_gff_with(g, conv)).transpose()?,
            }),
            From::Files { .. } => bail!("{} has no area {lower}", self.label),
            From::Project(p) => {
                let are = p.are.get(&lower).with_context(|| format!("{} has no area {lower}", self.label))?;
                Ok(AreaFiles {
                    resref: lower.clone(),
                    are: read_gff_with(are, &p.conv)?,
                    git: p.git.get(&lower).map(|g| read_gff_with(g, &p.conv)).transpose()?,
                })
            }
        }
    }

    /// The module itself, when it was opened as one (a `.mod` or a folder; a nasher project only when kept as NWNT): everything
    /// else it holds.
    pub fn module(&self) -> Option<&Module> {
        match &self.from {
            From::Module(m) => Some(m),
            From::Files { .. } | From::Project(_) => None,
        }
    }
}

/// A GFF file: binary, or nwn_gff's JSON when its name ends in `.json` (read with the code page of the nasher project it is in,
/// windows-1252 when it is in none).
pub fn read_gff(path: &Path) -> Result<Gff> {
    read_gff_with(path, &conversion_for(path))
}

/// GFF bytes: nwn_gff's JSON when `name` ends in `.json` (windows-1252), else binary.
pub fn gff_from(name: &str, bytes: &[u8]) -> Result<Gff> {
    gff_bytes(name, bytes, &Conversion::default())
}

fn read_gff_with(path: &Path, conv: &Conversion) -> Result<Gff> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    gff_bytes(&path.to_string_lossy(), &bytes, conv)
}

fn gff_bytes(name: &str, bytes: &[u8], conv: &Conversion) -> Result<Gff> {
    if name.to_ascii_lowercase().ends_with(".json") {
        return conv.from_text(mg_module::nasher::Form::Json, name, bytes).map_err(|e| anyhow::anyhow!("{e}"));
    }
    Gff::read(bytes).with_context(|| format!("reading {name}"))
}

/// A folder with no `nasher.cfg` that has nwn_gff's JSON files in it or its subfolders, read as a module: every
/// `<resref>.<gff type>.json` and every binary resource file, at any depth (folders whose names start with a dot left out). A
/// resource found twice (a binary file and a JSON file, or in two subfolders) is read from the one changed last, as Moonglow reads
/// a folder, with a warning. None when the folder has no such JSON file (a folder of binary files, read by Moonglow).
fn json_folder(dir: &Path) -> Result<Option<(Module, Vec<String>)>> {
    let mut files: Vec<(ResKey, PathBuf, bool)> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))? {
            let p = e?.path();
            let name = p.file_name().map_or_default(|n| n.to_string_lossy().into_owned());
            if p.is_dir() {
                if !name.starts_with('.') {
                    stack.push(p);
                }
                continue;
            }
            let lower = name.to_ascii_lowercase();
            match lower.strip_suffix(".json") {
                Some(stem) => {
                    if let Some(key) = ResKey::from_filename(stem).filter(|k| k.restype.is_gff()) {
                        files.push((key, p, true));
                    }
                }
                None => {
                    if let Some(key) = ResKey::from_filename(&lower) {
                        files.push((key, p, false));
                    }
                }
            }
        }
    }
    if !files.iter().any(|(_, _, json)| *json) {
        return Ok(None);
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    // The one changed last of each resource (of two changed at once, the first by path).
    let changed = |p: &Path| p.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    let mut chosen: indexmap::IndexMap<ResKey, (PathBuf, bool, std::time::SystemTime, Vec<PathBuf>)> = indexmap::IndexMap::new();
    for (key, path, json) in files {
        let at = changed(&path);
        match chosen.get_mut(&key) {
            None => _ = chosen.insert(key, (path, json, at, Vec::new())),
            Some(c) if at > c.2 => {
                let old = std::mem::replace(&mut c.0, path);
                c.3.push(old);
                (c.1, c.2) = (json, at);
            }
            Some(c) => c.3.push(path),
        }
    }
    let conv = conversion_for(dir);
    let (mut m, mut warnings) = (Module::new(), Vec::new());
    let shown = |p: &Path| p.strip_prefix(dir).unwrap_or(p).display().to_string();
    for (key, (path, json, _, others)) in chosen {
        if !others.is_empty() {
            let others: Vec<String> = others.iter().map(|p| shown(p)).collect();
            warnings.push(format!(
                "{key} is in {} more than once: {}, changed last, is read; not {}",
                dir.display(),
                shown(&path),
                others.join(", ")
            ));
        }
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let bytes = if json { gff_bytes(&path.to_string_lossy(), &bytes, &conv)?.to_bytes()? } else { bytes };
        m.set(key, bytes);
    }
    Ok(Some((m, warnings)))
}

/// `x.git` beside `x.are`, `x.git.json` beside `x.are.json` (in the case the area file's name has).
fn sibling_git(are: &Path) -> Option<PathBuf> {
    let name = are.file_name()?.to_string_lossy().into_owned();
    let lower = name.to_ascii_lowercase();
    let cut = lower.rfind(".are")?;
    let ext = &name[cut + 1..cut + 4];
    let git = match ext {
        "ARE" => "GIT",
        "Are" => "Git",
        _ => "git",
    };
    Some(are.with_file_name(format!("{}.{git}{}", &name[..cut], &name[cut + 4..])))
}

/// The hak list and talk table of the nearest `module.ifo.json` or `module.ifo` above `area` (none when there is none).
fn nearest_info(area: &Path) -> Result<(Vec<String>, Option<String>)> {
    for dir in area.ancestors().skip(1) {
        for file in ["module.ifo.json", "module.ifo"] {
            let p = dir.join(file);
            if p.is_file() {
                return Ok(info_of(&read_gff_with(&p, &conversion_for(&p))?));
            }
        }
    }
    Ok((Vec::new(), None))
}

/// A `module.ifo`'s hak list and talk table, read as Moonglow reads a module's (the pre-1.69 single `Mod_Hak` too).
fn info_of(info: &Gff) -> (Vec<String>, Option<String>) {
    let mut m = Module::new();
    if m.set_info(info).is_err() {
        return (Vec::new(), None);
    }
    (m.haks().unwrap_or_default(), m.custom_tlk().ok().flatten())
}

/// The JSON conversion of the nasher project `path` is in (its code page), else nasher's default.
fn conversion_for(path: &Path) -> Conversion {
    path.ancestors()
        .find(|d| d.join("nasher.cfg").is_file())
        .map_or_default(|root| Conversion::of(&mg_module::nasher::Settings::read(root)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_git_beside_an_area_keeps_its_names_case_and_form() {
        // Compared as paths, not text: Windows joins with `\`, and its paths take either separator.
        let p = |s: &str, want: &str| assert_eq!(sibling_git(Path::new(s)).as_deref(), Some(Path::new(want)), "{s}");
        p("a/b/x.are", "a/b/x.git");
        p("a/b/x.are.json", "a/b/x.git.json");
        p("X.ARE", "X.GIT");
    }
}
