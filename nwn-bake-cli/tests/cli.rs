// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command line itself, on the generated fixture (`nwn-bake/tests/support/fixture.rs`; no game content): help, version and
//! misuse (exit code 2, before the game is opened); the user folder from `--user`, `NWN_BAKE_USER` or `NWN_HOME` for every
//! command; `serve` without a user folder, and its unknown requests; the talk table (a wrong `--tlk` is an error, a name without
//! `.tlk` is found, the name recorded is the file read); the caches' limit from `--cache-mb` or `NWN_BAKE_CACHE_MB` for every
//! command, the same pictures whatever it is; a wrong `--root`; an install in another language; and errors that name the file.

#[path = "../../nwn-bake/tests/support/fixture.rs"]
mod fixture;

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

/// `nwn-bake` with `args`, nothing from the environment that names the game, and `HOME` in `home` (so the game's default user
/// folder is not this machine's), then `env` set.
fn bake(home: &Path, args: &[&str], env: &[(&str, &Path)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.args(args);
    for v in [
        "NWN_BAKE_USER",
        "NWN_BAKE_TLK",
        "NWN_ROOT",
        "NWN_HOME",
        "NWN_USER_DIRECTORY",
        "NWN_BAKE_COMMIT",
        "NWN_BAKE_JOBS",
        "NWN_BAKE_CACHE_MB",
    ] {
        c.env_remove(v);
    }
    c.env("HOME", home);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn help_version_and_misuse() {
    let home = fixture::dir("cli-help");
    std::fs::create_dir_all(&home).unwrap();
    let help = bake(&home, &["--help"], &[]);
    assert_eq!(help.status.code(), Some(0));
    let text = String::from_utf8_lossy(&help.stdout);
    for command in ["creature", "serve", "scene", "area-iso", "area-3d", "area-scene", "item-thumb", "resources"] {
        assert!(text.contains(command), "--help lists {command}:\n{text}");
    }
    let iso = String::from_utf8_lossy(&bake(&home, &["area-iso", "--help"], &[]).stdout).into_owned();
    assert!(iso.contains("--supersample") && iso.contains("--views") && !iso.contains("--quality"), "{iso}");
    let version = bake(&home, &["--version"], &[]);
    assert_eq!(version.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&version.stdout).starts_with(&format!("nwn-bake {}", env!("CARGO_PKG_VERSION"))));
    // A bare run: the usage, exit 2; the first line is what an installer checks for.
    let bare = bake(&home, &[], &[]);
    assert_eq!(bare.status.code(), Some(2));
    assert!(stderr(&bare).starts_with("Error: usage: nwn-bake"), "{}", stderr(&bare));
    // An unknown command is misuse before the game is opened (the install named here is not there).
    let unknown = bake(&home, &["frobnicate", "--root", "/nowhere/at/all"], &[]);
    assert_eq!(unknown.status.code(), Some(2), "{}", stderr(&unknown));
    assert!(!stderr(&unknown).contains("/nowhere"), "{}", stderr(&unknown));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_user_folder_comes_from_one_rule() {
    let f = fixture::write(&fixture::dir("cli-user"));
    let (out, module) = (f.base.join("out"), f.module_mod());
    let area = |env: &[(&str, &Path)]| {
        bake(&f.base, &["area-iso", s(&module), "fx_flat", "--out", s(&out), "--view", "top", "--format", "png", "--root", s(&f.root)], env)
    };
    for var in ["NWN_BAKE_USER", "NWN_HOME"] {
        let o = area(&[(var, &f.user)]);
        assert!(o.status.success(), "{var}: {}", stderr(&o));
        let m: Value = serde_json::from_slice(&std::fs::read(out.join("fx_flat.json")).unwrap()).unwrap();
        assert_eq!(m["inputs"]["missing_haks"], serde_json::json!([]), "{var}: the haks are found in it");
    }
    let o = area(&[("NWN_BAKE_USER", &f.base.join("nope"))]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("NWN_BAKE_USER") && stderr(&o).contains("nope"), "{}", stderr(&o));
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_caches_limit_reaches_every_command_and_changes_no_picture() {
    let f = fixture::write(&fixture::dir("cli-cache-mb"));
    let module = f.module_mod();
    let game = ["--root", s(&f.root), "--user", s(&f.user)];
    // NWN_BAKE_CACHE_MB is read by every command that opens a game, `serve` too: one that is not a number is an error naming it.
    let lots: &Path = Path::new("lots");
    for args in [
        &["area-iso", s(&module), "fx_flat", "--out", s(&f.base.join("x")), "--view", "top"][..],
        &["area-3d", s(&module), "fx_flat", "--out", s(&f.base.join("x"))],
        &["area-scene", s(&module), "fx_flat"],
        &["thumb", "a.utc", "b.png"],
        &["serve"],
    ] {
        let o = bake(&f.base, &[args, &game[..]].concat(), &[("NWN_BAKE_CACHE_MB", lots)]);
        assert_eq!(o.status.code(), Some(1), "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains("NWN_BAKE_CACHE_MB=lots: not a number of MiB"), "{args:?}: {}", stderr(&o));
        // The option wins over the variable.
        let given = bake(&f.base, &[args, &game[..], &["--cache-mb", "-1"]].concat(), &[("NWN_BAKE_CACHE_MB", Path::new("64"))]);
        assert_eq!(given.status.code(), Some(2), "{args:?}: {}", stderr(&given));
    }
    // The same pictures and manifest with the default caches, with none (from the option) and with a few MiB (from the variable).
    let area = |out: &Path, extra: &[&str], env: &[(&str, &Path)]| {
        let args =
            [&["area-iso", s(&module), "fx_flat", "--out", s(out), "--view", "top", "--format", "png"][..], &game[..], extra].concat();
        let o = bake(&f.base, &args, env);
        assert!(o.status.success(), "{extra:?}: {}", stderr(&o));
        let m: Value = serde_json::from_slice(&std::fs::read(out.join("fx_flat.json")).unwrap()).unwrap();
        let png = std::fs::read(out.join(m["views"][0]["file"].as_str().unwrap())).unwrap();
        (png, m)
    };
    let default = area(&f.base.join("default"), &[], &[]);
    assert_eq!(area(&f.base.join("none"), &["--cache-mb", "0"], &[]), default);
    assert_eq!(area(&f.base.join("few"), &[], &[("NWN_BAKE_CACHE_MB", Path::new("3"))]), default);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn serve_starts_without_a_user_folder() {
    use std::io::Write;
    let f = fixture::write(&fixture::dir("cli-serve-nouser"));
    // (The module's haks in the install's data/hk/, where they are looked up when there is no user folder.)
    std::fs::create_dir_all(f.root.join("data/hk")).unwrap();
    for hak in ["fx_tiles.hak", "fx_over.hak"] {
        std::fs::copy(f.user.join("hak").join(hak), f.root.join("data/hk").join(hak)).unwrap();
    }
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.args(["serve", "--root", s(&f.root)]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped());
    c.stderr(std::process::Stdio::null()).env("HOME", &f.base);
    for v in ["NWN_BAKE_USER", "NWN_BAKE_TLK", "NWN_ROOT", "NWN_HOME", "NWN_USER_DIRECTORY", "NWN_BAKE_CACHE_MB"] {
        c.env_remove(v);
    }
    let mut child = c.spawn().unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        let req = serde_json::json!({ "module": f.project(), "area": "fx_mark", "format": "png" });
        writeln!(stdin, "{}", serde_json::json!({ "area_iso_key": req })).unwrap();
        writeln!(stdin, "{}", serde_json::json!({ "textures_of": 0 })).unwrap();
        writeln!(stdin, "{}", serde_json::json!({ "textures_off": 42 })).unwrap();
        writeln!(stdin, "{{}}").unwrap();
        // A scene is checked before the game is needed: a bad spec, and a good one with nowhere to write.
        writeln!(stdin, "{}", serde_json::json!({ "scene": { "objects": [{ "placeable": 0, "model": "fx_box" }] }, "out": "x.png" }))
            .unwrap();
        writeln!(stdin, "{}", serde_json::json!({ "scene": { "objects": [{ "placeable": 0 }] } })).unwrap();
    }
    drop(child.stdin.take());
    let o = child.wait_with_output().unwrap();
    let answers: Vec<Value> = String::from_utf8(o.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(answers[0]["ready"], true);
    assert_eq!(answers[1]["area"], "fx_mark", "{}", answers[1]);
    assert!(answers[2]["error"].as_str().unwrap().contains("user folder"), "{}", answers[2]);
    // A field that is not known, or no request at all, is an error that says so (not a look of an empty path).
    assert!(answers[3]["error"].as_str().unwrap().contains("unknown field `textures_off`"), "{}", answers[3]);
    assert!(answers[4]["error"].as_str().unwrap().starts_with("unknown request"), "{}", answers[4]);
    assert!(answers[5]["error"].as_str().unwrap().contains("not placeable and model"), "{}", answers[5]);
    assert!(answers[6]["error"].as_str().unwrap().contains("needs `out`"), "{}", answers[6]);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_talk_table_is_the_one_asked_for_and_read() {
    let f = fixture::write(&fixture::dir("cli-tlk"));
    let out = f.base.join("out");
    let module = f.module_mod();
    let area = |extra: &[&str]| {
        let args = [&["area-iso", s(&module), "fx_flat", "--out", s(&out), "--view", "top", "--format", "png"][..], extra].concat();
        bake(&f.base, &[&args[..], &["--root", s(&f.root), "--user", s(&f.user)]].concat(), &[])
    };
    let wrong = area(&["--tlk", "nope"]);
    assert_eq!(wrong.status.code(), Some(1));
    assert!(stderr(&wrong).contains("talk table nope: not found"), "{}", stderr(&wrong));
    std::fs::create_dir_all(f.user.join("tlk")).unwrap();
    std::fs::write(f.user.join("tlk/broken.tlk"), b"not a talk table").unwrap();
    let broken = area(&["--tlk", "broken"]);
    assert_eq!(broken.status.code(), Some(1));
    assert!(stderr(&broken).contains("broken.tlk: not a talk table"), "{}", stderr(&broken));
    std::fs::remove_file(f.user.join("tlk/broken.tlk")).unwrap();
    // A name without `.tlk`, as a module's Mod_CustomTlk gives it, is found and recorded.
    std::fs::write(f.user.join("tlk/fx.tlk"), mg_tlk::Tlk::new(mg_core::Language::ENGLISH).to_bytes().unwrap()).unwrap();
    let o = area(&["--tlk", "fx"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let m: Value = serde_json::from_slice(&std::fs::read(out.join("fx_flat.json")).unwrap()).unwrap();
    assert_eq!(m["inputs"]["tlk"], "fx");
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_install_is_checked_and_its_language_found() {
    let f = fixture::write(&fixture::dir("cli-root"));
    let out = f.base.join("out");
    let module = f.module_mod();
    let area = |root: &Path| {
        let args = ["area-iso", s(&module), "fx_flat", "--out", s(&out), "--view", "top", "--format", "png"];
        bake(&f.base, &[&args[..], &["--root", s(root), "--user", s(&f.user)]].concat(), &[])
    };
    let o = area(&f.user);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("not an NWN:EE install"), "{}", stderr(&o));
    // An install with only French.
    let fr = f.base.join("game-fr");
    std::fs::create_dir_all(fr.join("lang/fr/data")).unwrap();
    std::fs::copy(f.root.join("lang/en/data/dialog.tlk"), fr.join("lang/fr/data/dialog.tlk")).unwrap();
    let o = area(&fr);
    assert!(o.status.success(), "{}", stderr(&o));
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn a_scene_spec_is_checked_before_the_game_is_opened() {
    use std::io::Write;
    let home = fixture::dir("cli-scene");
    std::fs::create_dir_all(&home).unwrap();
    let spec = home.join("bad.json");
    std::fs::write(&spec, r#"{"objects": [{"placeable": 0}], "camera": {"pitch": 95}}"#).unwrap();
    // (The install named here is not there: the spec fails first.)
    let o = bake(&home, &["scene", s(&spec), s(&home.join("x.png")), "--root", "/nowhere/at/all"], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("pitch is -89.9 to 89.9"), "{}", stderr(&o));
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.args(["scene", "-", "x.png", "--root", "/nowhere/at/all"]).env("HOME", &home);
    c.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let mut child = c.spawn().unwrap();
    writeln!(child.stdin.as_mut().unwrap(), r#"{{"objects": [], "zoom": 2}}"#).unwrap();
    drop(child.stdin.take());
    let o = child.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("not a scene spec") && stderr(&o).contains("unknown field `zoom`"), "{}", stderr(&o));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_file_that_cannot_be_read_is_named() {
    let f = fixture::write(&fixture::dir("cli-read"));
    let missing = f.base.join("no-such.bic");
    let o = bake(&f.base, &["describe", s(&missing), "--root", s(&f.root), "--user", s(&f.user)], &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains(s(&missing)), "{}", stderr(&o));
    let _ = std::fs::remove_dir_all(&f.base);
}
