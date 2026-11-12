// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The pictures' version guard: the fixture's pictures and manifests, drawn with the default options (every area, all five views,
//! PNG and WebP), must have the digests recorded in `tests/pictures.json` for the current [`PICTURES_VERSION`].
//!
//! The cache keys hold the pictures' version instead of the build's commit, so a new build keeps every area's pictures. That is only
//! right while the pictures stay the same: a change that alters a picture or a manifest must raise `PICTURES_VERSION`, or areas
//! would keep pictures the new build no longer draws. This test notices when the fixture's pictures change and the version did
//! not. After raising it, write the digests again:
//!
//! ```text
//! NWN_BAKE_PICTURES=write cargo test -p nwn-bake --features webp --test pictures_version
//! ```
//!
//! Writing refuses when the digests changed but the version is still the one recorded. A change to the fixture itself (which no
//! real area's pictures depend on) may be recorded without a raise: `NWN_BAKE_PICTURES=write-same`.
//!
//! The digests are of the files' bytes. The fixture is made by code, the drawing is deterministic (one thread per view, a fixed
//! order) and libwebp encodes on one thread, so they are the same on every machine of the platform they were recorded on
//! (`platform` in the file: x86-64 Linux). On another platform float results may differ, so the test only says it is skipped there.
//! The manifests are compared without what does not describe the pictures: `renderer.version`, `renderer.commit` and `quick_key`
//! (which depends on the files' times).

#[path = "support/fixture.rs"]
mod fixture;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nwn_bake::game::{self, GameOptions, Haks, Tlk};
use nwn_bake::iso::batch::{self, Job};
use nwn_bake::iso::{Format, IsoOptions, OutOptions, PICTURES_VERSION, View};
use nwn_bake::module::ModuleSource;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const ALL: [View; 5] = [View::Sw, View::Se, View::Ne, View::Nw, View::Top];

const WRITE_AGAIN: &str = "NWN_BAKE_PICTURES=write cargo test -p nwn-bake --features webp --test pictures_version";

fn record_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pictures.json")
}

fn platform() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

fn formats() -> Vec<Format> {
    let mut out = vec![Format::Png];
    if matches!(Format::best(), Format::Webp) {
        out.push(Format::Webp);
    }
    out
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Draws every area of the fixture in `format` with the default options; the digest of each area's manifest and pictures, by
/// `<format>/<area>/<manifest|view>`.
fn digests(f: &fixture::Fixture, format: Format, out: &Path) -> BTreeMap<String, String> {
    let g = game::open(&GameOptions {
        root: Some(f.root.clone()),
        user: Some(f.user.clone()),
        haks: Haks::List(fixture::HAKS.map(String::from).to_vec()),
        tlk: Tlk::None,
    })
    .unwrap();
    let source = ModuleSource::open(&f.project()).unwrap();
    let iso = IsoOptions { views: ALL.to_vec(), ..Default::default() };
    let job = Job {
        iso,
        objects: true,
        out: OutOptions::tree(out, format),
        cache: None,
        force: false,
        strict: false,
        jobs: 4,
        index: false,
        tlk: None,
    };
    for r in batch::run(&g, &source, &source.areas(), &job, None, &|_| {}).unwrap() {
        assert!(r.error.is_none(), "{}: {:?}", r.resref, r.error);
    }
    let ext = format.ext();
    let mut d = BTreeMap::new();
    for area in fixture::AREAS {
        let mut m: Value = serde_json::from_slice(&std::fs::read(out.join(format!("json/{area}.json"))).unwrap()).unwrap();
        let views = m["views"].as_array().unwrap().clone();
        assert_eq!(views.len(), ALL.len(), "{area}");
        for (v, view) in views.iter().zip(ALL) {
            let file = v["file"].as_str().unwrap();
            d.insert(format!("{ext}/{area}/{}", view.name()), sha256(&std::fs::read(out.join(file)).unwrap()));
        }
        let o = m.as_object_mut().unwrap();
        o.remove("quick_key");
        let r = o["renderer"].as_object_mut().unwrap();
        r.remove("version");
        r.remove("commit");
        d.insert(format!("{ext}/{area}/manifest"), sha256(&serde_json::to_vec(&m).unwrap()));
    }
    d
}

fn read_record() -> Option<Value> {
    std::fs::read(record_path()).ok().map(|b| serde_json::from_slice(&b).expect("tests/pictures.json"))
}

fn recorded_digests(r: &Value) -> BTreeMap<String, String> {
    r["digests"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect()
}

/// The entries that differ (or are only on one side), one a line.
fn differences(want: &BTreeMap<String, String>, got: &BTreeMap<String, String>) -> Vec<String> {
    let keys: std::collections::BTreeSet<&String> = want.keys().chain(got.keys()).collect();
    keys.into_iter()
        .filter(|k| want.get(*k) != got.get(*k))
        .map(|k| format!("{k}: recorded {}, now {}", want.get(k).map_or("none", String::as_str), got.get(k).map_or("none", String::as_str)))
        .collect()
}

#[test]
fn pictures_version() {
    let mode = std::env::var("NWN_BAKE_PICTURES").unwrap_or_default();
    let writing = mode == "write" || mode == "write-same";
    let record = read_record();
    if !writing {
        let Some(r) = &record else { panic!("{} is missing: write it with {WRITE_AGAIN}", record_path().display()) };
        if r["platform"].as_str() != Some(platform().as_str()) {
            eprintln!("pictures_version: skipped (the digests were recorded on {}, this is {})", r["platform"], platform());
            return;
        }
    }

    let f = fixture::write(&fixture::dir("pictures"));
    let mut got = BTreeMap::new();
    for format in formats() {
        got.extend(digests(&f, format, &f.base.join(format!("out-{}", format.ext()))));
    }
    let _ = std::fs::remove_dir_all(&f.base);

    if writing {
        assert!(matches!(Format::best(), Format::Webp), "write the digests with the webp feature: {WRITE_AGAIN}");
        if let Some(r) = &record
            && r["pictures_version"].as_u64() == Some(u64::from(PICTURES_VERSION))
            && r["platform"].as_str() == Some(platform().as_str())
            && recorded_digests(r) != got
            && mode != "write-same"
        {
            panic!(
                "The pictures or manifests changed, but PICTURES_VERSION is still {PICTURES_VERSION}, the version they were recorded \
                 for. Raise PICTURES_VERSION (nwn-bake/src/iso/output.rs) first, then write the digests again. (Only when the change is \
                 to the test fixture itself, and no real area's pictures can change, write them with NWN_BAKE_PICTURES=write-same.)\n{}",
                differences(&recorded_digests(r), &got).join("\n")
            );
        }
        let out = json!({
            "about": "Digests of the test fixture's pictures and manifests for this pictures version (tests/pictures_version.rs). \
                      Written by the test; do not edit by hand.",
            "pictures_version": PICTURES_VERSION,
            "platform": platform(),
            "digests": got,
        });
        std::fs::write(record_path(), format!("{}\n", serde_json::to_string_pretty(&out).unwrap())).unwrap();
        eprintln!("pictures_version: wrote {} digests for pictures version {PICTURES_VERSION}", got.len());
        return;
    }

    let r = record.unwrap();
    let recorded = r["pictures_version"].as_u64().unwrap();
    let mut want = recorded_digests(&r);
    if !matches!(Format::best(), Format::Webp) {
        // Without the webp feature only the PNG pictures can be checked.
        want.retain(|k, _| k.starts_with("png/"));
    }
    let diff = differences(&want, &got);
    if recorded != u64::from(PICTURES_VERSION) {
        panic!(
            "PICTURES_VERSION is {PICTURES_VERSION}, but tests/pictures.json was recorded for {recorded}. Write the digests again for \
             the new version: {WRITE_AGAIN}"
        );
    }
    assert!(
        diff.is_empty(),
        "The test fixture's pictures or manifests are not the ones recorded for PICTURES_VERSION {PICTURES_VERSION}:\n{}\n\n\
         A change that alters the pictures or the manifests must raise PICTURES_VERSION (nwn-bake/src/iso/output.rs) by one, so the \
         cache keys change and every area is drawn again. Then write the digests again: {WRITE_AGAIN}\n\
         (See docs/testing.md, \"The pictures' version\".)",
        diff.join("\n")
    );
}
