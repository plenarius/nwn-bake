// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Opening the game and resolving textures, on a made-up install (no game content): hak order, the user folder's haks by name, a
//! missing hak, a module's hak list, and what a texture name means through an EE material.

use std::path::{Path, PathBuf};

use mg_core::{ResRef, ResType};
use nwn_bake::game::{self, GameOptions, Haks, Tlk};
use nwn_bake::texture::{self, Kind};

/// A 2x2 uncompressed 32-bit TGA of one colour.
fn tga(rgba: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 8];
    for _ in 0..4 {
        out.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
    }
    out
}

fn hak(path: &Path, files: &[(&str, ResType, Vec<u8>)]) {
    let mut w = mg_erf::ErfWriter::new(*b"HAK ");
    for (name, t, data) in files {
        w.add(ResRef::from_str(name).unwrap(), *t, data.clone()).unwrap();
    }
    std::fs::write(path, w.to_bytes().unwrap()).unwrap();
}

/// An install (only its talk table) and a user folder with two haks that both have the material `shared`, each naming its own image.
fn install(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("nwn-bake-game-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (root, user) = (base.join("game"), base.join("user"));
    std::fs::create_dir_all(root.join("lang/en/data")).unwrap();
    std::fs::create_dir_all(user.join("hak")).unwrap();
    std::fs::write(root.join("lang/en/data/dialog.tlk"), mg_tlk::Tlk::new(mg_core::Language::ENGLISH).to_bytes().unwrap()).unwrap();
    hak(
        &user.join("hak/a_first.hak"),
        &[
            ("shared", ResType::MTR, b"texture0 img_a\n".to_vec()),
            ("img_a", ResType::TGA, tga([255, 0, 0, 255])),
            // A coloured part's texture: a PLT and a plain image of the same name.
            ("part01", ResType::PLT, b"PLT V1  ".to_vec()),
            ("part01", ResType::TGA, tga([9, 9, 9, 255])),
            // A material that names its own name.
            ("selfish", ResType::MTR, b"texture0 selfish\n".to_vec()),
            ("selfish", ResType::TGA, tga([1, 2, 3, 255])),
            // A material whose image is nowhere.
            ("lost", ResType::MTR, b"texture0 nowhere\n".to_vec()),
            ("lost", ResType::TGA, tga([4, 5, 6, 255])),
        ],
    );
    hak(
        &user.join("hak/b_second.hak"),
        &[("shared", ResType::MTR, b"texture0 img_b\n".to_vec()), ("img_b", ResType::TGA, tga([0, 0, 255, 255]))],
    );
    (base, root, user)
}

fn open(root: &Path, user: &Path, haks: Haks) -> game::Game {
    game::open(&GameOptions { root: Some(root.to_path_buf()), user: Some(user.to_path_buf()), haks, tlk: Tlk::Auto }).unwrap()
}

#[test]
fn the_first_hak_in_a_module_s_list_wins() {
    let (base, root, user) = install("order");
    let image = |g: &game::Game| texture::resolve(g, "shared").unwrap().image_name();
    assert_eq!(image(&open(&root, &user, Haks::List(vec!["b_second".into(), "a_first".into()]))), "img_b");
    assert_eq!(image(&open(&root, &user, Haks::List(vec!["a_first".into(), "b_second".into()]))), "img_a");
    // Every hak in the user folder, by name: a_first before b_second.
    assert_eq!(image(&open(&root, &user, Haks::AllInUser)), "img_a");
    // No haks: no material, and the name means itself.
    let none = open(&root, &user, Haks::None);
    assert_eq!(image(&none), "shared");
    assert!(texture::image(&none, "shared").is_none());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_hak_found_nowhere_is_reported_and_the_rest_are_opened() {
    let (base, root, user) = install("missing");
    let mut g = open(&root, &user, Haks::List(vec!["nope".into(), "b_second".into()]));
    assert_eq!(g.missing_haks(), ["nope".to_string()]);
    assert_eq!(texture::resolve(&g, "shared").unwrap().image_name(), "img_b");
    // A hak added later is under the ones already there.
    assert!(g.add_haks(&["a_first".into(), "gone".into()]).unwrap() == ["gone".to_string()]);
    assert_eq!(g.missing_haks(), ["nope".to_string(), "gone".to_string()]);
    assert_eq!(texture::resolve(&g, "shared").unwrap().image_name(), "img_b");
    assert!(texture::image(&g, "part01").is_some());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_texture_name_means_what_the_game_draws() {
    let (base, root, user) = install("texture");
    let g = open(&root, &user, Haks::AllInUser);
    // Through the material: its texture0, decoded (rows from the bottom).
    let rgba = texture::rgba(&g, "shared").unwrap();
    assert_eq!((rgba.width, rgba.height, &rgba.data[..4]), (2, 2, &[255, 0, 0, 255][..]));
    // A coloured part takes its PLT; anything else the plain image first.
    let part = texture::resolve(&g, "part01").unwrap();
    assert_eq!((part.kind(&g, true), part.kind(&g, false)), (Some(Kind::Plt), Some(Kind::Plain)));
    // A material that names itself: the name's own image.
    assert_eq!(texture::resolve(&g, "selfish").unwrap().image_name(), "selfish");
    assert_eq!(&texture::rgba(&g, "selfish").unwrap().data[..4], &[1, 2, 3, 255]);
    // A material whose image is nowhere: nothing to draw (the game does not fall back to the name's own image).
    let lost = texture::resolve(&g, "lost").unwrap();
    assert_eq!((lost.image_name().as_str(), lost.kind(&g, false)), ("nowhere", None));
    assert!(texture::image(&g, "lost").is_none());
    // Unknown names and names that cannot be resource names.
    assert!(texture::resolve(&g, "unknown").unwrap().kind(&g, false).is_none());
    assert!(texture::resolve(&g, "").is_none());
    // As PNG.
    assert!(texture::png(&g, "shared", None).unwrap().starts_with(b"\x89PNG"));
    let _ = std::fs::remove_dir_all(base);
}
