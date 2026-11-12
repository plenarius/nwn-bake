// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Batches and the cache (`nwn_bake::iso::batch`) on the generated fixture (`support/fixture.rs`; no game content): every area
//! drawn once, a second run drawing nothing and leaving every file as it was, a changed `.git` drawing that area only, a changed
//! hak entry that no area reads drawing nothing, a changed tileset texture drawing the areas that read it, pictures coming back from
//! the cache, the same files whatever the number of threads, the keys, WebP and depth pictures, and `serve`'s two requests.

#[path = "support/fixture.rs"]
mod fixture;

use std::collections::BTreeMap;
use std::path::Path;

use mg_core::ResType;
use nwn_bake::game::{self, Game, GameOptions, Haks, Tlk};
use nwn_bake::iso::batch::{self, Action, AreaRequest, FileDigests, Job, Service};
use nwn_bake::iso::{Format, IsoOptions, OutOptions, View};
use nwn_bake::module::ModuleSource;
use serde_json::Value;

const ALL: [View; 5] = [View::Sw, View::Se, View::Ne, View::Nw, View::Top];

fn open(f: &fixture::Fixture) -> Game {
    game::open(&GameOptions {
        root: Some(f.root.clone()),
        user: Some(f.user.clone()),
        haks: Haks::List(fixture::HAKS.map(String::from).to_vec()),
        tlk: Tlk::None,
    })
    .unwrap()
}

/// The format the batch tests write: WebP when this build has it.
fn format() -> Format {
    Format::best()
}

fn job(out: &Path, cache: Option<&Path>, jobs: usize) -> Job {
    // (No shadows: the fixture's pictures are small, and the cache logic is the same.)
    let iso = IsoOptions { views: ALL.to_vec(), shadows: false, ..Default::default() };
    Job {
        iso,
        objects: true,
        out: OutOptions::tree(out, format()),
        cache: cache.map(Path::to_path_buf),
        force: false,
        strict: false,
        jobs,
        index: true,
        tlk: None,
    }
}

/// Runs a batch of every area of the fixture's nasher project, as a new process would (the game opened anew, the haks' digests
/// read from the cache); returns each area's action.
fn run(f: &fixture::Fixture, job: &Job) -> BTreeMap<String, Action> {
    let g = open(f);
    let source = ModuleSource::open(&f.project()).unwrap();
    let files = FileDigests::at(job.cache.as_ref().map(|c| c.join("v1").join("files.json")));
    let results = batch::run(&g, &source, &source.areas(), job, Some(&files), &|_| {}).unwrap();
    for r in &results {
        assert!(r.error.is_none(), "{}: {:?}", r.resref, r.error);
    }
    results.into_iter().map(|r| (r.resref, r.action)).collect()
}

fn all(action: Action) -> BTreeMap<String, Action> {
    fixture::AREAS.iter().map(|a| (a.to_string(), action)).collect()
}

/// Every file under `dir` (relative path, bytes, modification time).
fn files(dir: &Path) -> BTreeMap<String, (Vec<u8>, std::time::SystemTime)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.insert(rel, (std::fs::read(&p).unwrap(), p.metadata().unwrap().modified().unwrap()));
            }
        }
    }
    out
}

fn read_json(p: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

fn schema(name: &str) -> jsonschema::Validator {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs").join(name)).unwrap();
    jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap()
}

fn valid(v: &jsonschema::Validator, m: &Value) {
    let errors: Vec<String> = v.iter_errors(m).map(|e| format!("{e} at {}", e.instance_path())).collect();
    assert!(errors.is_empty(), "{errors:#?}\n{m:#}");
}

/// Rewrites `fx_tiles.hak` with `change` applied to its entries.
fn change_tiles_hak(f: &fixture::Fixture, change: impl FnOnce(&mut Vec<(String, ResType, Vec<u8>)>)) {
    let mut entries = fixture::tiles_hak();
    change(&mut entries);
    fixture::write_hak(&f.user.join("hak/fx_tiles.hak"), &entries);
}

#[test]
fn a_batch_draws_every_area_once_and_a_second_run_draws_nothing() {
    let f = fixture::write(&fixture::dir("batch"));
    let (out, cache) = (f.base.join("out"), f.base.join("cache"));
    let job = job(&out, Some(&cache), 3);
    assert_eq!(run(&f, &job), all(Action::Rendered));

    // The tree layout: <format>/<view>/<area>.<format>, json/<area>.json, manifest.jsonl, index.json.
    let (manifest, line) = (schema("area-iso.schema.json"), schema("area-iso-line.schema.json"));
    let ext = format().ext();
    for area in fixture::AREAS {
        let m = read_json(&out.join(format!("json/{area}.json")));
        valid(&manifest, &m);
        for k in ["key", "quick_key", "resources"] {
            assert!(m[k].as_str().is_some_and(|s| s.starts_with("sha256:")), "{area}: {k} {}", m[k]);
        }
        assert_eq!((m["options"]["format"].as_str(), m["options"]["depth"].as_bool()), (Some(ext), Some(false)));
        for (v, view) in m["views"].as_array().unwrap().iter().zip(ALL) {
            let file = format!("{ext}/{}/{area}.{ext}", view.name());
            assert_eq!(v["file"].as_str(), Some(file.as_str()));
            assert_eq!(v["bytes"].as_u64(), Some(std::fs::metadata(out.join(&file)).unwrap().len()));
        }
    }
    let jsonl = std::fs::read_to_string(out.join("manifest.jsonl")).unwrap();
    let lines: Vec<Value> = jsonl.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), fixture::AREAS.len() * 5);
    let mut sorted = fixture::AREAS.to_vec();
    sorted.sort_unstable();
    for (i, l) in lines.iter().enumerate() {
        valid(&line, l);
        assert_eq!(
            (l["resref"].as_str(), l["view"].as_str(), l["status"].as_str()),
            (Some(sorted[i / 5]), Some(ALL[i % 5].name()), Some("ok"))
        );
        let m = read_json(&out.join(format!("json/{}.json", sorted[i / 5])));
        assert_eq!(l["transform"]["matrix"], m["views"][i % 5]["matrix"]);
        assert_eq!(l["path"], m["views"][i % 5]["file"]);
        assert_eq!(l["key"], m["key"]);
    }
    let index = read_json(&out.join("index.json"));
    assert_eq!(index["areas"].as_array().unwrap().len(), 6);
    assert!(index["areas"].as_array().unwrap().iter().all(|a| a["status"] == "ok" && a["key"].is_string()));

    // Again: nothing drawn, nothing written (the files keep their bytes and their times), the same keys.
    let before = files(&out);
    assert_eq!(run(&f, &job), all(Action::Current));
    assert_eq!(files(&out), before);
    // Without the cache too (the output folder's manifests have the keys).
    assert_eq!(run(&f, &Job { cache: None, ..job.clone() }), all(Action::Current));
    assert_eq!(files(&out), before);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the baker names its files in lower case; the test holds it to that")]
fn a_changed_git_draws_its_area_only() {
    let f = fixture::write(&fixture::dir("batch-git"));
    let (out, cache) = (f.base.join("out"), f.base.join("cache"));
    let job = job(&out, Some(&cache), 2);
    run(&f, &job);
    let before = files(&out);
    // The marker moved a metre east.
    let mut git = fixture::git("fx_mark").unwrap();
    let Some(mg_gff::Value::List(list)) = git.root.get_mut("Placeable List") else { panic!("no placeables") };
    list[0] = fixture::placeable(1, [fixture::MARKER[0] + 1.0, fixture::MARKER[1], 0.0], 0.0);
    std::fs::write(f.project().join("src/areas/fx_mark.git.json"), fixture::json(&git)).unwrap();
    let mut want = all(Action::Current);
    want.insert("fx_mark".into(), Action::Rendered);
    assert_eq!(run(&f, &job), want);
    // Only fx_mark's files changed.
    let after = files(&out);
    for (name, (bytes, time)) in &before {
        let changed = after[name].0 != *bytes || after[name].1 != *time;
        let expected = name.contains("fx_mark") || name == "manifest.jsonl" || name == "index.json";
        assert!(!changed || expected, "{name} changed");
        if name.ends_with(".webp") || name.ends_with(".png") {
            assert_eq!(changed, name.contains("fx_mark"), "{name}");
        }
    }
    // Moved back: the cache has those pictures.
    std::fs::write(f.project().join("src/areas/fx_mark.git.json"), fixture::json(&fixture::git("fx_mark").unwrap())).unwrap();
    want.insert("fx_mark".into(), Action::Cached);
    assert_eq!(run(&f, &job), want);
    for (name, (bytes, _)) in &before {
        if !name.ends_with(".jsonl") && !name.ends_with("index.json") {
            assert_eq!(&files(&out)[name].0, bytes, "{name}");
        }
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the baker names its files in lower case; the test holds it to that")]
fn a_hak_change_no_area_reads_draws_nothing_and_a_tileset_change_draws_the_areas_that_read_it() {
    let f = fixture::write(&fixture::dir("batch-hak"));
    let (out, cache) = (f.base.join("out"), f.base.join("cache"));
    let job = job(&out, Some(&cache), 3);
    run(&f, &job);
    let keys = |area: &str| {
        let m = read_json(&out.join(format!("json/{area}.json")));
        (m["quick_key"].as_str().unwrap().to_string(), m["key"].as_str().unwrap().to_string())
    };
    let before: Vec<_> = fixture::AREAS.iter().map(|a| keys(a)).collect();
    let pictures = |files: &BTreeMap<String, (Vec<u8>, std::time::SystemTime)>| {
        files
            .iter()
            .filter(|(n, _)| !n.starts_with("json") && !n.ends_with(".jsonl") && !n.ends_with("index.json"))
            .map(|(n, v)| (n.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let pics = pictures(&files(&out));

    // A texture no area uses, added to the tileset's hak: every quick key changes, every exact key stays, nothing is drawn and no
    // picture is written.
    change_tiles_hak(&f, |e| e.push(("fx_unused".into(), ResType::TGA, fixture::plain([1, 2, 3]))));
    assert_eq!(run(&f, &job), all(Action::Current));
    for (a, (q, k)) in fixture::AREAS.iter().zip(&before) {
        let (q2, k2) = keys(a);
        assert_ne!(&q2, q, "{a}: the quick key did not change");
        assert_eq!(&k2, k, "{a}: the exact key changed");
    }
    assert_eq!(pictures(&files(&out)), pics);
    // And again: the new quick keys are known now, so not even the scenes are built.
    assert_eq!(run(&f, &job), all(Action::Current));

    // The stone texture changed: the areas that draw stone (fx_main's rock through its material, fx_flat's ceilings, fx_cave's rock
    // top, the pit's walls in fx_chasm) are drawn again; fx_mark and fx_step (grass and void) are not.
    change_tiles_hak(&f, |e| {
        e.push(("fx_unused".into(), ResType::TGA, fixture::plain([1, 2, 3])));
        let stone = e.iter_mut().find(|(n, t, _)| n == "fx_stone" && *t == ResType::TGA).unwrap();
        stone.2 = fixture::checker([90, 20, 20], [70, 10, 10]);
    });
    let mut want = all(Action::Rendered);
    want.insert("fx_mark".into(), Action::Current);
    want.insert("fx_step".into(), Action::Current);
    assert_eq!(run(&f, &job), want);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn pictures_come_back_from_the_cache_and_the_threads_change_nothing() {
    let f = fixture::write(&fixture::dir("batch-threads"));
    let cache = f.base.join("cache");
    let one = f.base.join("one");
    assert_eq!(run(&f, &job(&one, None, 1)), all(Action::Rendered));
    // Five threads, into the cache: the same files.
    let five = f.base.join("five");
    assert_eq!(run(&f, &job(&five, Some(&cache), 5)), all(Action::Rendered));
    let bytes = |d: &Path| files(d).into_iter().map(|(n, (b, _))| (n, b)).collect::<BTreeMap<_, _>>();
    let (b1, b5) = (bytes(&one), bytes(&five));
    assert_eq!(b1.keys().collect::<Vec<_>>(), b5.keys().collect::<Vec<_>>());
    for (n, b) in &b1 {
        assert!(b == &b5[n], "{n} differs between one thread and five");
    }
    // A new output folder, the same cache: every area from the cache, the same files.
    let again = f.base.join("again");
    assert_eq!(run(&f, &job(&again, Some(&cache), 2)), all(Action::Cached));
    assert_eq!(bytes(&again), b5);
    // --force draws them anyway, and they are the same.
    assert_eq!(run(&f, &Job { force: true, ..job(&again, Some(&cache), 4) }), all(Action::Rendered));
    assert_eq!(bytes(&again), b5);
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn the_keys_follow_the_options_and_the_inputs() {
    let f = fixture::write(&fixture::dir("batch-keys"));
    let key = |job: &Job, area: &str| {
        run(&f, job);
        let m = read_json(&job.out.dir.join(job.out.manifest(area)));
        (m["quick_key"].as_str().unwrap().to_string(), m["key"].as_str().unwrap().to_string(), m["resources"].as_str().unwrap().to_string())
    };
    let base = job(&f.base.join("a"), None, 2);
    let a = key(&base, "fx_main");
    // The same inputs in another folder: the same keys.
    assert_eq!(key(&job(&f.base.join("b"), None, 3), "fx_main"), a);
    // Another option: other keys, the same resources.
    let mut scaled = job(&f.base.join("c"), None, 2);
    scaled.iso.scale = 6.0;
    let c = key(&scaled, "fx_main");
    assert!(c.0 != a.0 && c.1 != a.1 && c.2 == a.2, "{a:?} {c:?}");
    // Another area: other keys.
    let d = key(&base, "fx_mark");
    assert!(d.0 != a.0 && d.1 != a.1 && d.2 != a.2);
    // The renderer counts by its name and the pictures' version only, not the crate's version or the build's commit, so a new
    // build that draws the same pictures keeps the keys.
    assert_eq!(batch::renderer_id(), format!("nwn-bake pictures {}", nwn_bake::iso::PICTURES_VERSION));
    let m = read_json(&base.out.dir.join(base.out.manifest("fx_main")));
    assert_eq!(m["renderer"]["pictures"].as_u64(), Some(u64::from(nwn_bake::iso::PICTURES_VERSION)));
    let _ = std::fs::remove_dir_all(&f.base);
}

#[cfg(feature = "webp")]
#[test]
fn webp_pictures_keep_their_alpha_and_show_the_marker_where_the_matrix_puts_it() {
    let f = fixture::write(&fixture::dir("batch-webp"));
    let out = f.base.join("out");
    let job = Job { iso: IsoOptions { views: ALL.to_vec(), ..Default::default() }, ..job(&out, None, 3) };
    run(&f, &job);
    let m = read_json(&out.join("json/fx_mark.json"));
    assert_eq!((m["options"]["format"].as_str(), m["options"]["webp_quality"].as_f64()), (Some("webp"), Some(85.0)));
    for v in m["views"].as_array().unwrap() {
        let bytes = std::fs::read(out.join(v["file"].as_str().unwrap())).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");
        let img = webp::Decoder::new(&bytes).decode().unwrap();
        assert!(img.is_alpha());
        assert_eq!((Some(u64::from(img.width())), Some(u64::from(img.height()))), (v["width"].as_u64(), v["height"].as_u64()));
        let rgba: &[u8] = &img;
        assert_eq!(rgba[3], 0, "the corner is transparent");
        let want = fixture::project(
            &serde_json::from_value(v["matrix"].clone()).unwrap(),
            [fixture::MARKER[0], fixture::MARKER[1], fixture::MARKER[2] + fixture::MARKER_UP],
        );
        // (Strictly red: lossy WebP leaves a trace of red in the ground's colours everywhere.)
        let (mut sum, mut n) = ([0.0f32; 2], 0.0f32);
        for (i, p) in rgba.as_chunks::<4>().0.iter().enumerate() {
            if p[0] > 150 && p[1] < 90 && p[2] < 90 && p[3] > 128 {
                sum = [sum[0] + (i as u32 % img.width()) as f32 + 0.5, sum[1] + (i as u32 / img.width()) as f32 + 0.5];
                n += 1.0;
            }
        }
        assert!(n > 0.0, "{}: no marker", v["view"]);
        let got = [sum[0] / n, sum[1] / n];
        let off = ((got[0] - want[0]).powi(2) + (got[1] - want[1]).powi(2)).sqrt();
        // (Lossy: the red bleeds a little into its neighbours.)
        assert!(off < 1.0, "{}: the marker at {got:?}, the matrix says {want:?}", v["view"]);
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn depth_pictures_give_the_height_of_what_each_pixel_shows() {
    let f = fixture::write(&fixture::dir("batch-depth"));
    let out = f.base.join("out");
    let job = Job {
        iso: IsoOptions { views: vec![View::Sw, View::Top], shadows: false, depth: true, ..Default::default() },
        ..job(&out, None, 2)
    };
    run(&f, &job);
    let m = read_json(&out.join("json/fx_mark.json"));
    valid(&schema("area-iso.schema.json"), &m);
    assert_eq!(m["options"]["depth"].as_bool(), Some(true));
    for v in m["views"].as_array().unwrap() {
        let file = v["depth"].as_str().unwrap();
        assert_eq!(file, format!("depth/{}/fx_mark.png", v["view"].as_str().unwrap()));
        let range: [f32; 2] = serde_json::from_value(v["depth_range"].clone()).unwrap();
        let mut d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(out.join(file)).unwrap()));
        d.set_transformations(png::Transformations::IDENTITY);
        let mut r = d.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Grayscale, png::BitDepth::Sixteen));
        assert_eq!((Some(u64::from(info.width)), Some(u64::from(info.height))), (v["width"].as_u64(), v["height"].as_u64()));
        let matrix: [[f32; 4]; 2] = serde_json::from_value(v["matrix"].clone()).unwrap();
        let z_at = |p: [f32; 3]| {
            let q = fixture::project(&matrix, p);
            let i = (q[1].floor() as usize * info.width as usize + q[0].floor() as usize) * 2;
            let raw = u16::from_be_bytes([buf[i], buf[i + 1]]);
            assert_ne!(raw, 0, "nothing drawn at {p:?}");
            range[0] + f32::from(raw) / 65535.0 * (range[1] - range[0])
        };
        // The grass at 0 m, the marker's red square 1 m up.
        assert!(z_at([5.0, 15.0, 0.0]).abs() < 0.01, "{}", z_at([5.0, 15.0, 0.0]));
        let top = z_at([fixture::MARKER[0], fixture::MARKER[1], fixture::MARKER_UP]);
        assert!((top - fixture::MARKER_UP).abs() < 0.01, "{top}");
        // The corner shows nothing.
        assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), 0);
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn serve_draws_an_area_once_and_tells_its_keys() {
    let f = fixture::write(&fixture::dir("batch-serve"));
    let (out, cache) = (f.base.join("out"), f.base.join("cache"));
    let service = Service::new(Some(f.root.clone()), Some(f.user.clone()));
    let req = |extra: Value| -> AreaRequest {
        let mut r = serde_json::json!({ "module": f.project(), "area": "fx_main", "views": ["sw", "top"], "shadows": false, "out": out, "cache": cache });
        r.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(r).unwrap()
    };
    let first = service.area_iso(&req(serde_json::json!({}))).unwrap();
    valid(&schema("area-iso.schema.json"), &first);
    assert_eq!(first["action"].as_str(), Some("rendered"));
    let ext = format().ext();
    assert!(out.join(format!("{ext}/top/fx_main.{ext}")).is_file() && out.join("json/fx_main.json").is_file());
    let again = service.area_iso(&req(serde_json::json!({}))).unwrap();
    assert_eq!((again["action"].as_str(), &again["key"]), (Some("current"), &first["key"]));
    let keys = service.area_iso_key(&req(serde_json::json!({}))).unwrap();
    assert_eq!(
        (&keys["key"], &keys["quick_key"], keys["cached"].as_bool(), keys["current"].as_bool()),
        (&first["key"], &first["quick_key"], Some(true), Some(true))
    );
    // The area by its files (the haks of the nearest module.ifo): the same keys; another scale, other keys and nothing cached.
    let files = req(serde_json::json!({ "module": null, "area": null, "are": f.project().join("src/areas/fx_main.are.json") }));
    let by_files = service.area_iso_key(&AreaRequest { module: None, area: None, ..files }).unwrap();
    assert_eq!((&by_files["key"], &by_files["area"]), (&first["key"], &first["area"]["resref"]));
    let other = service.area_iso_key(&req(serde_json::json!({ "scale": 4 }))).unwrap();
    assert!(other["key"] != first["key"] && other["cached"] == false && other["current"] == false);
    // A request that is wrong says so.
    assert!(service.area_iso(&req(serde_json::json!({ "area": "fx_nope" }))).is_err());
    assert!(serde_json::from_value::<AreaRequest>(serde_json::json!({ "module": "x", "colour": "red" })).is_err());
    let _ = std::fs::remove_dir_all(&f.base);
}

#[test]
fn a_nasher_project_opens_its_areas_lazily_and_gives_what_the_whole_project_gives() {
    let f = fixture::write(&fixture::dir("batch-lazy"));
    let lazy = ModuleSource::open(&f.project()).unwrap();
    let whole = ModuleSource::open(&f.module_mod()).unwrap();
    assert!(lazy.module().is_none());
    assert_eq!(lazy.areas(), whole.areas());
    assert_eq!((lazy.haks(), lazy.tlk()), (whole.haks(), whole.tlk()));
    for a in fixture::AREAS {
        let (x, y) = (lazy.area(a).unwrap(), whole.area(a).unwrap());
        assert_eq!(x.are.to_bytes().unwrap(), y.are.to_bytes().unwrap(), "{a}");
        assert_eq!(x.git.map(|g| g.to_bytes().unwrap()), y.git.map(|g| g.to_bytes().unwrap()), "{a}");
    }
    assert!(lazy.area("fx_nope").is_err());
    let _ = std::fs::remove_dir_all(&f.base);
}

/// The fixture's areas, with area `bad`'s work panicking.
struct Panics<'a> {
    source: &'a ModuleSource,
    bad: &'a str,
}

impl batch::AreaSource for Panics<'_> {
    fn area(&self, resref: &str) -> anyhow::Result<nwn_bake::module::AreaFiles> {
        assert_ne!(resref, self.bad, "a broken area");
        self.source.area(resref)
    }
}

#[test]
fn a_panic_fails_its_area_and_the_batch_ends() {
    let f = fixture::write(&fixture::dir("batch-panic"));
    let g = open(&f);
    let source = ModuleSource::open(&f.project()).unwrap();
    let areas = source.areas();
    for jobs in [1, 3] {
        let out = f.base.join(format!("out-{jobs}"));
        let job = job_in(&out, jobs);
        // An area whose work panics fails with the panic's message; the others are drawn.
        let results = batch::run(&g, &Panics { source: &source, bad: "fx_mark" }, &areas, &job, None, &|_| {}).unwrap();
        assert_eq!(results.len(), areas.len());
        for r in &results {
            if r.resref == "fx_mark" {
                assert_eq!(r.action, Action::Failed);
                assert!(r.error.as_deref().is_some_and(|e| e.contains("panicked") && e.contains("a broken area")), "{:?}", r.error);
            } else {
                assert_eq!(r.action, Action::Rendered, "{}: {:?}", r.resref, r.error);
            }
        }
        // A report that panics (a progress line to a closed stderr) loses nothing: every area is drawn and has its result.
        let again = f.base.join(format!("again-{jobs}"));
        let results = batch::run(&g, &source, &areas, &job_in(&again, jobs), None, &|r| panic!("cannot report {}", r.resref)).unwrap();
        assert!(results.iter().all(|r| r.action == Action::Rendered), "{:?}", results.iter().map(|r| &r.error).collect::<Vec<_>>());
    }
    let _ = std::fs::remove_dir_all(&f.base);
}

fn job_in(out: &Path, jobs: usize) -> Job {
    Job { index: false, ..job(out, None, jobs) }
}
