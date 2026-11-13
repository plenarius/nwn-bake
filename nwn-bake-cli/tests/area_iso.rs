// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `nwn-bake area-iso` and `area-scene` end to end, as a user runs them, on the generated fixture (`nwn-bake/tests/support/fixture.rs`;
//! no game content): the PNGs of all five views and their manifests, checked against the schema and against each other (the
//! marker drawn where the manifest's matrix puts it), the same pictures from every module form, the options, and the exit codes.

#[path = "../../nwn-bake/tests/support/fixture.rs"]
mod fixture;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use sha2::Digest;

const VIEWS: [&str; 5] = ["sw", "se", "ne", "nw", "top"];

/// Runs `nwn-bake` with `args`, the fixture's install and user folder, and nothing from the environment that would change them.
fn run(f: &fixture::Fixture, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.args(args).arg("--root").arg(&f.root).arg("--user").arg(&f.user);
    for v in ["NWN_BAKE_USER", "NWN_BAKE_TLK", "NWN_ROOT", "NWN_HOME", "NWN_BAKE_COMMIT", "NWN_BAKE_CACHE_MB"] {
        c.env_remove(v);
    }
    c.output().unwrap()
}

fn ok(o: &Output) -> Vec<Value> {
    assert!(o.status.success(), "exit {:?}\n{}", o.status.code(), String::from_utf8_lossy(&o.stderr));
    lines(o)
}

/// stdout's JSON lines.
fn lines(o: &Output) -> Vec<Value> {
    String::from_utf8(o.stdout.clone()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

/// A PNG: (width, height, RGBA rows from the top).
fn read_png(p: &Path) -> (u32, u32, Vec<u8>) {
    let mut d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).unwrap()));
    d.set_transformations(png::Transformations::EXPAND);
    let mut r = d.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        t => panic!("{}: {t:?}", p.display()),
    };
    (info.width, info.height, rgba)
}

fn schema() -> jsonschema::Validator {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/area-iso.schema.json")).unwrap();
    jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap()
}

fn valid(v: &jsonschema::Validator, m: &Value) {
    let errors: Vec<String> = v.iter_errors(m).map(|e| format!("{e} at {}", e.instance_path())).collect();
    assert!(errors.is_empty(), "{errors:#?}\n{m:#}");
}

fn matrix(view: &Value) -> [[f32; 4]; 2] {
    serde_json::from_value(view["matrix"].clone()).unwrap()
}

const ABSENT: &str = "tileset fx01: [TERRAIN2] is counted but absent (read as empty, as the game does)";

#[test]
fn area_iso_draws_all_five_views_and_writes_manifests_that_match_them() {
    let f = fixture::write(&fixture::dir("cli-iso"));
    let out = f.base.join("out");
    let lines =
        ok(&run(&f, &["area-iso", s(&f.project()), "fx_main", "fx_mark", "--out", s(&out), "--view", "all,top", "--format", "png"]));
    assert_eq!(lines.len(), 2);
    let v = schema();
    for (line, area) in lines.iter().zip(["fx_main", "fx_mark"]) {
        assert_eq!((line["area"].as_str(), line["status"].as_str()), (Some(area), Some("ok")), "{line}");
        assert_eq!(line["views"].as_array().unwrap().len(), 5);
        let m = read_json(&out.join(format!("{area}.json")));
        valid(&v, &m);
        assert_eq!(PathBuf::from(line["manifest"].as_str().unwrap()), out.join(format!("{area}.json")));
        assert_eq!((m["format"].as_str(), m["version"].as_u64()), (Some("nwn-bake/area-iso"), Some(1)));
        let a = &m["area"];
        assert_eq!(a["name"].as_str(), Some(format!("Fixture {area}").as_str()));
        assert_eq!((a["tileset"].as_str(), a["kind"].as_str()), (Some("fx01"), Some("surface")));
        assert_eq!((&a["tiles"], &a["size_m"]), (&serde_json::json!([3, 2]), &serde_json::json!([30.0, 20.0])));
        // The .are and .git digests are those of the binary files (the project's JSON packed is the same bytes).
        let sha = |p: PathBuf| format!("sha256:{:x}", sha2::Sha256::digest(std::fs::read(p).unwrap()));
        assert_eq!(m["inputs"]["are"].as_str().unwrap(), sha(f.folder().join(format!("{area}.are"))));
        assert_eq!(m["inputs"]["git"].as_str().unwrap(), sha(f.folder().join(format!("{area}.git"))));
        assert_eq!(m["inputs"]["haks"], serde_json::json!(fixture::HAKS));
        assert_eq!((m["inputs"]["missing_haks"].as_array().unwrap().len(), m["roof_cut_m"].is_null()), (0, true));
        assert_eq!(m["notes"], serde_json::json!([ABSENT]));
        assert_eq!(
            m["options"],
            serde_json::json!({ "scale": 8.0, "quality": 2, "light": "day", "shadows": true, "roof": "auto", "exposure": "auto", "objects": true, "background": "transparent", "look": "default", "format": "png", "depth": false })
        );
        // The exact key (a run of named areas with no cache makes no quick key).
        assert!(m["key"].as_str().is_some_and(|k| k.starts_with("sha256:")) && m["quick_key"].is_null());
        assert_eq!(line["key"], m["key"]);
        // Each view's picture, as big as its entry says, with something drawn and a transparent corner.
        let views = m["views"].as_array().unwrap();
        assert_eq!(views.iter().map(|x| x["view"].as_str().unwrap()).collect::<Vec<_>>(), VIEWS);
        for view in views {
            let file = view["file"].as_str().unwrap();
            assert_eq!(file, format!("{area}.{}.png", view["view"].as_str().unwrap()));
            let (w, h, rgba) = read_png(&out.join(file));
            assert_eq!((Some(u64::from(w)), Some(u64::from(h))), (view["width"].as_u64(), view["height"].as_u64()), "{file}");
            assert!(rgba.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count() > (w * h / 4) as usize, "{file}: little drawn");
            assert_eq!(rgba[3], 0, "{file}: the top-left corner is not transparent");
            // The marker, read from the PNG, where the manifest's matrix puts it.
            if area == "fx_mark" {
                let centre = [fixture::MARKER[0], fixture::MARKER[1], fixture::MARKER[2] + fixture::MARKER_UP];
                let want = fixture::project(&matrix(view), centre);
                let got = fixture::red_centroid(w, h, &rgba).unwrap_or_else(|| panic!("{file}: no marker"));
                let off = ((got[0] - want[0]).powi(2) + (got[1] - want[1]).powi(2)).sqrt();
                assert!(off < 0.5, "{file}: the marker is drawn at {got:?}, the manifest's matrix says {want:?} ({off:.2} px off)");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn every_module_form_gives_the_same_pictures_and_manifests() {
    let f = fixture::write(&fixture::dir("cli-forms"));
    let mut forms = f.forms();
    forms.extend(f.area_files("fx_main"));
    let mut first: Option<(String, Value, Vec<Vec<u8>>)> = None;
    for (i, (form, path)) in forms.iter().enumerate() {
        let out = f.base.join(format!("out{i}"));
        ok(&run(&f, &["area-iso", s(path), "fx_main", "--out", s(&out), "--view", "top,sw", "--format", "png"]));
        let m = read_json(&out.join("fx_main.json"));
        let pictures: Vec<Vec<u8>> = ["top", "sw"].iter().map(|v| std::fs::read(out.join(format!("fx_main.{v}.png"))).unwrap()).collect();
        match &first {
            None => first = Some((form.to_string(), m, pictures)),
            Some((name, m0, p0)) => {
                assert_eq!(&m, m0, "{form} and {name}: the manifests differ");
                assert!(&pictures == p0, "{form} and {name}: the pictures differ");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_options_are_drawn_and_written_into_the_manifest() {
    let f = fixture::write(&fixture::dir("cli-options"));
    let out = f.base.join("out");
    let (look, module) = (f.base.join("look.json"), f.module_mod());
    std::fs::write(&look, r#"{ "exposure_target": 180 }"#).unwrap();
    let args = [
        "area-iso",
        s(&module),
        "fx_flat",
        "fx_cave",
        "--out",
        s(&out),
        "--view",
        "ne,top",
        "--scale",
        "4",
        "--supersample",
        "1",
        "--background",
        "#102030",
        "--roof",
        "keep",
        "--light",
        "area",
        "--shadows",
        "off",
        "--exposure",
        "off",
        "--objects",
        "off",
        "--name",
        "{view}/{area}.{ext}",
        "--look",
        s(&look),
        "--format",
        "png",
    ];
    ok(&run(&f, &args));
    let v = schema();
    for area in ["fx_flat", "fx_cave"] {
        let m = read_json(&out.join(format!("{area}.json")));
        valid(&v, &m);
        let o = &m["options"];
        assert_eq!((o["scale"].as_f64(), o["quality"].as_u64(), o["light"].as_str()), (Some(4.0), Some(1), Some("area")));
        assert_eq!((o["shadows"].as_bool(), o["exposure"].as_str(), o["objects"].as_bool()), (Some(false), Some("off"), Some(false)));
        assert_eq!((o["roof"].as_str(), o["background"].as_str()), (Some("keep"), Some("#102030")));
        assert_eq!(o["look"]["exposure_target"].as_f64(), Some(180.0));
        assert!(m["roof_cut_m"].is_null());
        for (view, name) in m["views"].as_array().unwrap().iter().zip(["ne", "top"]) {
            assert_eq!(view["file"].as_str().unwrap(), format!("{name}/{area}.png"));
            let (w, h, rgba) = read_png(&out.join(view["file"].as_str().unwrap()));
            assert_eq!((u64::from(w), u64::from(h)), (view["width"].as_u64().unwrap(), view["height"].as_u64().unwrap()));
            assert!(rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "an opaque background");
            assert_eq!(rgba[..4], [0x10, 0x20, 0x30, 255], "the corner shows the background");
        }
    }
    // The roof given in metres, and the cave's own cut.
    ok(&run(&f, &["area-iso", s(&f.module_mod()), "fx_flat", "--out", s(&out), "--view", "top", "--roof", "1.25", "--format", "png"]));
    assert_eq!(read_json(&out.join("fx_flat.json"))["roof_cut_m"].as_f64(), Some(1.25));
    ok(&run(&f, &["area-iso", s(&f.module_mod()), "fx_cave", "--out", s(&out), "--view", "top", "--format", "png"]));
    let cave = read_json(&out.join("fx_cave.json"));
    assert_eq!((cave["roof_cut_m"].as_f64(), cave["area"]["kind"].as_str()), (Some(2.0), Some("underground")));
    valid(&v, &cave);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn a_missing_hak_is_a_note_and_strict_fails_the_area() {
    let f = fixture::write(&fixture::dir("cli-haks"));
    let out = f.base.join("out");
    let haks = ["--haks", "fx_over,fx_gone,fx_tiles"];
    let first = ok(&run(&f, &[&["area-iso", s(&f.folder()), "fx_main", "--out", s(&out)][..], &haks].concat()));
    assert_eq!(first[0]["status"].as_str(), Some("ok"));
    let m = read_json(&out.join("fx_main.json"));
    valid(&schema(), &m);
    assert_eq!(
        (&m["inputs"]["haks"], &m["inputs"]["missing_haks"]),
        (&serde_json::json!(["fx_over", "fx_gone", "fx_tiles"]), &serde_json::json!(["fx_gone"]))
    );
    assert!(m["notes"].as_array().unwrap().iter().any(|n| n == "hak fx_gone not found"), "{}", m["notes"]);
    // With --strict the area fails: exit code 3, and its line says so.
    let o = run(&f, &[&["area-iso", s(&f.folder()), "fx_mark", "--out", s(&out), "--strict"][..], &haks].concat());
    assert_eq!(o.status.code(), Some(3));
    assert_eq!(lines(&o)[0]["status"].as_str(), Some("failed"));
    assert!(!out.join("fx_mark.json").exists());
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn an_area_that_is_not_there_fails_and_the_others_are_drawn() {
    let f = fixture::write(&fixture::dir("cli-fail"));
    let out = f.base.join("out");
    let o = run(&f, &["area-iso", s(&f.module_mod()), "fx_nope", "fx_step", "--out", s(&out), "--view", "top", "--format", "png"]);
    assert_eq!(o.status.code(), Some(3), "{}", String::from_utf8_lossy(&o.stderr));
    let l = lines(&o);
    assert_eq!((l[0]["area"].as_str(), l[0]["status"].as_str()), (Some("fx_nope"), Some("failed")));
    assert_eq!((l[1]["area"].as_str(), l[1]["status"].as_str()), (Some("fx_step"), Some("ok")));
    assert!(out.join("fx_step.top.png").is_file());
    // Options that are wrong are misuse (exit 2) and stop it before anything is drawn.
    for bad in [
        &["--supersample", "9"][..],
        &["--quality", "9"],
        &["--scale", "big"],
        &["--view", "up"],
        &["--format", "gif"],
        &["--background", "blue"],
        &["--roof", "high"],
        &["--jobs", "0"],
        &["--layout", "deep"],
        &["--all"],
    ] {
        let o = run(&f, &[&["area-iso", s(&f.module_mod()), "fx_step", "--out", s(&f.base.join("bad"))][..], bad].concat());
        assert_eq!(o.status.code(), Some(2), "{bad:?}: {}", String::from_utf8_lossy(&o.stderr));
    }
    assert!(!f.base.join("bad").exists());
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn all_draws_every_area_as_webp_in_folders_and_a_second_run_draws_nothing() {
    let f = fixture::write(&fixture::dir("cli-all"));
    let (out, cache, project) = (f.base.join("out"), f.base.join("cache"), f.project());
    let args =
        ["area-iso", s(&project), "--all", "--out", s(&out), "--cache", s(&cache), "--view", "all,top", "--jobs", "3", "--shadows", "off"];
    let first = ok(&run(&f, &args));
    assert_eq!(first.iter().map(|l| l["area"].as_str().unwrap()).collect::<Vec<_>>(), fixture::AREAS);
    assert!(first.iter().all(|l| l["action"] == "rendered"), "{first:?}");
    let v = schema();
    for area in fixture::AREAS {
        let m = read_json(&out.join(format!("json/{area}.json")));
        valid(&v, &m);
        assert_eq!(m["options"]["format"].as_str(), Some("webp"));
        for view in VIEWS {
            let bytes = std::fs::read(out.join(format!("webp/{view}/{area}.webp"))).unwrap();
            assert_eq!((&bytes[..4], &bytes[8..12]), (&b"RIFF"[..], &b"WEBP"[..]));
        }
    }
    let lines = std::fs::read_to_string(out.join("manifest.jsonl")).unwrap();
    assert_eq!(lines.lines().count(), 30);
    let stamp = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
    let before = (stamp(&out.join("webp/sw/fx_main.webp")), stamp(&out.join("manifest.jsonl")));
    let second = ok(&run(&f, &args));
    assert!(second.iter().all(|l| l["action"] == "current"), "{second:?}");
    assert_eq!((stamp(&out.join("webp/sw/fx_main.webp")), stamp(&out.join("manifest.jsonl"))), before);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn serve_answers_area_requests() {
    use std::io::Write;
    let f = fixture::write(&fixture::dir("cli-serve"));
    let out = f.base.join("out");
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.args(["serve", "--root", s(&f.root), "--user", s(&f.user)])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    for v in ["NWN_BAKE_USER", "NWN_BAKE_TLK", "NWN_ROOT", "NWN_HOME", "NWN_BAKE_COMMIT", "NWN_BAKE_CACHE_MB"] {
        c.env_remove(v);
    }
    let mut child = c.spawn().unwrap();
    let req = serde_json::json!({ "module": f.project(), "area": "fx_mark", "views": ["top"], "out": out, "format": "png" });
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", serde_json::json!({ "area_iso": req })).unwrap();
        writeln!(stdin, "{}", serde_json::json!({ "area_iso_key": req })).unwrap();
        writeln!(stdin, "{}", serde_json::json!({ "area_iso": { "module": f.project(), "area": "fx_nope", "out": out } })).unwrap();
    }
    drop(child.stdin.take());
    let o = child.wait_with_output().unwrap();
    let answers = lines(&o);
    assert_eq!(answers[0]["ready"], true);
    let m = &answers[1];
    valid(&schema(), m);
    assert_eq!((m["action"].as_str(), m["views"][0]["file"].as_str()), (Some("rendered"), Some("png/top/fx_mark.png")));
    assert!(out.join("png/top/fx_mark.png").is_file());
    assert_eq!((&answers[2]["key"], answers[2]["current"].as_bool()), (&m["key"], Some(true)));
    assert!(answers[3]["error"].as_str().is_some_and(|e| e.contains("fx_nope")), "{}", answers[3]);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn area_scene_tells_the_same_scene_for_every_form() {
    let f = fixture::write(&fixture::dir("cli-scene"));
    let mut digests = Vec::new();
    for (form, path) in f.forms() {
        let l = ok(&run(&f, &["area-scene", s(&path), "fx_cave"]));
        let x = &l[0];
        assert_eq!(
            (x["area"].as_str(), x["kind"].as_str(), x["roof_cut_applies"].as_bool()),
            (Some("fx_cave"), Some("underground"), Some(true)),
            "{form}"
        );
        assert_eq!((x["roof_cut"]["at"].as_f64(), x["roof_cut"]["slab"].as_bool()), (Some(2.0), Some(true)), "{form}");
        assert_eq!((x["counts"]["tiles"].as_u64(), x["counts"]["tile_triangles"].as_u64()), (Some(6), Some(12)), "{form}");
        assert_eq!(x["notes"], serde_json::json!([ABSENT]), "{form}");
        digests.push(x["digest"].as_str().unwrap().to_string());
    }
    assert!(digests.windows(2).all(|w| w[0] == w[1]), "{digests:?}");
    let _ = std::fs::remove_dir_all(&f.base);
}

/// Runs `nwn-bake` as [`run`] does, with its stdout and stderr closed as soon as it starts (`| head` that has stopped reading,
/// `2>&-`); its exit code, or a failure when it has not ended within two minutes.
fn run_with_closed_output(f: &fixture::Fixture, args: &[&str]) -> Option<i32> {
    use std::process::Stdio;
    let mut c = Command::new(env!("CARGO_BIN_EXE_nwn-bake"));
    c.args(args).arg("--root").arg(&f.root).arg("--user").arg(&f.user).stdout(Stdio::piped()).stderr(Stdio::piped());
    for v in ["NWN_BAKE_USER", "NWN_BAKE_TLK", "NWN_ROOT", "NWN_HOME", "NWN_BAKE_COMMIT", "NWN_BAKE_CACHE_MB"] {
        c.env_remove(v);
    }
    let mut child = c.spawn().unwrap();
    drop(child.stdout.take());
    drop(child.stderr.take());
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        if started.elapsed().as_secs() > 120 {
            let _ = child.kill();
            panic!("nwn-bake {args:?} still running after two minutes with its output closed");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn a_batch_ends_when_its_output_is_closed() {
    let f = fixture::write(&fixture::dir("cli-closed"));
    let out = f.base.join("out");
    // (Before, the first progress line panicked a drawing thread, and the others waited for it for ever.)
    let code = run_with_closed_output(&f, &["area-iso", s(&f.module_mod()), "--all", "--out", s(&out), "--jobs", "3"]);
    assert_eq!(code, Some(0));
    for area in fixture::AREAS {
        assert!(out.join(format!("json/{area}.json")).is_file(), "{area}");
    }
    let _ = std::fs::remove_dir_all(&f.base);
}
