// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The `.glb`s' version guard: what `area-3d` writes for the fixture's areas with the default options (each `.glb`, its manifest, its
//! floor file and its texture files) must have the digests recorded in `tests/glbs.json` for the current [`GLB_VERSION`].
//!
//! area-3d's keys hold [`GLB_VERSION`] instead of the crate's version and the build's commit, so a new build keeps every area's
//! files. That is only right while the files stay the same: a change that alters a `.glb`, a texture file or a manifest must raise
//! `GLB_VERSION`, or areas would keep files the new build no longer writes. After raising it, write the digests again:
//!
//! ```text
//! NWN_BAKE_GLBS=write cargo test -p nwn-bake --test glb_version
//! ```
//!
//! Writing refuses when the digests changed but the version is still the one recorded. A change to the fixture itself (which no
//! real area's files depend on) may be recorded without a raise: `NWN_BAKE_GLBS=write-same`.
//!
//! The textures are encoded by a stand-in (the texels behind a small header), so the test needs no WebP; a change to the WebP
//! settings must raise the version all the same. The `.glb` is compared without `asset.generator` (it names the crate's version),
//! the manifest without `renderer.version` and `renderer.commit`. Recorded on x86-64 Linux; on another platform float results may
//! differ, so the test says it is skipped there.

#[path = "support/fixture.rs"]
mod fixture;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nwn_bake::area3d::gltf::read_glb;
use nwn_bake::area3d::{self, GLB_VERSION, Inputs, Options, TextureFiles};
use nwn_bake::game::{self, GameOptions, Haks, Tlk};
use nwn_bake::module::ModuleSource;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const WRITE_AGAIN: &str = "NWN_BAKE_GLBS=write cargo test -p nwn-bake --test glb_version";

fn record_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glbs.json")
}

fn platform() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// A stand-in encoder: `RAW`, the width and height, the quality, the texels.
fn raw(img: &mg_image::Rgba, quality: f32) -> Vec<u8> {
    let mut out = b"RAW".to_vec();
    out.extend(img.width.to_le_bytes());
    out.extend(img.height.to_le_bytes());
    out.push(quality as u8);
    out.extend(&img.data);
    out
}

/// Bakes every area of the fixture with the default options; the digest of each area's `.glb`, manifest and textures, by
/// `<area>/<glb|manifest|textures>`.
fn digests(f: &fixture::Fixture, out: &Path) -> BTreeMap<String, String> {
    let g = game::open(&GameOptions {
        root: Some(f.root.clone()),
        user: Some(f.user.clone()),
        haks: Haks::List(fixture::HAKS.map(String::from).to_vec()),
        tlk: Tlk::None,
    })
    .unwrap();
    let source = ModuleSource::open(&f.module_mod()).unwrap();
    let tex = TextureFiles::new(&out.join("tex"), raw, "raw");
    let mut d = BTreeMap::new();
    for name in fixture::AREAS {
        let area = source.area(name).unwrap();
        let inputs = Inputs::of(&area, source.haks(), &[], None).unwrap();
        let baked = area3d::bake(&g, &area, &Options::default(), &tex, inputs, None).unwrap();
        let (mut j, bin) = read_glb(&baked.glb).unwrap();
        j["asset"].as_object_mut().unwrap().remove("generator");
        d.insert(format!("{name}/glb"), sha256(&[serde_json::to_vec(&j).unwrap(), bin].concat()));
        let mut m = serde_json::to_value(&baked.manifest).unwrap();
        let r = m["renderer"].as_object_mut().unwrap();
        r.remove("version");
        r.remove("commit");
        d.insert(format!("{name}/manifest"), sha256(&serde_json::to_vec(&m).unwrap()));
        let mut textures = Vec::new();
        for t in &baked.manifest.textures {
            textures.extend(t.file.as_bytes());
            textures.extend(Sha256::digest(std::fs::read(out.join("tex").join(&t.file)).unwrap()));
        }
        d.insert(format!("{name}/textures"), sha256(&textures));
        d.insert(format!("{name}/floor"), sha256(&baked.floor));
    }
    d
}

fn read_record() -> Option<Value> {
    std::fs::read(record_path()).ok().map(|b| serde_json::from_slice(&b).expect("tests/glbs.json"))
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
fn glb_version() {
    let mode = std::env::var("NWN_BAKE_GLBS").unwrap_or_default();
    let writing = mode == "write" || mode == "write-same";
    let record = read_record();
    if !writing {
        let Some(r) = &record else { panic!("{} is missing: write it with {WRITE_AGAIN}", record_path().display()) };
        if r["platform"].as_str() != Some(platform().as_str()) {
            eprintln!("glb_version: skipped (the digests were recorded on {}, this is {})", r["platform"], platform());
            return;
        }
    }

    let f = fixture::write(&fixture::dir("glbs"));
    let got = digests(&f, &f.base.join("out"));
    let _ = std::fs::remove_dir_all(&f.base);

    if writing {
        if let Some(r) = &record
            && r["glb_version"].as_u64() == Some(u64::from(GLB_VERSION))
            && r["platform"].as_str() == Some(platform().as_str())
            && recorded_digests(r) != got
            && mode != "write-same"
        {
            panic!(
                "The .glbs, manifests or textures changed, but GLB_VERSION is still {GLB_VERSION}, the version they were recorded \
                 for. Raise GLB_VERSION (nwn-bake/src/area3d.rs) first, then write the digests again. (Only when the change is to \
                 the test fixture itself, and no real area's files can change, write them with NWN_BAKE_GLBS=write-same.)\n{}",
                differences(&recorded_digests(r), &got).join("\n")
            );
        }
        let out = json!({
            "about": "Digests of the test fixture's area-3d files for this GLB_VERSION (tests/glb_version.rs). Written by the test; \
                      do not edit by hand.",
            "glb_version": GLB_VERSION,
            "platform": platform(),
            "digests": got,
        });
        std::fs::write(record_path(), format!("{}\n", serde_json::to_string_pretty(&out).unwrap())).unwrap();
        eprintln!("glb_version: wrote {} digests for GLB_VERSION {GLB_VERSION}", got.len());
        return;
    }

    let r = record.unwrap();
    let recorded = r["glb_version"].as_u64().unwrap();
    assert_eq!(
        recorded,
        u64::from(GLB_VERSION),
        "GLB_VERSION is {GLB_VERSION}, but tests/glbs.json was recorded for {recorded}. Write the digests again for the new version: \
         {WRITE_AGAIN}"
    );
    let diff = differences(&recorded_digests(&r), &got);
    assert!(
        diff.is_empty(),
        "The test fixture's area-3d files are not the ones recorded for GLB_VERSION {GLB_VERSION}:\n{}\n\n\
         A change that alters a .glb, a texture file or a manifest must raise GLB_VERSION (nwn-bake/src/area3d.rs) by one, so the \
         cache keys change and every area is baked again. Then write the digests again: {WRITE_AGAIN}\n\
         (See docs/testing.md, \"The .glbs' version\".)",
        diff.join("\n")
    );
}
