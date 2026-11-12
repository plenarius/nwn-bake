// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writes the tests' made-up game world (`tests/support/fixture.rs`: an install, two haks, a module in every form; no game content)
//! to a folder, for trying `nwn-bake` without NWN or for a release's smoke test:
//!
//! ```text
//! cargo run -p nwn-bake --example fixture -- <dir>
//! nwn-bake area-iso <dir>/fx.mod fx_main fx_mark --out <dir>/out --view all,top --root <dir>/game --user <dir>/user
//! ```

#[path = "../tests/support/fixture.rs"]
mod fixture;

fn main() {
    let Some(dir) = std::env::args_os().nth(1) else {
        eprintln!("usage: fixture <dir> (a new or empty folder)");
        std::process::exit(1);
    };
    // (The fixture empties its folder first: never one with something in it.)
    if std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some()) {
        eprintln!("{}: not empty", std::path::Path::new(&dir).display());
        std::process::exit(1);
    }
    let f = fixture::write(std::path::Path::new(&dir));
    println!(
        "nwn-bake area-iso {} {} --out {} --view all,top --root {} --user {}",
        f.module_mod().display(),
        fixture::AREAS.join(" "),
        f.base.join("out").display(),
        f.root.display(),
        f.user.display()
    );
}
