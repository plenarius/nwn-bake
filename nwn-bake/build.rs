// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! The commit nwn-bake is built from (`NWN_BAKE_COMMIT`, for the area pictures' manifests), when it is built from a git checkout:
//! always its first 7 characters, whether a release build sets the whole hash or git gives the short one.

use std::process::Command;

/// The commit's length in the manifests.
const COMMIT_LEN: usize = 7;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=NWN_BAKE_COMMIT");
    if let Ok(given) = std::env::var("NWN_BAKE_COMMIT") {
        // (This setting replaces the one in the environment for the compiler.)
        println!("cargo:rustc-env=NWN_BAKE_COMMIT={}", given.trim().chars().take(COMMIT_LEN).collect::<String>());
        return;
    }
    if let (Some(dir), Some(common)) =
        (git(&["rev-parse", "--absolute-git-dir"]), git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]))
    {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={common}/refs/heads");
        println!("cargo:rerun-if-changed={common}/packed-refs");
    }
    if let Some(commit) = git(&["rev-parse", &format!("--short={COMMIT_LEN}"), "HEAD"]) {
        println!("cargo:rustc-env=NWN_BAKE_COMMIT={}", commit.chars().take(COMMIT_LEN).collect::<String>());
    }
}
