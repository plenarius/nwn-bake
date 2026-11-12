// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! `cargo run --release -p nwn-bake --example module_compare -- <module A> <module B>`
//!
//! Opens two area sources (a `.mod`, a folder, a nasher project, one area file) and checks that every area of A reads the same
//! from B: its `.are` and `.git` as GFF, field by field. Prints how long each took to open. For checking that a nasher project's
//! JSON and the `.mod` packed from it give the same areas.

use std::time::Instant;

use anyhow::{Context, Result};
use nwn_bake::module::ModuleSource;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [a, b] = args.as_slice() else { anyhow::bail!("usage: module_compare <module A> <module B>") };
    let open = |p: &str| -> Result<ModuleSource> {
        let t = Instant::now();
        let m = ModuleSource::open(std::path::Path::new(p))?;
        eprintln!(
            "{p}: opened in {:.2} s, {} areas, {} haks, talk table {:?}",
            t.elapsed().as_secs_f64(),
            m.areas().len(),
            m.haks().len(),
            m.tlk()
        );
        Ok(m)
    };
    let (ma, mb) = (open(a)?, open(b)?);
    anyhow::ensure!(ma.haks() == mb.haks(), "hak lists differ: {:?} / {:?}", ma.haks(), mb.haks());
    anyhow::ensure!(ma.tlk() == mb.tlk(), "talk tables differ: {:?} / {:?}", ma.tlk(), mb.tlk());
    let only_b: Vec<String> = mb.areas().into_iter().filter(|r| !ma.areas().contains(r)).collect();
    let (mut same, mut same_but_order, mut differ, mut missing) = (0, 0, Vec::new(), Vec::new());
    let t = Instant::now();
    for r in ma.areas() {
        let fa = ma.area(&r).with_context(|| format!("{a}: {r}"))?;
        let Ok(fb) = mb.area(&r) else {
            missing.push(r);
            continue;
        };
        // (Field order means nothing in a GFF, the game looks fields up by label; nasher's JSON keeps them sorted, nwn_gff's
        // binary does not.)
        let mut what = Vec::new();
        let mut reordered = false;
        let mut check = |kind: &str, a: &mg_gff::Gff, b: &mg_gff::Gff| {
            if a == b {
                return;
            }
            if sorted(a) == sorted(b) {
                reordered = true;
            } else {
                what.push(format!("{kind}: {}", mg_gff::diff(&sorted(a), &sorted(b), 3).join("; ")));
            }
        };
        check("are", &fa.are, &fb.are);
        match (&fa.git, &fb.git) {
            (Some(ga), Some(gb)) => check("git", ga, gb),
            (None, None) => {}
            _ => what.push("git in one only".to_string()),
        }
        if !what.is_empty() {
            differ.push(format!("{r} ({})", what.join(" | ")));
        } else if reordered {
            same_but_order += 1;
        } else {
            same += 1;
        }
    }
    eprintln!("read and compared in {:.2} s", t.elapsed().as_secs_f64());
    println!(
        "{same} areas the same, {same_but_order} the same but for the order of fields, {} different, {} only in A, {} only in B",
        differ.len(),
        missing.len(),
        only_b.len()
    );
    for (label, list) in [("different", &differ), ("only in A", &missing), ("only in B", &only_b)] {
        if !list.is_empty() {
            println!("{label}:\n  {}", list.join("\n  "));
        }
    }
    if !differ.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

/// `g` with every struct's fields in label order.
fn sorted(g: &mg_gff::Gff) -> mg_gff::Gff {
    fn sort(s: &mut mg_gff::Struct) {
        s.fields.sort_by(|a, b| a.label.as_bytes().cmp(b.label.as_bytes()));
        for f in &mut s.fields {
            match &mut f.value {
                mg_gff::Value::Struct(c) => sort(c),
                mg_gff::Value::List(items) => items.iter_mut().for_each(sort),
                _ => {}
            }
        }
    }
    let mut g = g.clone();
    sort(&mut g.root);
    g
}
