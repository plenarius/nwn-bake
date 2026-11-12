// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A scene's objects: what each one is (a placeables.2da row, a model, a creature or character, an item), where it stands, which
//! way it faces and how large it is, and what is drawn for it ([`prepare`], on the CPU: no GPU needed).
//!
//! The game's models face north (+y) unturned: [`crate::creature::thumbnail`] looks at a creature's front from a yaw of 60 degrees
//! (north-east). A facing is the game's (`GetFacing`: degrees counter-clockwise from east), so a model is turned by its facing less 90.

use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use glam::{Mat4, Quat, Vec3};
use mg_gff::Gff;
use mg_rules::GameData;
use serde::Deserialize;

use crate::items::{Items, MissingItem};

/// One thing in a scene. Exactly one of `placeable`, `model`, `creature` and `item` says what it is.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    /// A row of placeables.2da, drawn as a placed placeable shows it (its idle animation, its particles).
    pub placeable: Option<u32>,
    /// A model by resref, on its own (a tile, a part, a model no 2da row names).
    pub model: Option<String>,
    /// A creature blueprint (`.utc`, or nwn_gff's `.utc.json`) or a character (`.bic`): dressed in its equipped items.
    pub creature: Option<PathBuf>,
    /// An item blueprint (`.uti`, or nwn_gff's `.uti.json`), as `item-thumb` draws it.
    pub item: Option<PathBuf>,
    /// Where it stands: its model's origin, in metres (x east, y north, z up).
    #[serde(default)]
    pub position: [f32; 3],
    /// The way it faces, degrees counter-clockwise from east, as the game's `GetFacing`; 270 (south, towards the default camera)
    /// when left out. A placeable of a `.git` with `Bearing` b (radians) faces b in degrees plus 90.
    #[serde(default = "south")]
    pub facing: f32,
    /// Its size, 1 as the game draws it.
    #[serde(default = "one")]
    pub scale: f32,
    /// Textures drawn instead of the model's, `[old, new]` by name, as `ReplaceObjectTexture` does (on a creature's body, not on
    /// what it wears).
    #[serde(default)]
    pub textures: Vec<(String, String)>,
    /// The animation it is drawn in: a creature's and an item's `pause1`, a placeable's idle state (`default` without one) when
    /// left out.
    pub anim: Option<String>,
    /// How far into the animation, in seconds.
    #[serde(default)]
    pub t: f32,
}

fn south() -> f32 {
    270.0
}

fn one() -> f32 {
    1.0
}

/// What an [`Object`] is.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A row of placeables.2da.
    Placeable(u32),
    /// A model by resref.
    Model(String),
    /// A creature blueprint or character file.
    Creature(PathBuf),
    /// An item blueprint file.
    Item(PathBuf),
}

impl Kind {
    /// `placeable`, `model`, `creature` or `item`.
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Placeable(_) => "placeable",
            Kind::Model(_) => "model",
            Kind::Creature(_) => "creature",
            Kind::Item(_) => "item",
        }
    }
}

impl Object {
    /// What it is; an error unless exactly one kind is given.
    pub fn kind(&self) -> Result<Kind> {
        let mut kinds = Vec::new();
        if let Some(r) = self.placeable {
            kinds.push(Kind::Placeable(r));
        }
        if let Some(m) = &self.model {
            kinds.push(Kind::Model(m.trim().to_ascii_lowercase()));
        }
        if let Some(c) = &self.creature {
            kinds.push(Kind::Creature(c.clone()));
        }
        if let Some(i) = &self.item {
            kinds.push(Kind::Item(i.clone()));
        }
        match kinds.len() {
            1 => Ok(kinds.remove(0)),
            0 => bail!("give one of placeable, model, creature and item"),
            _ => bail!(
                "give one of placeable, model, creature and item, not {}",
                kinds.iter().map(Kind::name).collect::<Vec<_>>().join(" and ")
            ),
        }
    }

    /// Checks it without the game: its kind and its numbers.
    pub fn check(&self) -> Result<()> {
        let kind = self.kind()?;
        if let Kind::Model(m) = &kind {
            ensure!(!m.is_empty() && m.len() <= mg_core::ResRef::MAX_LEN, "model {m:?}: a resref is 1 to 16 characters");
        }
        ensure!(self.position.iter().all(|v| v.is_finite()), "position is three numbers");
        ensure!(self.facing.is_finite(), "facing is degrees");
        ensure!(self.scale.is_finite() && self.scale > 0.0, "scale is more than 0");
        ensure!(self.t.is_finite() && self.t >= 0.0, "t is seconds, 0 or more");
        Ok(())
    }

    /// Its kind and what names it, for messages: `creature nw_goblina.utc`.
    pub fn describe(&self) -> String {
        match self.kind() {
            Ok(Kind::Placeable(r)) => format!("placeable {r}"),
            Ok(Kind::Model(m)) => format!("model {m}"),
            Ok(Kind::Creature(p)) => format!("creature {}", p.display()),
            Ok(Kind::Item(p)) => format!("item {}", p.display()),
            Err(_) => "no kind".into(),
        }
    }

    /// Model space to the game's: scaled, turned by its facing less 90 degrees about z, moved to its position.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(self.scale),
            Quat::from_rotation_z((self.facing - 90.0).to_radians()),
            Vec3::from_array(self.position),
        )
    }
}

/// What is drawn for an object.
pub(crate) struct Prepared {
    pub kind: Kind,
    pub preview: mg_preview::Preview,
    /// The animation it is drawn in.
    pub anim: String,
    pub missing_items: Vec<MissingItem>,
}

/// What `o` is drawn as, a creature's equipped items read from `items`: as the one-object pictures make it
/// ([`crate::creature::placeable_preview`], [`crate::creature::thumbnail`], [`crate::creature::item_thumbnail`]), with its texture
/// replacements.
pub(crate) fn prepare(game: &GameData, o: &Object, items: &Items) -> Result<Prepared> {
    let kind = o.kind()?;
    let read = |p: &PathBuf| -> Result<Gff> { crate::module::read_gff(p).with_context(|| format!("reading {}", p.display())) };
    let (mut preview, missing_items, anim) = match &kind {
        Kind::Placeable(row) => (crate::creature::placeable_preview(game, *row, &[])?, Vec::new(), None),
        Kind::Model(m) => {
            ensure!(game.resman.get_named(m, mg_core::ResType::MDL).is_ok(), "model {m}: not found");
            (mg_preview::Preview::model(m), Vec::new(), None)
        }
        Kind::Creature(p) => {
            let (preview, missing) = crate::creature::dressed(game, &read(p)?, items)?;
            (preview, missing, Some("pause1"))
        }
        Kind::Item(p) => (mg_preview::item(game, &read(p)?.root)?, Vec::new(), Some("pause1")),
    };
    for (old, new) in &o.textures {
        let (old, new) = (old.trim().to_ascii_lowercase(), new.trim().to_ascii_lowercase());
        if old.is_empty() || new.is_empty() {
            continue;
        }
        for part in std::iter::once(&mut preview.base).chain(preview.parts.iter_mut().filter(|p| !p.item)) {
            part.textures.insert(old.clone(), new.clone());
        }
    }
    let anim = o.anim.clone().or_else(|| anim.map(str::to_string)).or_else(|| preview.idle.clone()).unwrap_or_else(|| "default".into());
    Ok(Prepared { kind, preview, anim, missing_items })
}

/// One model of what an object is drawn as: [`parts`] gives them.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Part {
    /// The model's resref, lower case.
    pub model: String,
    /// Its textures drawn instead of the model's, old name to new.
    pub textures: std::collections::BTreeMap<String, String>,
    /// The model of something a creature wears or holds, not of its body.
    pub item: bool,
}

/// What an object is drawn as, without drawing it: [`parts`] gives one for each object.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Parts {
    /// `placeable`, `model`, `creature` or `item`.
    pub kind: &'static str,
    /// The animation it is drawn in.
    pub anim: String,
    /// Its base model first, then the parts hung on it.
    pub models: Vec<Part>,
    /// The equipped items a creature blueprint names that were not found.
    pub missing_items: Vec<MissingItem>,
}

/// What each of `spec`'s objects is drawn as, in order: the models, their textures and the animation, and the items not found.
/// Read on the CPU from the game's data alone, so a caller (or a test) can see what [`super::render`] would draw without a GPU.
pub fn parts(game: &GameData, spec: &super::Spec, items: &Items) -> Result<Vec<Parts>> {
    spec.check()?;
    let mut out = Vec::new();
    for (i, o) in spec.objects.iter().enumerate() {
        let p = prepare(game, o, items).with_context(|| format!("object {i} ({})", o.describe()))?;
        let part = |q: &mg_preview::Part| Part { model: q.model.clone(), textures: q.textures.clone(), item: q.item };
        out.push(Parts {
            kind: p.kind.name(),
            anim: p.anim,
            models: std::iter::once(&p.preview.base).chain(&p.preview.parts).map(part).collect(),
            missing_items: p.missing_items,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(json: &str) -> Object {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn an_object_is_one_thing() {
        assert_eq!(object(r#"{"placeable": 4}"#).kind().unwrap(), Kind::Placeable(4));
        assert_eq!(object(r#"{"model": " PLC_A01 "}"#).kind().unwrap(), Kind::Model("plc_a01".into()));
        assert_eq!(object(r#"{"creature": "a.utc"}"#).kind().unwrap().name(), "creature");
        let e = object(r#"{"placeable": 4, "item": "a.uti"}"#).kind().unwrap_err().to_string();
        assert!(e.contains("not placeable and item"), "{e}");
        assert!(object(r#"{"model": "a_name_far_too_long"}"#).check().is_err());
        assert!(object(r#"{"placeable": 1, "t": -1}"#).check().is_err());
    }

    #[test]
    fn a_facing_is_the_games() {
        // Unturned, a model faces north; facing south (the default) turns it half round, so its front (+y) points to -y.
        let o = object(r#"{"placeable": 1, "position": [5, 6, 1]}"#);
        let front = o.matrix().transform_vector3(Vec3::Y);
        assert!((front - Vec3::NEG_Y).length() < 1e-5, "{front}");
        let east = object(r#"{"placeable": 1, "facing": 0, "scale": 2}"#).matrix().transform_vector3(Vec3::Y);
        assert!((east - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5, "facing east, twice the size: {east}");
        assert_eq!(o.matrix().transform_point3(Vec3::ZERO), Vec3::new(5.0, 6.0, 1.0));
    }
}
