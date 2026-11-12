# Moonglow

nwn-bake is built on the crates of the [Moonglow Toolset](https://github.com/jadzziaa/moonglow-toolset) by jadzziaa (GPL-3.0-only).
They are not on crates.io, so they come from git, pinned to one commit for all of them, in `[workspace.dependencies]` of the
top `Cargo.toml`. Every build, here and in CI, uses the same Moonglow.

**Now:** `ac17ade3292c9e9a81ac72efbd75411899cf4583`, Moonglow 4.0.0 (the commit the tag `v4.0.0` points at). Its minimum Rust is
1.98, which is nwn-bake's too (`rust-version`).

## Programs that use nwn-bake and Moonglow both

Cargo treats one git URL at two revisions as two different crates: a `GameData` from one is not the `GameData` of the other, and the
build fails. A program that uses nwn-bake and Moonglow's crates itself must pin Moonglow to the same commit as the nwn-bake it uses.
Each nwn-bake release names its Moonglow commit here and in `CHANGELOG.md`.

## Moving to another commit

1. Change the `rev` of every `mg-*` crate in the top `Cargo.toml` to the new commit (all the same). A `cargo build` then updates
   `Cargo.lock` for Moonglow alone; a plain `cargo update` would move every other dependency too.
2. Build and fix what changed in Moonglow's API. When a struct gained a field, set it to what keeps the old behaviour, and say so
   in a comment.
3. `cargo test --workspace`. If the pictures' version guard fails (`tests/pictures_version.rs`), the pictures changed: find out why
   before going on (`docs/testing.md`, "The pictures' version").
4. With a real install: the golden tests (`NWN_BAKE_GAME=... cargo test -p nwn-bake-cli --test golden`), then compare the old and
   the new build's output on real areas and characters: `area-iso` in all five views, the `area-scene` digests, characters
   (`describe`, `look`), placeable pictures and `placeables`. Anything that differs has to be understood; a change to the pictures
   raises `PICTURES_VERSION`.
5. Note the new commit here and in `CHANGELOG.md`, with what changed for nwn-bake.
