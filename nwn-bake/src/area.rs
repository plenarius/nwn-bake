// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Areas: reading an area's tileset ([`tileset`]), and its scene straight from its files ([`scene()`]), which the area pictures are drawn
//! from, with the floor probe the roof cut is decided by ([`floor`]); and a layout that is not a file yet, a draft of its tiles, made
//! into an area's files ([`draft`]).

pub mod draft;
pub mod floor;
pub mod scene;

pub use floor::{FloorFile, FloorProbe, FloorStats, RoofCut, RoofRule};
pub use scene::{
    Kind, Light, Part, Scene, SceneOptions, SceneTexture, Triangle, chasm_black, chasm_shape, cut, is_chasm_floor, is_chasm_terrain, scene,
};

use std::collections::HashSet;

use anyhow::{Result, anyhow};
use mg_core::{ResRef, ResType};
use mg_resman::ResKey;
use mg_rules::GameData;
use mg_set::Tileset;

/// A tileset as the game reads it, and the sections it counts but does not have.
pub struct TilesetRead {
    /// The tileset as read, with the absent sections empty.
    pub tileset: Tileset,
    /// Sections a list counts but the file does not contain (`TERRAIN2` when it says `[TERRAIN TYPES] Count=3` and has
    /// `[TERRAIN0]` and `[TERRAIN1]`; mst03 does). They are read as empty, as the game reads them. A long run of them is one entry,
    /// `TILE20..TILE99`.
    pub absent: Vec<String>,
}

impl TilesetRead {
    /// One note per absent section, for a picture's (or any other reader's) notes.
    pub fn notes(&self, resref: &str) -> Vec<String> {
        self.absent.iter().map(|s| format!("tileset {resref}: [{s}] is counted but absent (read as empty, as the game does)")).collect()
    }
}

/// Reads the tileset `resref` from the game (with its haks).
pub fn tileset(game: &GameData, resref: ResRef) -> Result<TilesetRead> {
    let data = game.resman.get(&ResKey::new(resref, ResType::SET)).map_err(|_| anyhow!("tileset {resref}: not found"))?;
    parse_tileset(&data, game.language.codepage()).map_err(|e| anyhow!("tileset {resref}: {e}"))
}

/// Parses a `.set`. A terrain, crosser, rule or tile list whose `Count` is higher than the sections it has (mst03 declares three
/// terrains and has two) is read the way the game reads it, the absent sections empty; they are listed in
/// [`TilesetRead::absent`]. Only a file with no `[GENERAL]` is no tileset.
pub fn parse_tileset(data: &[u8], codepage: mg_core::Codepage) -> Result<TilesetRead, mg_set::SetError> {
    // (Moonglow reads absent sections as holes since 3.4.0; the note of which ones is ours.)
    let tileset = Tileset::parse(data, codepage)?;
    let absent = absent_sections(&tileset.ini);
    Ok(TilesetRead { tileset, absent })
}

/// The sections the lists count and the file lacks.
fn absent_sections(ini: &mg_set::Ini) -> Vec<String> {
    let names: HashSet<String> = ini.sections.iter().map(|s| s.name.to_ascii_uppercase()).collect();
    let mut absent = Vec::new();
    for (list, item) in [
        ("TERRAIN TYPES", "TERRAIN"),
        ("CROSSER TYPES", "CROSSER"),
        ("PRIMARY RULES", "PRIMARY RULE"),
        ("SECONDARY RULES", "SECONDARY RULE"),
        ("TILES", "TILE"),
    ] {
        let count =
            ini.sections.iter().find(|s| s.name.eq_ignore_ascii_case(list)).and_then(|s| s.int("Count")).unwrap_or(0).max(0) as usize;
        // (A count that is no count, `Count=1869573190`, costs nothing: past the last section there is, the rest is one run.)
        let last = names.iter().filter_map(|n| n.strip_prefix(item)?.parse::<usize>().ok()).filter(|&i| i < count).max();
        let n = last.map_or(0, |i| i + 1);
        absent.extend((0..n).filter(|i| !names.contains(&format!("{item}{i}"))).map(|i| format!("{item}{i}")));
        match count - n {
            0 => {}
            1..=8 => absent.extend((n..count).map(|i| format!("{item}{i}"))),
            _ => absent.push(format!("{item}{n}..{item}{}", count - 1)),
        }
    }
    absent
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest tileset that parses, with `terrains` terrains counted and the sections `present` written.
    fn set(terrains: usize, present: &[usize]) -> String {
        let mut s = String::from(
            "[GENERAL]\r\nName=TST01\r\nType=SET\r\nVersion=V1.0\r\nInterior=0\r\nHasHeightTransition=0\r\nEnvMap=\r\nTransition=5\r\nDisplayName=-1\r\nBorder=\r\nDefault=\r\nFloor=\r\n\r\n[GRASS]\r\nGrass=0\r\n\r\n",
        );
        s += &format!("[TERRAIN TYPES]\r\nCount={terrains}\r\n\r\n");
        for t in present {
            s += &format!("[TERRAIN{t}]\r\nName=Terrain{t}\r\nStrRef=-1\r\n\r\n");
        }
        s += "[CROSSER TYPES]\r\nCount=0\r\n\r\n[PRIMARY RULES]\r\nCount=0\r\n\r\n[SECONDARY RULES]\r\nCount=0\r\n\r\n[TILES]\r\nCount=0\r\n\r\n[GROUPS]\r\nCount=0\r\n";
        s
    }

    const CP: mg_core::Codepage = mg_core::Codepage::WINDOWS_1252;

    /// mst03: `[TERRAIN TYPES] Count=3`, `[TERRAIN0]` and `[TERRAIN1]`, no `[TERRAIN2]`.
    #[test]
    fn a_terrain_counted_but_absent_is_read_as_empty() {
        let read = parse_tileset(set(3, &[0, 1]).as_bytes(), CP).unwrap();
        assert_eq!(read.absent, ["TERRAIN2"]);
        assert_eq!(read.tileset.terrains.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Terrain0", "Terrain1"]);
        assert_eq!(read.notes("mst03"), ["tileset mst03: [TERRAIN2] is counted but absent (read as empty, as the game does)"]);
    }

    #[test]
    fn a_hole_and_a_long_run_are_named() {
        assert_eq!(parse_tileset(set(3, &[0, 2]).as_bytes(), CP).unwrap().absent, ["TERRAIN1"]);
        assert_eq!(parse_tileset(set(40, &[0]).as_bytes(), CP).unwrap().absent, ["TERRAIN1..TERRAIN39"]);
        assert_eq!(parse_tileset(set(1869573190, &[0]).as_bytes(), CP).unwrap().absent, ["TERRAIN1..TERRAIN1869573189"]);
    }

    #[test]
    fn a_whole_tileset_has_nothing_absent_and_one_without_general_is_none() {
        assert!(parse_tileset(set(2, &[0, 1]).as_bytes(), CP).unwrap().absent.is_empty());
        assert!(parse_tileset(set(1, &[0]).replacen("[GENERAL]", "[GENERALX]", 1).as_bytes(), CP).is_err());
    }
}
