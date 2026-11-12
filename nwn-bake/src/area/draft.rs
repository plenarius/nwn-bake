// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A layout that is not a file yet: a draft of an area's tiles, made into the area's files so it bakes as any area does
//! (`nwn-bake area-3d --draft`, `docs/area-3d.md`, "Drafts").
//!
//! A draft is the tiles a tile painter drew: a size, one tile per cell (in the form of the game's `SetTileJson`), where the
//! template's origin now is and which of the template's objects stand on changed tiles. It is made the way a module that repaints an
//! area by script would build it from its template, so a preview shows what the game will build:
//!
//! - **With a template** (an area's `.are` and `.git`): the `.are` gets the draft's `Width`, `Height` and `Tile_List`. Each new tile
//!   starts from the template's tile at the same place once the origin is taken off (cell `i` is the template's cell
//!   `(i % width - origin_dx, i / width - origin_dy)`), else from the template's first tile, so a tile the template had keeps its
//!   lights; then its `Tile_ID`, `Tile_Orientation` and `Tile_Height` (and `Tile_AnimLoop1..3` when given) are the draft's. Every
//!   object of the `.git`'s lists with a position (`X`/`Y` for doors and placeables, `XPosition`/`YPosition` for the others) moves
//!   by the origin, 10 metres a tile, an encounter's `Geometry` and `SpawnPointList` with it; it is left out when it is named in
//!   `drop_objects` (by list, tag and template position within half a metre) or when it then stands outside the new size. An
//!   object whose tag `keep_tags` names (exactly, or by a `prefix*` entry) is never dropped by name. Every other field is the
//!   template's.
//! - **Without a template**: a bare `.are` of the draft's tileset, size and tiles, and no `.git`.
//!
//! A draft whose size, tiles and origin are its template's own gives the template's files byte for byte, so it bakes to the same
//! `.glb` and floor file, under the same key.

use anyhow::{Context, Result, bail, ensure};
use mg_gff::{Gff, Struct, Value};
use serde::Deserialize;

use super::scene::MAX_TILES;
use crate::iso::batch::AreaSource;
use crate::module::AreaFiles;

/// How far the origin may move, in tiles either way.
pub const MAX_ORIGIN: i32 = 1024;
/// A tile's side in metres.
const TILE_M: f32 = 10.0;

/// A draft (the JSON `--draft` reads): a list of tiles in the form of the game's `SetTileJson`, with the origin it ends at and the
/// template objects to leave out.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    /// The area's resref in the files baked (`<name>.glb`): 1 to 16 of `a-z`, `0-9`, `_`. By default the template's, else `draft`.
    #[serde(default)]
    pub name: Option<String>,
    /// The tileset. Required without a template; with one, it must be the template's (a repainted area keeps its tileset).
    #[serde(default)]
    pub tileset: Option<String>,
    /// Width in tiles, 1 to 32.
    pub width: u32,
    /// Height in tiles, 1 to 32.
    pub height: u32,
    /// One tile per cell, row by row from the south-west (index `y * width + x`).
    pub tiles: Vec<DraftTile>,
    /// Where the template's south-west corner is, in tiles east of the draft's (a house that grew 2 tiles west has 2). With a
    /// template only.
    #[serde(default)]
    pub origin_dx: i32,
    /// The same, in tiles north.
    #[serde(default)]
    pub origin_dy: i32,
    /// Template objects left out, in template coordinates. With a template only.
    #[serde(default)]
    pub drop_objects: Vec<DropObject>,
    /// Tags `drop_objects` never drops: a tag exactly, or `prefix*` for every tag that starts with `prefix`. With a template only.
    /// When left out, none: the caller names the tags to keep.
    #[serde(default)]
    pub keep_tags: Vec<String>,
}

/// A tile of a draft (the game's `SetTileJson` form).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftTile {
    /// Its index; when given it must be its place in the list.
    #[serde(default)]
    pub index: Option<u32>,
    /// The tileset's tile, 0 or more.
    pub tileid: i32,
    /// Quarter turns anticlockwise, 0 to 3.
    pub orientation: i32,
    /// Height steps (0 in a house).
    #[serde(default)]
    pub height: i32,
    /// `Tile_AnimLoop1`; the base tile's when not given.
    #[serde(default)]
    pub animloop1: Option<u8>,
    /// `Tile_AnimLoop2`; the base tile's when not given.
    #[serde(default)]
    pub animloop2: Option<u8>,
    /// `Tile_AnimLoop3`; the base tile's when not given.
    #[serde(default)]
    pub animloop3: Option<u8>,
}

/// A template object a draft leaves out (`drop_objects`).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropObject {
    /// Its `.git` list: `door`, `placeable`, `trigger`, `waypoint` or `sound`.
    pub list: String,
    /// Its tag.
    pub tag: String,
    /// Its position in the template, metres east.
    pub x: f64,
    /// Its position in the template, metres north.
    pub y: f64,
}

impl Draft {
    /// Reads a draft from its JSON and checks it (see [`Draft::check`]).
    pub fn parse(json: &[u8]) -> Result<Draft> {
        let d: Draft = serde_json::from_slice(json).context("the draft is not a draft's JSON")?;
        d.check()?;
        Ok(d)
    }

    /// Checks what can be checked without the template or the game: the name, the size (1 to 32 a side), one tile per cell in index
    /// order, each tile 0 or more and turned 0 to 3, and the origin (at most [`MAX_ORIGIN`] tiles).
    pub fn check(&self) -> Result<()> {
        if let Some(n) = &self.name {
            ensure!(is_resref(n), "the draft's name {n:?} is not a resref (1 to 16 of a-z, 0-9, _)");
        }
        if let Some(t) = &self.tileset {
            ensure!(is_resref(&t.to_ascii_lowercase()), "the draft's tileset {t:?} is not a resref");
        }
        let side = 1..=MAX_TILES as u32;
        ensure!(
            side.contains(&self.width) && side.contains(&self.height),
            "the draft is {}x{} tiles; an area is 1 to {MAX_TILES} a side",
            self.width,
            self.height
        );
        let n = (self.width * self.height) as usize;
        ensure!(self.tiles.len() == n, "the draft has {} tiles for {}x{}", self.tiles.len(), self.width, self.height);
        for (i, t) in self.tiles.iter().enumerate() {
            ensure!(t.index.is_none_or(|x| x as usize == i), "tile {i} of the draft has index {}", t.index.unwrap_or(0));
            ensure!(t.tileid >= 0, "tile {i} of the draft has tile id {}", t.tileid);
            ensure!((0..=3).contains(&t.orientation), "tile {i} of the draft is turned {} (0 to 3)", t.orientation);
        }
        ensure!(
            self.origin_dx.abs() <= MAX_ORIGIN && self.origin_dy.abs() <= MAX_ORIGIN,
            "the draft's origin ({}, {}) is more than {MAX_ORIGIN} tiles away",
            self.origin_dx,
            self.origin_dy
        );
        Ok(())
    }

    /// The area the draft makes: from `template` (its `.are` and `.git`) when given, else a bare area of the draft's tileset (see the
    /// module). Fails when the draft does not check, names another tileset than the template's, or has none without a template.
    pub fn area(&self, template: Option<&AreaFiles>) -> Result<AreaFiles> {
        self.check()?;
        let name = self.name.clone().or_else(|| template.map(|t| t.resref.clone())).unwrap_or_else(|| "draft".to_string());
        let tileset = self.tileset.as_deref().map(str::to_ascii_lowercase);
        let Some(template) = template else {
            let Some(tileset) = tileset else { bail!("a draft without a template names its tileset") };
            ensure!(self.origin_dx == 0 && self.origin_dy == 0, "a draft without a template has no origin to move");
            ensure!(self.drop_objects.is_empty(), "a draft without a template has no objects to drop");
            ensure!(self.keep_tags.is_empty(), "a draft without a template has no objects to keep");
            return Ok(AreaFiles { resref: name, are: self.bare_are(&tileset), git: None });
        };
        let theirs = template.are.root.resref("Tileset").map_or_default(|r| r.to_string().to_ascii_lowercase());
        if let Some(t) = tileset {
            ensure!(t == theirs, "the draft's tileset {t} is not its template's ({theirs})");
        }
        let mut are = template.are.clone();
        self.paint_tiles(&mut are.root).with_context(|| format!("{}.are", template.resref))?;
        let git = template.git.clone().map(|mut g| {
            self.move_objects(&mut g.root);
            g
        });
        Ok(AreaFiles { resref: name, are, git })
    }

    /// The template's `.are` given the draft's size and tiles.
    fn paint_tiles(&self, are: &mut Struct) -> Result<()> {
        let old_w = are.integer("Width").unwrap_or(0);
        let old_h = are.integer("Height").unwrap_or(0);
        let old = are.list("Tile_List").unwrap_or(&[]);
        ensure!(old.len() as i64 == old_w * old_h, "Tile_List has {} tiles for {old_w}x{old_h}", old.len());
        let first = old.first().cloned().unwrap_or_else(|| Struct::new(1));
        let w = i64::from(self.width);
        let tiles: Vec<Struct> = self
            .tiles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let (ox, oy) = (i as i64 % w - i64::from(self.origin_dx), i as i64 / w - i64::from(self.origin_dy));
                let inside = (0..old_w).contains(&ox) && (0..old_h).contains(&oy);
                let mut s = if inside { old[(oy * old_w + ox) as usize].clone() } else { first.clone() };
                set_tile(&mut s, t);
                s
            })
            .collect();
        set_int(are, "Width", self.width.into(), Value::Int);
        set_int(are, "Height", self.height.into(), Value::Int);
        are.set("Tile_List", Value::List(tiles));
        Ok(())
    }

    /// The template's `.git` with its objects moved by the origin, those dropped or now outside left out.
    fn move_objects(&self, git: &mut Struct) {
        let (dx, dy) = (TILE_M * self.origin_dx as f32, TILE_M * self.origin_dy as f32);
        let (max_x, max_y) = (TILE_M * self.width as f32, TILE_M * self.height as f32);
        let moved = self.origin_dx != 0 || self.origin_dy != 0;
        for field in &mut git.fields {
            let list = field.label.to_string_lossy();
            let Value::List(items) = &mut field.value else { continue };
            if list == "VarTable" {
                continue;
            }
            let (lx, ly) = if list == "Door List" || list == "Placeable List" { ("X", "Y") } else { ("XPosition", "YPosition") };
            let kind = list_kind(&list);
            items.retain_mut(|o| {
                let Some(x) = number(o, lx) else { return true };
                let y = number(o, ly).unwrap_or(0.0);
                if self.dropped(kind, &text(o, "Tag"), x, y) {
                    return false;
                }
                let (x, y) = (x + dx, y + dy);
                if x < 0.0 || y < 0.0 || x > max_x || y > max_y {
                    return false;
                }
                if moved {
                    set_float(o, lx, x);
                    set_float(o, ly, y);
                    if list == "Encounter List" {
                        for points in ["Geometry", "SpawnPointList"] {
                            for p in o.list_mut(points).into_iter().flatten() {
                                for (label, by) in [("X", dx), ("Y", dy)] {
                                    if let Some(v) = number(p, label) {
                                        set_float(p, label, v + by);
                                    }
                                }
                            }
                        }
                    }
                }
                true
            });
        }
    }

    /// Whether `keep_tags` names `tag`.
    fn kept(&self, tag: &str) -> bool {
        self.keep_tags.iter().any(|k| k.strip_suffix('*').map_or(k == tag, |prefix| tag.starts_with(prefix)))
    }

    /// Whether the object of list `kind` tagged `tag` at template position (`x`, `y`) is named in `drop_objects` (never one
    /// `keep_tags` names).
    fn dropped(&self, kind: &str, tag: &str, x: f32, y: f32) -> bool {
        if self.kept(tag) {
            return false;
        }
        self.drop_objects.iter().any(|d| d.list == kind && d.tag == tag && (d.x as f32 - x).abs() < 0.5 && (d.y as f32 - y).abs() < 0.5)
    }

    /// A bare `.are`: the tileset, the size and the tiles, nothing else.
    fn bare_are(&self, tileset: &str) -> Gff {
        let mut are = Gff::new(*b"ARE ");
        let r = &mut are.root;
        r.set("Tileset", Value::ResRef(tileset.as_bytes().to_vec()));
        r.set("Width", Value::Int(self.width as i32));
        r.set("Height", Value::Int(self.height as i32));
        let tiles = self
            .tiles
            .iter()
            .map(|t| {
                let mut s = Struct::new(1);
                set_tile(&mut s, t);
                s
            })
            .collect();
        r.set("Tile_List", Value::List(tiles));
        are
    }
}

/// `.git` list `list`'s name in `drop_objects`, "" for a list a draft does not name.
fn list_kind(list: &str) -> &'static str {
    match list {
        "Door List" => "door",
        "Placeable List" => "placeable",
        "TriggerList" => "trigger",
        "WaypointList" => "waypoint",
        "SoundList" => "sound",
        _ => "",
    }
}

/// Whether `s` is a resref as a draft names one: 1 to 16 of `a-z`, `0-9`, `_`.
fn is_resref(s: &str) -> bool {
    (1..=16).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A tile struct given the draft's tile.
fn set_tile(s: &mut Struct, t: &DraftTile) {
    set_int(s, "Tile_ID", t.tileid.into(), Value::Int);
    set_int(s, "Tile_Orientation", t.orientation.into(), Value::Int);
    set_int(s, "Tile_Height", t.height.into(), Value::Int);
    for (label, v) in [("Tile_AnimLoop1", t.animloop1), ("Tile_AnimLoop2", t.animloop2), ("Tile_AnimLoop3", t.animloop3)] {
        if let Some(v) = v {
            set_int(s, label, v.into(), |v| Value::Byte(v as u8));
        }
    }
}

/// Sets integer field `label` to `v`, in the field's own type when it has one that holds `v`, else as `new` makes it.
fn set_int(s: &mut Struct, label: &str, v: i64, new: fn(i32) -> Value) {
    let kept = s.get(label).and_then(|old| old.as_integer().and(old.field_type().int_value(i128::from(v))));
    s.set(label, kept.unwrap_or_else(|| new(v as i32)));
}

/// Sets float field `label` to `v`, a `Double` when it is one, else a `Float`.
fn set_float(s: &mut Struct, label: &str, v: f32) {
    let value = if matches!(s.get(label), Some(Value::Double(_))) { Value::Double(f64::from(v)) } else { Value::Float(v) };
    s.set(label, value);
}

/// A number field as a float (integers too); None when it is missing or not a number.
fn number(s: &Struct, label: &str) -> Option<f32> {
    match s.get(label)? {
        Value::Float(f) => Some(*f),
        Value::Double(d) => Some(*d as f32),
        v => v.as_integer().map(|i| i as f32),
    }
}

/// A string field (a `String`, or a `ResRef`) as text; "" when missing.
fn text(s: &Struct, label: &str) -> String {
    match s.get(label) {
        Some(Value::String(b) | Value::ResRef(b)) => String::from_utf8_lossy(b).into_owned(),
        _ => String::new(),
    }
}

/// One area made from a draft, as an [`AreaSource`] for the batch: it has only that area.
pub struct DraftSource {
    /// The area the draft made ([`Draft::area`]).
    pub area: AreaFiles,
}

impl AreaSource for DraftSource {
    fn area(&self, resref: &str) -> Result<AreaFiles> {
        ensure!(self.area.resref.eq_ignore_ascii_case(resref), "the draft is area {}, not {resref}", self.area.resref);
        Ok(self.area.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(id: i32, o: i32) -> Struct {
        let mut s = Struct::new(1);
        s.set("Tile_ID", Value::Int(id));
        s.set("Tile_Orientation", Value::Int(o));
        s.set("Tile_Height", Value::Int(0));
        s.set("Tile_MainLight1", Value::Byte(7));
        s.set("Tile_AnimLoop1", Value::Byte(1));
        s
    }

    fn obj(tag: &str, x: f32, y: f32) -> Struct {
        let mut s = Struct::new(8);
        s.set("Tag", Value::String(tag.as_bytes().to_vec()));
        s.set("X", Value::Float(x));
        s.set("Y", Value::Float(y));
        s
    }

    /// A 2x2 template on `tst01`: tiles 10, 11, 12, 13 with their main light 7; a door at the entry, a door and a placeable inside, a
    /// waypoint, and an encounter with a spawn point.
    fn template() -> AreaFiles {
        let mut are = Gff::new(*b"ARE ");
        are.root.set("Tileset", Value::ResRef(b"tst01".to_vec()));
        are.root.set("Width", Value::Int(2));
        are.root.set("Height", Value::Int(2));
        are.root.set("Flags", Value::Dword(1));
        are.root.set("Tile_List", Value::List((10..14).map(|i| tile(i, 0)).collect()));
        let mut git = Gff::new(*b"GIT ");
        let mut wp = Struct::new(5);
        wp.set("Tag", Value::String(b"wp".to_vec()));
        wp.set("XPosition", Value::Float(15.0));
        wp.set("YPosition", Value::Float(15.0));
        let mut enc = Struct::new(7);
        enc.set("XPosition", Value::Float(5.0));
        enc.set("YPosition", Value::Float(5.0));
        let mut pt = Struct::new(0);
        pt.set("X", Value::Float(4.0));
        pt.set("Y", Value::Float(6.0));
        enc.set("SpawnPointList", Value::List(vec![pt]));
        git.root.set("Door List", Value::List(vec![obj("entry", 5.0, 0.0), obj("inner", 10.0, 15.0)]));
        git.root.set("Placeable List", Value::List(vec![obj("chair", 15.0, 5.0)]));
        git.root.set("WaypointList", Value::List(vec![wp]));
        git.root.set("Encounter List", Value::List(vec![enc]));
        git.root.set("VarTable", Value::List(vec![Struct::new(0)]));
        AreaFiles { resref: "tpl".into(), are, git: Some(git) }
    }

    fn draft(json: serde_json::Value) -> Draft {
        Draft::parse(json.to_string().as_bytes()).unwrap()
    }

    fn tiles(ids: &[i32]) -> serde_json::Value {
        ids.iter().map(|&i| serde_json::json!({"tileid": i, "orientation": 0})).collect()
    }

    fn ids(a: &AreaFiles) -> Vec<i64> {
        a.are.root.list("Tile_List").unwrap().iter().map(|t| t.integer("Tile_ID").unwrap()).collect()
    }

    fn tags(a: &AreaFiles, list: &str) -> Vec<(String, f32, f32)> {
        let (lx, ly) = if list.ends_with("List") && list.contains(' ') { ("X", "Y") } else { ("XPosition", "YPosition") };
        a.git
            .as_ref()
            .unwrap()
            .root
            .list(list)
            .unwrap()
            .iter()
            .map(|o| (text(o, "Tag"), number(o, lx).unwrap(), number(o, ly).unwrap()))
            .collect()
    }

    #[test]
    fn a_draft_of_the_template_itself_gives_its_files_byte_for_byte() {
        let t = template();
        let a = draft(serde_json::json!({"width": 2, "height": 2, "tiles": tiles(&[10, 11, 12, 13])})).area(Some(&t)).unwrap();
        assert_eq!(a.resref, "tpl");
        assert_eq!(a.are.to_bytes().unwrap(), t.are.to_bytes().unwrap());
        assert_eq!(a.git.unwrap().to_bytes().unwrap(), t.git.unwrap().to_bytes().unwrap());
    }

    #[test]
    fn growing_west_and_south_moves_the_tiles_and_objects_by_the_origin() {
        let t = template();
        // 3x3, the template in the north-east corner: origin (1, 1).
        let d = draft(serde_json::json!({
            "name": "grown", "tileset": "TST01", "width": 3, "height": 3, "origin_dx": 1, "origin_dy": 1,
            "tiles": tiles(&[1, 2, 3, 4, 10, 11, 7, 12, 99]),
        }));
        let a = d.area(Some(&t)).unwrap();
        assert_eq!(a.resref, "grown");
        assert_eq!((a.are.root.integer("Width"), a.are.root.integer("Height")), (Some(3), Some(3)));
        assert_eq!(ids(&a), [1, 2, 3, 4, 10, 11, 7, 12, 99]);
        // Every tile kept the template's light (a new one the first tile's), and a field the draft does not name.
        assert!(a.are.root.list("Tile_List").unwrap().iter().all(|s| s.byte("Tile_MainLight1") == Some(7)));
        assert_eq!(a.are.root.integer("Flags"), Some(1));
        assert_eq!(tags(&a, "Door List"), [("entry".to_string(), 15.0, 10.0), ("inner".to_string(), 20.0, 25.0)]);
        assert_eq!(tags(&a, "Placeable List"), [("chair".to_string(), 25.0, 15.0)]);
        assert_eq!(tags(&a, "WaypointList"), [("wp".to_string(), 25.0, 25.0)]);
        let enc = &a.git.as_ref().unwrap().root.list("Encounter List").unwrap()[0];
        let pt = &enc.list("SpawnPointList").unwrap()[0];
        assert_eq!((number(pt, "X"), number(pt, "Y")), (Some(14.0), Some(16.0)));
        // The VarTable is not a list of placed objects.
        assert_eq!(a.git.unwrap().root.list("VarTable").unwrap().len(), 1);
    }

    #[test]
    fn dropped_and_outside_objects_are_left_out_but_never_a_kept_tag() {
        let t = template();
        // 2x1, the south row only: the inner door (y 15) and the waypoint fall outside; the chair is dropped by name.
        let d = draft(serde_json::json!({
            "width": 2, "height": 1, "tiles": tiles(&[10, 5]), "keep_tags": ["entry"],
            "drop_objects": [
                {"list": "placeable", "tag": "chair", "x": 15.2, "y": 4.9},
                {"list": "door", "tag": "entry", "x": 5.0, "y": 0.0},
            ],
        }));
        let a = d.area(Some(&t)).unwrap();
        assert_eq!(tags(&a, "Door List"), [("entry".to_string(), 5.0, 0.0)]);
        assert!(tags(&a, "Placeable List").is_empty());
        assert!(tags(&a, "WaypointList").is_empty());
        // A drop that names another list, or is half a metre off, drops nothing.
        for drop in [
            serde_json::json!({"list": "door", "tag": "chair", "x": 15.0, "y": 5.0}),
            serde_json::json!({"list": "placeable", "tag": "chair", "x": 15.5, "y": 5.0}),
        ] {
            let d = draft(serde_json::json!({"width": 2, "height": 2, "tiles": tiles(&[1, 2, 3, 4]), "drop_objects": [drop]}));
            assert_eq!(tags(&d.area(Some(&t)).unwrap(), "Placeable List").len(), 1);
        }
    }

    #[test]
    fn keep_tags_keeps_exact_tags_and_prefixes() {
        let t = template();
        let drops = serde_json::json!([
            {"list": "placeable", "tag": "chair", "x": 15.0, "y": 5.0},
            {"list": "door", "tag": "entry", "x": 5.0, "y": 0.0},
        ]);
        let left = |keep: serde_json::Value| {
            let mut j = serde_json::json!({"width": 2, "height": 2, "tiles": tiles(&[1, 2, 3, 4]), "drop_objects": drops});
            if !keep.is_null() {
                j["keep_tags"] = keep;
            }
            let a = draft(j).area(Some(&t)).unwrap();
            (tags(&a, "Door List").len(), tags(&a, "Placeable List").len())
        };
        assert_eq!(left(serde_json::json!([])), (1, 0));
        assert_eq!(left(serde_json::json!(["ch*"])), (1, 1));
        assert_eq!(left(serde_json::json!(["chair", "entry"])), (2, 1));
        assert_eq!(left(serde_json::json!(["chai"])), (1, 0));
        // Without keep_tags, nothing is kept.
        assert_eq!(left(serde_json::Value::Null), (1, 0));
    }

    #[test]
    fn a_draft_without_a_template_is_a_bare_area_of_its_tileset() {
        let a = draft(serde_json::json!({"tileset": "tst01", "width": 1, "height": 2, "tiles": [
            {"index": 0, "tileid": 3, "orientation": 1, "height": 1, "animloop2": 0}, {"tileid": 4, "orientation": 3},
        ]}))
        .area(None)
        .unwrap();
        assert_eq!(a.resref, "draft");
        assert!(a.git.is_none());
        let r = &a.are.root;
        assert_eq!(r.resref("Tileset").unwrap().to_string(), "tst01");
        let t = r.list("Tile_List").unwrap();
        assert_eq!(
            (t[0].integer("Tile_Orientation"), t[0].integer("Tile_Height"), t[0].byte("Tile_AnimLoop2")),
            (Some(1), Some(1), Some(0))
        );
        assert_eq!(t[1].integer("Tile_ID"), Some(4));
    }

    #[test]
    fn a_draft_that_does_not_fit_is_refused() {
        let t = template();
        let bad = |j: serde_json::Value| -> String {
            match Draft::parse(j.to_string().as_bytes()).and_then(|d| d.area(Some(&t))) {
                Ok(_) => panic!("{j} was taken"),
                Err(e) => format!("{e:#}"),
            }
        };
        assert!(bad(serde_json::json!({"width": 2, "height": 2, "tiles": tiles(&[1, 2, 3])})).contains("3 tiles for 2x2"));
        assert!(bad(serde_json::json!({"width": 33, "height": 1, "tiles": []})).contains("1 to 32"));
        assert!(bad(serde_json::json!({"width": 1, "height": 1, "tiles": [{"tileid": 1, "orientation": 4}]})).contains("turned 4"));
        assert!(
            bad(serde_json::json!({"width": 1, "height": 1, "tiles": [{"index": 1, "tileid": 1, "orientation": 0}]})).contains("index 1")
        );
        assert!(bad(serde_json::json!({"width": 1, "height": 1, "tiles": tiles(&[1]), "tileset": "other"})).contains("not its template's"));
        assert!(bad(serde_json::json!({"width": 1, "height": 1, "tiles": tiles(&[1]), "name": "Big Name"})).contains("not a resref"));
        assert!(bad(serde_json::json!({"width": 1, "height": 1, "tiles": tiles(&[1]), "shift_x": 1})).contains("not a draft"));
        let no_tileset = draft(serde_json::json!({"width": 1, "height": 1, "tiles": tiles(&[1])}));
        assert!(format!("{:#}", no_tileset.area(None).unwrap_err()).contains("names its tileset"));
    }

    #[test]
    fn the_source_has_only_the_draft() {
        let s = DraftSource { area: template() };
        assert!(s.area("TPL").is_ok());
        assert!(s.area("other").is_err());
    }
}
