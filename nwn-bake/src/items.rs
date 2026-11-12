// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The items a creature blueprint wears. A `.utc` names each equipped item by resref (`Equip_ItemList`, `EquippedRes`), where a
//! character (`.bic`) or a creature placed in an area embeds the item itself. [`Items`] says where those resrefs are looked up: a
//! folder of the module's item blueprints first (a module's own items are in no hak), then the game as it was opened (the user's
//! override and haks, then the install). [`equipped`] reads every slot's item and lists the ones it could not find
//! ([`MissingItem`]), so a caller can say so instead of drawing the creature without them in silence.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use mg_core::{ResRef, ResType};
use mg_gff::{Gff, Struct};
use mg_rules::GameData;
use serde::{Deserialize, Serialize};

/// Where a creature blueprint's equipped items are looked up. The default looks in the game alone.
#[derive(Clone, Debug, Default)]
pub struct Items {
    /// The folder's item blueprints by lower-case resref; empty without a folder.
    files: HashMap<String, PathBuf>,
    /// An item not found is an error ([`Items::strict`]).
    strict: bool,
}

impl Items {
    /// The game alone: the user's override and haks, then the install, as the game was opened.
    pub fn game() -> Items {
        Items::default()
    }

    /// `dir`'s item blueprints first, then the game. Every `<resref>.uti` (binary) and `<resref>.uti.json` (nwn_gff's JSON, as
    /// nasher projects keep them) in `dir` and its subfolders, folders whose names start with a dot left out.
    /// An item found twice is read from the file changed last, as [`crate::module::ModuleSource`] reads a folder. A `dir` that is
    /// not a folder is an error.
    pub fn folder(dir: &Path) -> Result<Items> {
        anyhow::ensure!(dir.is_dir(), "{}: no such folder (the item blueprints)", dir.display());
        let mut found: HashMap<String, (PathBuf, std::time::SystemTime)> = HashMap::new();
        let mut todo = vec![dir.to_path_buf()];
        while let Some(d) = todo.pop() {
            for entry in std::fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))?.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() {
                    if !name.starts_with('.') {
                        todo.push(path);
                    }
                    continue;
                }
                let Some(resref) = name.strip_suffix(".uti").or_else(|| name.strip_suffix(".uti.json")) else { continue };
                if resref.is_empty() || resref.len() > ResRef::MAX_LEN {
                    continue;
                }
                let changed = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                match found.get(resref) {
                    Some((_, at)) if *at >= changed => {}
                    _ => _ = found.insert(resref.to_string(), (path, changed)),
                }
            }
        }
        Ok(Items { files: found.into_iter().map(|(k, (p, _))| (k, p)).collect(), strict: false })
    }

    /// With `strict`, [`equipped`] fails when an equipped item is not found, instead of listing it: for a batch that must not
    /// draw a creature short of what it wears.
    pub fn strict(mut self, strict: bool) -> Items {
        self.strict = strict;
        self
    }

    /// How many item blueprints the folder has (0 without one).
    pub fn in_folder(&self) -> usize {
        self.files.len()
    }

    /// The item `resref` names: the folder's, else the game's. `Ok(None)` when neither has it; an error when the file found does
    /// not read.
    pub fn find(&self, game: &GameData, resref: &str) -> Result<Option<Gff>> {
        let key = resref.to_ascii_lowercase();
        if let Some(path) = self.files.get(&key) {
            return crate::module::read_gff(path).map(Some);
        }
        match game.resman.get_named(&key, ResType::UTI) {
            Ok(bytes) => Gff::read(&bytes).map(Some).with_context(|| format!("reading {key}.uti from the game")),
            Err(_) => Ok(None),
        }
    }
}

/// An equipped item that could not be found or read: the creature is drawn without it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingItem {
    /// The equipment slot, as the game's `INVENTORY_SLOT_*` constants name it, lower case: `head`, `chest`, `righthand`...
    /// ([`slot_name`]).
    pub slot: String,
    /// The item's resref, as the blueprint gives it.
    pub resref: String,
    /// Why, when a file was found that does not read; None when no file was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl std::fmt::Display for MissingItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.resref, self.slot)?;
        if let Some(e) = &self.error {
            write!(f, ": {e}")?;
        }
        Ok(())
    }
}

/// An equipment slot's name, from its `Equip_ItemList` struct id (one bit a slot), as the game's `INVENTORY_SLOT_*` constants name
/// it, lower case. Another id is `slot 0x...`.
pub fn slot_name(id: u32) -> String {
    const NAMES: [&str; 18] = [
        "head",
        "chest",
        "boots",
        "arms",
        "righthand",
        "lefthand",
        "cloak",
        "leftring",
        "rightring",
        "neck",
        "belt",
        "arrows",
        "bullets",
        "bolts",
        "cweapon_l",
        "cweapon_r",
        "cweapon_b",
        "carmour",
    ];
    match NAMES.get(id.trailing_zeros() as usize) {
        Some(name) if id.is_power_of_two() => (*name).to_string(),
        _ => format!("slot {id:#x}"),
    }
}

/// The items a creature wears that are named by resref, read: what [`equipped`] gives.
#[derive(Clone, Debug, Default)]
pub struct Equipped {
    /// The items found, by lower-case resref.
    found: HashMap<String, Gff>,
    /// The ones not found or not read, in the order of the blueprint's `Equip_ItemList`.
    pub missing: Vec<MissingItem>,
}

impl Equipped {
    /// The item `resref` names, as a creature's composition asks for it (`mg_preview::creature`); None for one not found.
    pub fn get(&self, resref: ResRef) -> Option<Gff> {
        self.found.get(&resref.to_string().to_ascii_lowercase()).cloned()
    }
}

/// Reads every item the creature `utc` names in its `Equip_ItemList` (by `EquippedRes`), from `items`. Items embedded whole (a
/// `.bic`, a creature placed in an area) need no lookup and are never missing. Every slot is read, those a picture does not show
/// (rings, boots, a belt) too: a resref that is nowhere is a fault in the blueprint either way. An error only when `items` is
/// [strict](Items::strict) and an item is missing, naming each with its slot.
pub fn equipped(game: &GameData, utc: &Struct, items: &Items) -> Result<Equipped> {
    let mut out = Equipped::default();
    for slot in utc.list("Equip_ItemList").unwrap_or(&[]) {
        let Some(resref) = slot.resref("EquippedRes").filter(|r| !r.is_empty()) else { continue };
        let name = resref.to_string();
        let key = name.to_ascii_lowercase();
        if out.found.contains_key(&key) {
            continue;
        }
        let error = match items.find(game, &name) {
            Ok(Some(gff)) => {
                out.found.insert(key, gff);
                continue;
            }
            Ok(None) => None,
            Err(e) => Some(format!("{e:#}")),
        };
        out.missing.push(MissingItem { slot: slot_name(slot.id), resref: name, error });
    }
    if items.strict && !out.missing.is_empty() {
        let list: Vec<String> = out.missing.iter().map(ToString::to_string).collect();
        anyhow::bail!("equipped items not found: {}", list.join(", "));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_named_as_the_game_names_them() {
        assert_eq!(slot_name(0x1), "head");
        assert_eq!(slot_name(0x20), "lefthand");
        assert_eq!(slot_name(0x20000), "carmour");
        assert_eq!(slot_name(0x3), "slot 0x3");
        assert_eq!(slot_name(0x40000), "slot 0x40000");
    }

    #[test]
    fn a_missing_item_says_its_slot() {
        let m = MissingItem { slot: "head".into(), resref: "helm".into(), error: None };
        assert_eq!(m.to_string(), "helm (head)");
        assert_eq!(MissingItem { error: Some("bad".into()), ..m }.to_string(), "helm (head): bad");
    }
}
