// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `nwn-bake area-3d` end to end, as a user runs it, on the generated fixture (`nwn-bake/tests/support/fixture.rs`; no game content):
//! the `.glb`, the floor file, the manifest (checked against `docs/area-3d.schema.json`) and the WebP textures; the second run finds
//! everything current; an area file on its own; the texture folder elsewhere; the exit codes.

#[path = "../../nwn-bake/tests/support/fixture.rs"]
mod fixture;

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn run(f: &fixture::Fixture, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.arg("area-3d").args(args).arg("--root").arg(&f.root).arg("--user").arg(&f.user);
    for v in ["NWN_BAKE_USER", "NWN_BAKE_TLK", "NWN_ROOT", "NWN_HOME", "NWN_BAKE_COMMIT", "NWN_BAKE_JOBS", "NWN_BAKE_CACHE_MB"] {
        c.env_remove(v);
    }
    c.output().unwrap()
}

fn lines(o: &Output) -> Vec<Value> {
    String::from_utf8(o.stdout.clone()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn ok(o: &Output) -> Vec<Value> {
    assert!(o.status.success(), "exit {:?}\n{}", o.status.code(), String::from_utf8_lossy(&o.stderr));
    lines(o)
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// Fails with every error when `m` does not follow `docs/area-3d.schema.json`.
fn valid(m: &Value) {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/area-3d.schema.json")).unwrap();
    let v = jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap();
    let errors: Vec<String> = v.iter_errors(m).map(|e| format!("{e} at {}", e.instance_path())).collect();
    assert!(errors.is_empty(), "{errors:#?}\n{m:#}");
}

#[test]
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the baker names its files in lower case; the test holds it to that")]
fn area_3d_writes_a_glb_a_manifest_and_webp_textures_and_reuses_them() {
    let f = fixture::write(&fixture::dir("cli-a3d"));
    let out = f.base.join("out");
    let first = ok(&run(&f, &[s(&f.module_mod()), "fx_main", "fx_mark", "--out", s(&out), "--jobs", "2"]));
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|l| l["action"] == "baked"), "{first:?}");
    let m: Value = serde_json::from_slice(&std::fs::read(out.join("fx_main.json")).unwrap()).unwrap();
    assert_eq!(m["format"], "nwn-bake/area-3d");
    valid(&m);
    // Named as in the area pictures' manifest.
    assert_eq!((m["renderer"]["name"].as_str(), m["area"]["size_m"].is_array()), (Some("nwn-bake"), true));
    assert_eq!(m["options"]["webp_quality"], 70.0);
    assert_eq!(m["tex_uri"], "tex/");
    assert_eq!(m["doors"][0]["tag"], "fx_door");
    let glb = std::fs::read(out.join("fx_main.glb")).unwrap();
    assert_eq!(&glb[..4], b"glTF");
    assert_eq!(m["glb"]["bytes"], glb.len());
    // The walkable floor beside it: 3x2 tiles, 49 by 33 samples.
    let floor = nwn_bake::area::FloorFile::read(&std::fs::read(out.join("fx_main.floor")).unwrap()).unwrap();
    assert_eq!((m["version"].as_u64(), m["floor"]["bytes"].as_u64()), (Some(2), Some(16 + 3 * 49 * 33)));
    assert_eq!((floor.nx, floor.ny), (49, 33));
    for t in m["textures"].as_array().unwrap() {
        let name = t["file"].as_str().unwrap();
        assert!(name.ends_with(".webp") && name.len() == 25, "{name}");
        let bytes = std::fs::read(out.join("tex").join(name)).unwrap();
        assert_eq!((&bytes[..4], &bytes[8..12]), (&b"RIFF"[..], &b"WEBP"[..]), "{name} is WebP");
        assert_eq!(nwn_bake_name(&bytes), name, "named by its content");
    }
    // Again: everything is current.
    let again = ok(&run(&f, &[s(&f.module_mod()), "fx_main", "fx_mark", "--out", s(&out)]));
    assert!(again.iter().all(|l| l["action"] == "current"), "{again:?}");
    assert_eq!(std::fs::read(out.join("fx_main.glb")).unwrap(), glb);

    // An area file on its own, the textures in a folder beside the output's: the same glb but for the texture URIs.
    let are = f.project().join("src").join("areas").join("fx_main.are.json");
    let are = if are.is_file() { are } else { find(&f.project(), "fx_main.are.json") };
    let other = f.base.join("glb");
    let lone = ok(&run(&f, &[s(&are), "--out", s(&other), "--tex-dir", s(&f.base.join("shared-tex"))]));
    assert_eq!(lone[0]["area"], "fx_main");
    let m2: Value = serde_json::from_slice(&std::fs::read(other.join("fx_main.json")).unwrap()).unwrap();
    assert_eq!(m2["tex_uri"], "../shared-tex/");
    valid(&m2);
    assert_eq!(m2["textures"], m["textures"]);
    assert_eq!(std::fs::read(other.join("fx_main.floor")).unwrap(), std::fs::read(out.join("fx_main.floor")).unwrap());

    // An area that isn't there fails (exit 3), the others are baked; a wrong option is misuse (exit 2) and stops it.
    let bad = run(&f, &[s(&f.module_mod()), "fx_cave", "nope", "--out", s(&out)]);
    assert_eq!(bad.status.code(), Some(3));
    let l = lines(&bad);
    assert_eq!((l[0]["action"].as_str(), l[1]["action"].as_str()), (Some("baked"), Some("failed")));
    assert_eq!(run(&f, &[s(&f.module_mod()), "fx_main", "--out", s(&out), "--objects", "maybe"]).status.code(), Some(2));
    assert_eq!(run(&f, &[s(&f.module_mod()), "fx_main"]).status.code(), Some(2), "--out is needed");
    assert_eq!(run(&f, &[s(&f.module_mod()), "fx_main", "--all", "--out", s(&out)]).status.code(), Some(2), "--all names no area");
    let _ = std::fs::remove_dir_all(&f.base);
}

/// What `nwn_bake::area3d::content_name` names a WebP file.
fn nwn_bake_name(bytes: &[u8]) -> String {
    nwn_bake::area3d::content_name(bytes, "webp")
}

fn find(dir: &Path, name: &str) -> std::path::PathBuf {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            let f = find(&p, name);
            if f.is_file() {
                return f;
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return p;
        }
    }
    dir.join("missing")
}

#[test]
fn a_batch_ends_when_its_output_is_closed() {
    use std::process::Stdio;
    let f = fixture::write(&fixture::dir("cli-a3d-closed"));
    let out = f.base.join("out");
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.arg("area-3d").args([s(&f.module_mod()), "--all", "--out", s(&out), "--jobs", "3"]);
    c.arg("--root").arg(&f.root).arg("--user").arg(&f.user).stdout(Stdio::piped()).stderr(Stdio::piped());
    for v in ["NWN_BAKE_USER", "NWN_BAKE_TLK", "NWN_ROOT", "NWN_HOME", "NWN_BAKE_COMMIT", "NWN_BAKE_JOBS", "NWN_BAKE_CACHE_MB"] {
        c.env_remove(v);
    }
    let mut child = c.spawn().unwrap();
    // (`| head` that has stopped reading, and `2>&-`.)
    drop(child.stdout.take());
    drop(child.stderr.take());
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed().as_secs() > 120 {
            let _ = child.kill();
            panic!("area-3d still running after two minutes with its output closed");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(0));
    assert!(out.join("fx_main.glb").is_file());
    let _ = std::fs::remove_dir_all(&f.base);
}

/// `--draft` (a layout that is not a file) run with stdin as the draft.
fn run_stdin(f: &fixture::Fixture, args: &[&str], stdin: &str) -> Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.arg("area-3d").args(args).arg("--root").arg(&f.root).arg("--user").arg(&f.user);
    for v in ["NWN_BAKE_USER", "NWN_BAKE_TLK", "NWN_ROOT", "NWN_HOME", "NWN_BAKE_COMMIT", "NWN_BAKE_JOBS", "NWN_BAKE_CACHE_MB"] {
        c.env_remove(v);
    }
    let mut child = c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

/// fx_main's own layout as a draft's tiles (`fixture::layout`).
fn fx_main_tiles() -> Vec<Value> {
    let (_, _, _, tiles) = fixture::layout("fx_main");
    tiles.iter().map(|&(id, o, z)| serde_json::json!({"tileid": id, "orientation": o, "height": z})).collect()
}

#[test]
fn a_draft_bakes_as_its_area_would() {
    let f = fixture::write(&fixture::dir("cli-a3d-draft"));
    let module = f.module_mod();
    let (normal, drafts) = (f.base.join("normal"), f.base.join("drafts"));
    let n = ok(&run(&f, &[s(&module), "fx_main", "--out", s(&normal)]));

    // fx_main's own layout as a draft of fx_main: its files byte for byte, so the same key, .glb and floor.
    let own = f.base.join("own.json");
    std::fs::write(&own, serde_json::json!({"width": 3, "height": 2, "tiles": fx_main_tiles()}).to_string()).unwrap();
    let d = ok(&run(&f, &[s(&module), "fx_main", "--draft", s(&own), "--out", s(&drafts), "--force"]));
    assert_eq!((d.len(), d[0]["area"].as_str(), d[0]["action"].as_str()), (1, Some("fx_main"), Some("baked")));
    assert_eq!(d[0]["key"], n[0]["key"]);
    for file in ["fx_main.glb", "fx_main.floor"] {
        assert_eq!(std::fs::read(drafts.join(file)).unwrap(), std::fs::read(normal.join(file)).unwrap(), "{file}");
    }

    // Grown one tile west, from stdin, under its own name: a 4x2 area with the door moved 10 m east; then current.
    let mut tiles = fx_main_tiles();
    tiles.insert(3, serde_json::json!({"tileid": 2, "orientation": 0}));
    tiles.insert(0, serde_json::json!({"tileid": 2, "orientation": 0}));
    let grown =
        serde_json::json!({"name": "fx_grown", "tileset": "fx01", "width": 4, "height": 2, "origin_dx": 1, "tiles": tiles}).to_string();
    let g = ok(&run_stdin(&f, &[s(&module), "fx_main", "--draft", "-", "--out", s(&drafts)], &grown));
    assert_eq!((g[0]["area"].as_str(), g[0]["action"].as_str()), (Some("fx_grown"), Some("baked")));
    assert_ne!(g[0]["key"], n[0]["key"]);
    let m: Value = serde_json::from_slice(&std::fs::read(drafts.join("fx_grown.json")).unwrap()).unwrap();
    valid(&m);
    assert_eq!(m["area"]["tiles"], serde_json::json!([4, 2]));
    assert_eq!(m["doors"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        (m["doors"][0]["tag"].as_str(), m["doors"][0]["x"].as_f64(), m["doors"][0]["y"].as_f64()),
        (Some("fx_door"), Some(35.0), Some(15.0))
    );
    let floor = nwn_bake::area::FloorFile::read(&std::fs::read(drafts.join("fx_grown.floor")).unwrap()).unwrap();
    assert_eq!((floor.nx, floor.ny), (65, 33));
    let again = ok(&run_stdin(&f, &[s(&module), "fx_main", "--draft", "-", "--out", s(&drafts)], &grown));
    assert_eq!(again[0]["action"], "current");

    // No template: a bare area of the draft's tileset, named `draft`.
    let bare = serde_json::json!({"tileset": "fx01", "width": 2, "height": 2, "tiles": [
        {"tileid": 4, "orientation": 0}, {"tileid": 4, "orientation": 1}, {"tileid": 2, "orientation": 0}, {"tileid": 4, "orientation": 0},
    ]})
    .to_string();
    let b = ok(&run_stdin(&f, &[s(&module), "--draft", "-", "--out", s(&drafts)], &bare));
    assert_eq!((b[0]["area"].as_str(), b[0]["action"].as_str()), (Some("draft"), Some("baked")));
    let mb: Value = serde_json::from_slice(&std::fs::read(drafts.join("draft.json")).unwrap()).unwrap();
    assert_eq!(mb["doors"].as_array().map(Vec::len), Some(0));

    // A draft that does not fit fails before the game is opened (exit 1, saying why); two templates or --all are misuse.
    let short = run_stdin(&f, &[s(&module), "fx_main", "--draft", "-", "--out", s(&drafts)], r#"{"width": 3, "height": 2, "tiles": []}"#);
    assert_eq!(short.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&short.stderr).contains("0 tiles for 3x2"), "{}", String::from_utf8_lossy(&short.stderr));
    let other = run_stdin(&f, &[s(&module), "fx_main", "--draft", "-", "--out", s(&drafts)], &bare.replace("fx01", "fx02"));
    assert!(String::from_utf8_lossy(&other.stderr).contains("not its template's (fx01)"));
    assert_eq!(run(&f, &[s(&module), "fx_main", "fx_mark", "--draft", s(&own), "--out", s(&drafts)]).status.code(), Some(2));
    assert_eq!(run(&f, &[s(&module), "--all", "--draft", s(&own), "--out", s(&drafts)]).status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&f.base);
}
