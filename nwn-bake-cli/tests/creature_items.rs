// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `--items` and `--strict-items` on the made-up creature kit (`nwn-bake/tests/support/fixture.rs`, `kit_hak`; no game content),
//! through `describe`, which needs no GPU: a creature blueprint kept as nwn_gff's JSON wears an item from the folder; without the
//! folder, a warning names the item and its slot; with `--strict-items`, that is an error.

#[path = "../../nwn-bake/tests/support/fixture.rs"]
mod fixture;

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn describe_finds_a_blueprint_s_items_in_a_folder_and_says_which_it_cannot() {
    let f = fixture::write(&fixture::dir("cli-items"));
    fixture::write_hak(&f.user.join("hak/fx_kit.hak"), &fixture::kit_hak());
    let items = f.base.join("uti");
    std::fs::create_dir_all(&items).unwrap();
    std::fs::write(items.join("fx_modrod.uti.json"), fixture::json(&fixture::rod("fx_modrod", 1))).unwrap();
    let utc = f.base.join("fx_knight.utc.json");
    std::fs::write(&utc, fixture::json(&fixture::creature(&[(0x10, "fx_modrod"), (0x20, fixture::GAME_ROD)]))).unwrap();
    let describe = |extra: &[&str]| -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
        c.args(["describe", s(&utc), "--root", s(&f.root), "--user", s(&f.user)]).args(extra).env("HOME", &f.base);
        for v in ["NWN_BAKE_USER", "NWN_BAKE_TLK", "NWN_ROOT", "NWN_HOME", "NWN_USER_DIRECTORY"] {
            c.env_remove(v);
        }
        c.output().unwrap()
    };
    let models = |o: &Output| -> Vec<String> {
        let look: Value = serde_json::from_slice(&o.stdout).unwrap();
        let mut m: Vec<String> = look["parts"].as_array().unwrap().iter().map(|p| p["model"].as_str().unwrap().to_string()).collect();
        m.sort();
        m
    };
    let stderr = |o: &Output| String::from_utf8_lossy(&o.stderr).into_owned();

    let found = describe(&["--items", s(&items)]);
    assert!(found.status.success(), "{}", stderr(&found));
    assert_eq!(models(&found), ["fxrod_001", "fxrod_002"], "the folder's item and the hak's");
    assert!(!stderr(&found).contains("warning"), "{}", stderr(&found));

    let without = describe(&[]);
    assert!(without.status.success(), "{}", stderr(&without));
    assert_eq!(models(&without), ["fxrod_002"]);
    let text = stderr(&without);
    let warnings: Vec<&str> = text.lines().filter(|l| l.starts_with("warning:")).collect();
    assert_eq!(warnings, ["warning: equipped item not found, drawn without it: fx_modrod (righthand)"]);
    let look: Value = serde_json::from_slice(&without.stdout).unwrap();
    assert_eq!(look["missing_items"], serde_json::json!([{ "slot": "righthand", "resref": "fx_modrod" }]));

    let strict = describe(&["--strict-items"]);
    assert_eq!(strict.status.code(), Some(1));
    assert!(strict.stdout.is_empty());
    assert!(stderr(&strict).contains("equipped items not found: fx_modrod (righthand)"), "{}", stderr(&strict));
    assert!(describe(&["--strict-items", "--items", s(&items)]).status.success());

    let nope = describe(&["--items", s(&f.base.join("nope"))]);
    assert_eq!(nope.status.code(), Some(1));
    assert!(stderr(&nope).contains("nope: no such folder"), "{}", stderr(&nope));
    let _ = std::fs::remove_dir_all(&f.base);
}
