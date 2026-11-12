// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A model's particle emitters as data, for a viewer that runs the particles itself (a web viewer's particle engine, a port of
//! `mg_render::particles`). The bundle and the part files carry what a simulation needs and nothing it would compute:
//!
//! - where the emitter is (`world`: its node's transform in the bake's pose, column-major, the game's metres, Z up) and, for
//!   point-to-point and lightning emitters, where its reference child is (`target`);
//! - its kind (`update`, `render`, `blend`, `spawntype`, `flags`, `xgrid`/`ygrid`, `looping`, `renderOrder`) and texture;
//! - its parameters (`params`), as the node has them, or as the bake's animation keys them at its time when it does.
//!
//! The pose is frozen (a bundle is one pose), so an emitter does not move; its particles do. Not carried: chunk emitters (`chunk`,
//! models as particles), `Explosion` emitters' detonate keys (a frozen pose never passes one) and the scene light for tinted emitters.

use glam::Mat4;
use mg_mdl::{Model, NodeKind};
use serde_json::{Value, json};

/// The single-value parameters a viewer reads, with the default `mg_render::particles` uses when a node has none.
const PARAMS: [(&str, f32); 30] = [
    ("birthrate", 0.0),
    ("lifeexp", 1.0),
    ("mass", 0.0),
    ("velocity", 0.0),
    ("randvel", 0.0),
    ("spread", 0.0),
    ("xsize", 0.0),
    ("ysize", 0.0),
    ("sizestart", 1.0),
    ("sizeend", 1.0),
    ("sizestart_y", 0.0),
    ("sizeend_y", 0.0),
    ("alphastart", 1.0),
    ("alphaend", 1.0),
    ("fps", 0.0),
    ("framestart", 0.0),
    ("frameend", 0.0),
    ("particlerot", 0.0),
    ("blurlength", 10.0),
    ("bounce_co", 0.0),
    ("grav", 0.0),
    ("drag", 1.0),
    ("threshold", 0.0),
    ("combinetime", 0.0),
    ("p2p_bezier2", 0.0),
    ("p2p_bezier3", 0.0),
    ("lightningdelay", 0.0),
    ("lightningradius", 0.0),
    ("lightningscale", 0.0),
    ("lightningsubdiv", -1.0),
];

/// Whether a model has any emitter.
pub fn has_emitters(model: &Model) -> bool {
    model.nodes.iter().any(|n| matches!(n.kind, NodeKind::Emitter(_)))
}

/// The emitters of `model` posed by `pose` (model space) under `transform`, in animation `anim` at `t` seconds. `texture` turns an
/// emitter's texture name into what the viewer fetches (a file name, or the name itself), or None where there is no such texture.
pub fn describe(
    model: &Model,
    pose: &[Mat4],
    transform: Mat4,
    anim: &str,
    t: f32,
    texture: &mut dyn FnMut(&str) -> Option<String>,
) -> Vec<Value> {
    let animation = model.animations.iter().find(|a| a.name.eq_ignore_ascii_case(anim));
    let at = animation.map(|a| mg_render::anim::time_in(a, t));
    let mut out = Vec::new();
    for (i, node) in model.nodes.iter().enumerate() {
        let NodeKind::Emitter(em) = &node.kind else { continue };
        if em.chunk.is_some() {
            continue;
        }
        let keyed = animation.and_then(|a| a.nodes.iter().find(|n| n.name.eq_ignore_ascii_case(&node.name)));
        let value = |name: &str| -> Option<Vec<f32>> {
            if let (Some(an), Some(at)) = (keyed, at)
                && let Some(c) = an.controllers.iter().find(|c| c.name == name)
            {
                return Some(mg_render::anim::sample(c, at));
            }
            node.value(name).map(<[f32]>::to_vec)
        };
        let mut params = serde_json::Map::new();
        for (name, default) in PARAMS {
            let v = value(name).and_then(|v| v.first().copied()).unwrap_or(default);
            params.insert(name.to_string(), json!(v));
        }
        for name in ["colorstart", "colorend"] {
            let c = value(name).filter(|v| v.len() >= 3).map_or([1.0, 1.0, 1.0], |v| [v[0], v[1], v[2]]);
            params.insert(name.to_string(), json!(c));
        }
        let world = transform * pose.get(i).copied().unwrap_or(Mat4::IDENTITY);
        // The reference child that point-to-point and lightning emitters aim at.
        let target = node
            .children
            .iter()
            .copied()
            .find(|&c| matches!(model.nodes[c].kind, NodeKind::Reference(_)))
            .map(|c| transform.transform_point3(pose.get(c).map_or(glam::Vec3::ZERO, |m| m.w_axis.truncate())).to_array());
        let tex = em.texture.as_deref().filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("null")).map(str::to_ascii_lowercase);
        let file = tex.as_deref().and_then(&mut *texture);
        out.push(json!({
            "node": node.name,
            "update": em.update, "render": em.render, "blend": em.blend,
            "spawntype": em.spawntype, "flags": em.flags, "looping": em.looping, "renderOrder": em.render_order,
            "xgrid": em.xgrid.max(1), "ygrid": em.ygrid.max(1),
            "texture": tex, "file": file,
            "world": world.to_cols_array(), "target": target,
            "params": params,
        }));
    }
    out
}

/// Item property "Visual Effect" (itempropdef.2da row 83): its subtype is the iprp_visualfx.2da row.
const VISUAL_EFFECT: i64 = 83;

/// The visual-effect models of a creature's held weapons (acid, fire, holy... from the item property "Visual Effect"), as parts hung where
/// the weapon is: the game draws `<ItemClass>_fx<ModelSuffix>` (baseitems.2da, iprp_visualfx.2da) on the weapon, and those models are made
/// of particle emitters along the blade. `preview` is the creature as Moonglow composed it (its weapons are parts attached to `rhand` or
/// `lhand`, which the effect follows: the same node, scale and visual transform).
pub fn weapon_effects(game: &mg_rules::GameData, root: &mg_gff::Struct, preview: &mg_preview::Preview) -> Vec<mg_preview::Part> {
    let (Ok(baseitems), Ok(visualfx)) = (game.table("baseitems"), game.table("iprp_visualfx")) else { return Vec::new() };
    let mut out = Vec::new();
    for (slot, node) in [(0x10u32, "rhand"), (0x20u32, "lhand")] {
        let Some(s) = root.list("Equip_ItemList").and_then(|l| l.iter().find(|s| s.id == slot)) else { continue };
        // A blueprint names its item (EquippedRes); a character carries it.
        let loaded;
        let item = match s.resref("EquippedRes") {
            Some(r) if !r.is_empty() => {
                let Some(g) = game.resman.get_named(&r.to_string(), mg_core::ResType::UTI).ok().and_then(|d| mg_gff::Gff::read(&d).ok())
                else {
                    continue;
                };
                loaded = g.root;
                &loaded
            }
            _ => s,
        };
        let Some(suffix) = item
            .list("PropertiesList")
            .unwrap_or(&[])
            .iter()
            .find(|p| p.integer("PropertyName") == Some(VISUAL_EFFECT))
            .and_then(|p| visualfx.get(usize::try_from(p.integer("Subtype")?).ok()?, "ModelSuffix"))
            .filter(|v| !v.is_empty() && *v != "****")
        else {
            continue;
        };
        let Some(class) = item
            .integer("BaseItem")
            .and_then(|b| baseitems.get(usize::try_from(b).ok()?, "ItemClass"))
            .filter(|v| !v.is_empty() && *v != "****")
        else {
            continue;
        };
        let model = format!("{class}_fx{suffix}").to_ascii_lowercase();
        if game.resman.get_named(&model, mg_core::ResType::MDL).is_err() {
            continue;
        }
        // The weapon's own part on that hand (the first: a weapon may be several part models), for where and how big.
        let Some(weapon) = preview.parts.iter().find(|p| p.item && p.attach.as_deref() == Some(node)) else { continue };
        out.push(mg_preview::Part {
            model,
            attach: weapon.attach.clone(),
            scale: weapon.scale,
            textures: Default::default(),
            colors: None,
            env_map: None,
            env_map_default: false,
            material_values: Vec::new(),
            animated: weapon.animated,
            local: weapon.local,
            item: true,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use mg_mdl::{Controller, Emitter, Node, Reference};

    /// A model with a lightning emitter aiming at its reference child, and a mesh-less dummy root.
    fn model() -> Model {
        let mut root = Node::new("m", NodeKind::Dummy);
        root.children = vec![1];
        let mut e = Node::new(
            "bolt",
            NodeKind::Emitter(Emitter {
                update: "Lightning".into(),
                render: "Normal".into(),
                blend: "Lighten".into(),
                texture: Some("FXPA_Lightning".into()),
                ..Default::default()
            }),
        );
        e.parent = Some(0);
        e.children = vec![2];
        for (name, v) in [("birthrate", 4.0), ("lifeexp", 0.5), ("lightningdelay", 0.1)] {
            e.controllers.push(Controller::constant(name, &[v]));
        }
        e.controllers.push(Controller::constant("colorstart", &[0.5, 0.25, 1.0]));
        let mut r = Node::new("bolt_ref", NodeKind::Reference(Reference::default()));
        r.parent = Some(1);
        Model { name: "m".into(), nodes: vec![root, e, r], ..Default::default() }
    }

    #[test]
    fn an_emitter_is_described_where_it_is_with_its_target_and_parameters() {
        let m = model();
        let pose = vec![Mat4::IDENTITY, Mat4::from_translation(Vec3::new(1.0, 0.0, 2.0)), Mat4::from_translation(Vec3::new(1.0, 0.0, 3.0))];
        let mut asked = Vec::new();
        let out = describe(&m, &pose, Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)), "pause1", 0.0, &mut |n| {
            asked.push(n.to_string());
            Some(format!("{n}.png"))
        });
        assert_eq!(out.len(), 1);
        let e = &out[0];
        assert_eq!(e["update"], "Lightning");
        assert_eq!(asked, ["fxpa_lightning"]); // asked for lower case
        assert_eq!(e["file"], "fxpa_lightning.png");
        let world: Vec<f32> = serde_json::from_value(e["world"].clone()).unwrap();
        assert_eq!(&world[12..15], &[11.0, 0.0, 2.0]); // the instance's transform times the node's
        assert_eq!(e["target"], json!([11.0, 0.0, 3.0]));
        assert_eq!(e["params"]["birthrate"], json!(4.0));
        assert_eq!(e["params"]["colorstart"], json!([0.5, 0.25, 1.0]));
        assert_eq!(e["params"]["colorend"], json!([1.0, 1.0, 1.0])); // the default where the node has none
        assert_eq!(e["params"]["blurlength"], json!(10.0));
    }

    #[test]
    fn chunk_emitters_and_models_without_emitters_give_nothing() {
        let mut m = model();
        assert!(has_emitters(&m));
        if let NodeKind::Emitter(e) = &mut m.nodes[1].kind {
            e.chunk = Some("rock".into());
        }
        assert!(describe(&m, &[], Mat4::IDENTITY, "pause1", 0.0, &mut |_| None).is_empty());
        m.nodes[1].kind = NodeKind::Dummy;
        assert!(!has_emitters(&m));
    }
}
