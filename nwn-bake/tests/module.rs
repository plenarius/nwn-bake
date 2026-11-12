// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Area sources on made-up modules (no game content): one module laid out as a `.mod`, a folder of binary files, a nasher project
//! keeping its files as nwn_gff's JSON, and single files, all read the same.

use std::path::PathBuf;

use mg_core::{Gender, Language, LocString, ResRef, ResType};
use mg_gff::{Gff, Struct, Value};
use nwn_bake::module::{ModuleSource, gff_from, read_gff};

fn text(s: &str) -> Value {
    Value::String(s.as_bytes().to_vec())
}

fn resref(s: &str) -> Value {
    Value::ResRef(s.as_bytes().to_vec())
}

/// A struct with its fields in nasher's order (labels sorted, ignoring case): a project's JSON keeps them so, so the binary it is
/// packed into and the JSON read back agree field for field.
fn item(id: u32, fields: &[(&str, Value)]) -> Struct {
    let mut fields = fields.to_vec();
    fields.sort_by_key(|(label, _)| label.to_ascii_lowercase());
    let mut s = Struct::new(id);
    for (label, v) in fields {
        s.set(label, v);
    }
    s
}

fn gff(file_type: &[u8; 4], fields: &[(&str, Value)]) -> Gff {
    let mut g = Gff::new(*file_type);
    g.root = item(u32::MAX, fields);
    g
}

/// `module.ifo`: two haks (the first listed wins in the game), a talk table and two areas, `beta` first.
fn ifo() -> Gff {
    let hak = |n: &str| item(8, &[("Mod_Hak", text(n))]);
    let area = |n: &str| item(6, &[("Area_Name", resref(n))]);
    gff(
        b"IFO ",
        &[
            ("Mod_HakList", Value::List(vec![hak("zz_top"), hak("aa_low")])),
            ("Mod_CustomTlk", text("mytlk")),
            ("Mod_Area_list", Value::List(vec![area("beta"), area("alpha")])),
        ],
    )
}

/// An area of 3 by 2 tiles; its floats have no more than four decimals, as nasher keeps them.
fn are(name: &str) -> Gff {
    let tile =
        |id: i32, o: i32| item(1, &[("Tile_ID", Value::Int(id)), ("Tile_Orientation", Value::Int(o)), ("Tile_Height", Value::Int(0))]);
    gff(
        b"ARE ",
        &[
            ("Tag", text(name)),
            ("Name", Value::LocString(LocString::from_text(Language::ENGLISH, Gender::Male, format!("Area {name}")))),
            ("Tileset", resref("tst01")),
            ("Width", Value::Int(3)),
            ("Height", Value::Int(2)),
            ("Flags", Value::Dword(1)),
            ("SunAmbientColor", Value::Dword(0x00336699)),
            ("FogClipDist", Value::Float(45.5)),
            ("Tile_List", Value::List((0..6).map(|i| tile(i, i % 4)).collect())),
        ],
    )
}

fn git() -> Gff {
    let placeable = item(
        9,
        &[
            ("Tag", text("chest")),
            ("Appearance", Value::Dword(4)),
            ("X", Value::Float(12.25)),
            ("Y", Value::Float(3.5)),
            ("Z", Value::Float(0.0)),
            // (A quarter turn to four decimals, as nasher keeps it: not the exact constant on purpose.)
            #[expect(clippy::approx_constant, reason = "a quarter turn to four decimals, as nasher keeps it")]
            ("Bearing", Value::Float(1.5708)),
        ],
    );
    gff(b"GIT ", &[("Placeable List", Value::List(vec![placeable])), ("Door List", Value::List(Vec::new()))])
}

/// nwn_gff's JSON as nasher keeps it (keys sorted, floats rounded).
fn json(g: &Gff) -> String {
    mg_gff::to_json_text(&mg_gff::to_json(g, mg_core::Codepage::WINDOWS_1252).unwrap(), mg_gff::TextStyle::NASHER)
}

fn files() -> Vec<(&'static str, ResType, Gff)> {
    vec![
        ("module", ResType::IFO, ifo()),
        ("alpha", ResType::ARE, are("alpha")),
        ("alpha", ResType::GIT, git()),
        ("beta", ResType::ARE, are("beta")),
    ]
}

/// The module in every form, under a fresh folder: `m.mod`, `folder/`, `project/` (nasher, JSON under `src/areas/`).
fn layout(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("nwn-bake-module-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("folder")).unwrap();
    std::fs::create_dir_all(base.join("project/src/areas")).unwrap();
    let mut erf = mg_erf::ErfWriter::new(*b"MOD ");
    for (name, t, g) in files() {
        let bytes = g.to_bytes().unwrap();
        erf.add(ResRef::from_str(name).unwrap(), t, bytes.clone()).unwrap();
        std::fs::write(base.join("folder").join(format!("{name}.{}", t.extension().unwrap())), &bytes).unwrap();
        let dir = if t == ResType::IFO { base.join("project") } else { base.join("project/src/areas") };
        std::fs::write(dir.join(format!("{name}.{}.json", t.extension().unwrap())), json(&g)).unwrap();
    }
    std::fs::write(base.join("m.mod"), erf.to_bytes().unwrap()).unwrap();
    std::fs::write(
        base.join("project/nasher.cfg"),
        "[package]\nname = \"test\"\n\n[sources]\ninclude = \"src/**/*.json\"\ninclude = \"module.ifo.json\"\n\n[target]\nname = \"default\"\nfile = \"test.mod\"\n",
    )
    .unwrap();
    base
}

#[test]
fn every_form_of_a_module_reads_the_same() {
    let base = layout("forms");
    let haks = ["zz_top".to_string(), "aa_low".to_string()];
    for form in ["m.mod", "folder", "project", "project/nasher.cfg"] {
        let m = ModuleSource::open(&base.join(form)).unwrap_or_else(|e| panic!("{form}: {e:#}"));
        assert_eq!(m.haks(), haks, "{form}");
        assert_eq!(m.tlk(), Some("mytlk"), "{form}");
        assert_eq!(m.areas(), ["beta", "alpha"], "{form}");
        let a = m.area("ALPHA").unwrap();
        assert_eq!((a.resref.as_str(), &a.are, a.git.as_ref()), ("alpha", &are("alpha"), Some(&git())), "{form}");
        let b = m.area("beta").unwrap();
        assert_eq!((&b.are, b.git.is_none()), (&are("beta"), true), "{form}");
        assert!(m.area("gamma").is_err(), "{form}");
    }
    // One area file, binary or JSON: that area, its .git beside it, the haks of the module.ifo(.json) above it.
    for (form, areas) in [("folder/alpha.are", ["alpha"]), ("project/src/areas/alpha.are.json", ["alpha"])] {
        let m = ModuleSource::open(&base.join(form)).unwrap();
        assert_eq!((m.haks(), m.tlk(), m.areas()), (&haks[..], Some("mytlk"), areas.map(String::from).to_vec()), "{form}");
        let a = m.area("alpha").unwrap();
        assert_eq!((&a.are, a.git.as_ref()), (&are("alpha"), Some(&git())), "{form}");
        assert!(m.area("beta").is_err(), "{form}");
    }
    let beta = ModuleSource::open(&base.join("project/src/areas/beta.are.json")).unwrap();
    assert!(beta.area("beta").unwrap().git.is_none());
    // module.ifo or module.ifo.json on its own: the haks and the talk table, no areas.
    for form in ["folder/module.ifo", "project/module.ifo.json"] {
        let m = ModuleSource::open(&base.join(form)).unwrap();
        assert_eq!((m.haks(), m.tlk(), m.areas().len()), (&haks[..], Some("mytlk"), 0), "{form}");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// A nasher project whose packed target leaves a folder of areas out, and a second target that lists them: opened by that target's
/// name, the project has those areas too, with the same haks and talk table; a target it does not have, or a target named for
/// something that is no nasher project, is an error.
#[test]
fn a_nasher_project_opens_by_target() {
    let base = layout("target");
    let project = base.join("project");
    std::fs::create_dir_all(project.join("houses")).unwrap();
    std::fs::write(project.join("houses/gamma.are.json"), json(&are("gamma"))).unwrap();
    std::fs::write(project.join("houses/gamma.git.json"), json(&git())).unwrap();
    std::fs::write(
        project.join("nasher.cfg"),
        "[package]\nname = \"test\"\n\n[sources]\ninclude = \"src/**/*.json\"\ninclude = \"houses/*.json\"\ninclude = \"module.ifo.json\"\n\n\
         [target]\nname = \"default\"\nfile = \"test.mod\"\ninclude = \"src/**/*\"\ninclude = \"module.ifo.json\"\n\n\
         [target]\nname = \"bake\"\nfile = \"test_bake.mod\"\ninclude = \"src/**/*\"\ninclude = \"houses/*\"\ninclude = \"module.ifo.json\"\n",
    )
    .unwrap();
    let packed = ModuleSource::open(&project).unwrap();
    assert_eq!(packed.areas(), ["beta", "alpha"]);
    assert!(format!("{:#}", packed.area("gamma").unwrap_err()).contains("has no area gamma"));
    for form in ["project", "project/nasher.cfg"] {
        let m = ModuleSource::open_target(&base.join(form), Some("bake")).unwrap_or_else(|e| panic!("{form}: {e:#}"));
        assert_eq!((m.haks(), m.tlk()), (packed.haks(), packed.tlk()), "{form}");
        // (Those the area list names, in its order, then the others by name.)
        assert_eq!(m.areas(), ["beta", "alpha", "gamma"], "{form}");
        let g = m.area("gamma").unwrap();
        assert_eq!((&g.are, g.git.as_ref()), (&are("gamma"), Some(&git())), "{form}");
        assert_eq!(m.area("alpha").unwrap().are, packed.area("alpha").unwrap().are, "{form}");
    }
    assert_eq!(ModuleSource::open_target(&project, None).unwrap().areas(), packed.areas());
    assert!(format!("{:#}", ModuleSource::open_target(&project, Some("nope")).err().unwrap()).contains("no target nope"));
    for form in ["m.mod", "folder", "project/src/areas/alpha.are.json"] {
        let e = ModuleSource::open_target(&base.join(form), Some("bake")).err().unwrap_or_else(|| panic!("{form}: opened"));
        assert!(format!("{e:#}").contains("not a nasher project"), "{form}: {e:#}");
    }
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_plain_erf_of_areas_has_no_haks() {
    let base = layout("erf");
    let mut erf = mg_erf::ErfWriter::new(*b"ERF ");
    erf.add(ResRef::from_str("alpha").unwrap(), ResType::ARE, are("alpha").to_bytes().unwrap()).unwrap();
    std::fs::write(base.join("areas.erf"), erf.to_bytes().unwrap()).unwrap();
    let m = ModuleSource::open(&base.join("areas.erf")).unwrap();
    assert_eq!((m.haks().len(), m.tlk(), m.areas()), (0, None, vec!["alpha".to_string()]));
    assert_eq!(m.area("alpha").unwrap().are, are("alpha"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_gff_reads_the_same_from_json_and_binary() {
    let base = layout("gff");
    let p = |s: &str| -> PathBuf { base.join(s) };
    assert_eq!(read_gff(&p("project/src/areas/alpha.git.json")).unwrap(), read_gff(&p("folder/alpha.git")).unwrap());
    assert_eq!(gff_from("x.are.json", json(&are("x")).as_bytes()).unwrap(), are("x"));
    assert_eq!(gff_from("x.are", &are("x").to_bytes().unwrap()).unwrap(), are("x"));
    assert!(gff_from("x.are.json", b"{ not json").is_err());
    let _ = std::fs::remove_dir_all(base);
}

/// Sets a file's modification time to `secs` after 2020-01-01.
fn touch(p: &std::path::Path, secs: u64) {
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800 + secs);
    std::fs::File::options().write(true).open(p).unwrap().set_modified(at).unwrap();
}

/// A folder of nwn_gff's JSON files with no nasher.cfg, in subfolders, binary files among them; a resource found twice is read
/// from the file changed last, with a warning.
#[test]
fn a_folder_of_json_files_without_nasher_cfg_reads_as_a_module() {
    let base = std::env::temp_dir().join(format!("nwn-bake-module-test-jsonfolder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let f = base.join("mixed");
    for d in ["areas/a", "areas/b", "areas/old", ".nasher/cache"] {
        std::fs::create_dir_all(f.join(d)).unwrap();
    }
    let write = |rel: &str, bytes: Vec<u8>, at: u64| {
        std::fs::write(f.join(rel), bytes).unwrap();
        touch(&f.join(rel), at);
    };
    write("module.ifo.json", json(&ifo()).into_bytes(), 10);
    // alpha: JSON, newer than a stale binary copy elsewhere.
    write("areas/a/alpha.are.json", json(&are("alpha")).into_bytes(), 20);
    write("areas/a/alpha.git.json", json(&git()).into_bytes(), 20);
    write("areas/old/alpha.are", are("stale").to_bytes().unwrap(), 5);
    // beta: binary, newer than a stale JSON copy.
    write("areas/b/beta.are", are("beta").to_bytes().unwrap(), 30);
    write("areas/old/beta.are.json", json(&are("stale")).into_bytes(), 6);
    // Left alone: a dot folder, a file that is no resource, JSON that is no GFF type's.
    write(".nasher/cache/gamma.are", are("gamma").to_bytes().unwrap(), 40);
    write("README.md", b"notes".to_vec(), 1);
    write("areas/b/strings.tlk.json", b"{}".to_vec(), 1);

    let m = ModuleSource::open(&f).unwrap();
    assert_eq!((m.haks(), m.tlk()), (&["zz_top".to_string(), "aa_low".to_string()][..], Some("mytlk")));
    assert_eq!(m.areas(), ["beta", "alpha"]);
    let a = m.area("alpha").unwrap();
    assert_eq!((&a.are, a.git.as_ref()), (&are("alpha"), Some(&git())));
    let b = m.area("beta").unwrap();
    assert_eq!((&b.are, b.git.is_none()), (&are("beta"), true));
    assert!(m.area("gamma").is_err());
    // The warnings name paths with the platform's separator: `/` here, so the test reads the same on Windows.
    let mut w: Vec<String> = m.warnings().iter().map(|s| s.replace('\\', "/")).collect();
    w.sort();
    assert_eq!(w.len(), 2, "{w:?}");
    assert!(
        w[0].starts_with("alpha.are is in") && w[0].contains("areas/a/alpha.are.json, changed last, is read; not areas/old/alpha.are"),
        "{w:?}"
    );
    assert!(
        w[1].starts_with("beta.are is in") && w[1].contains("areas/b/beta.are, changed last, is read; not areas/old/beta.are.json"),
        "{w:?}"
    );
    // The same as the module's .mod.
    let erf = layout("jsonfolder-mod");
    let packed = ModuleSource::open(&erf.join("m.mod")).unwrap();
    for r in ["alpha", "beta"] {
        let (x, y) = (m.area(r).unwrap(), packed.area(r).unwrap());
        assert_eq!((x.are, x.git), (y.are, y.git), "{r}");
    }
    let _ = std::fs::remove_dir_all(base);
    let _ = std::fs::remove_dir_all(erf);
}
