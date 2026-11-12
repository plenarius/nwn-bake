# Testing nwn-bake

`cargo test --workspace` needs no Neverwinter Nights install. Every test that reads game data reads a made-up game world the tests
generate themselves. The golden tests on a real install run only when one is named.

nwn-bake is built optimised (`opt-level = 3`) in debug and test builds too, set in the workspace's `Cargo.toml` (the repository's
top): the renderer fills millions of pixels a picture and takes minutes unoptimised.

## The fixture

`tests/support/fixture.rs` writes a small game world to a temporary folder (under Cargo's `CARGO_TARGET_TMPDIR`). Everything is
generated from code with Moonglow's writers (ERF, GFF, talk table), so it holds no file of the game and can be published with the
source:

- an "install" holding only an empty talk table, so the game opens without NWN;
- `fx_tiles.hak`:
  - the tileset `fx01`, which counts four terrains and has three (an absent `[TERRAIN2]`, as in the game's mst03), one of them
    `Chasm`;
  - six tile models as ASCII `.mdl`, with ASCII walkmeshes: grass with a rock and a pond, black void, plain grass, a rock top at
    6 m, a room with a ceiling, and a pit (grass around a hole whose floor, 3 m down, is black and does not walk). Two tiles use
    the pit: one with `Chasm` corners (a chasm) and one with `Grass` corners (a black floor that is no chasm);
  - checkerboard TGA textures, a black void texture, a pit texture (black but for a grey border, as the underdark's pit textures
    are) and a water face;
  - a material (`fx_rock.mtr`) that names another texture;
  - a placeable, a marker placeable and a door, with the 2DAs the scene reads;
- `fx_over.hak`: a texture also in `fx_tiles.hak` (the marker, red here and blue there). The module lists `fx_over` first, so the
  first hak listed must win;
- a module with six areas, laid out as a `.mod`, a folder of binary files, a folder of nwn_gff JSON files, and a nasher project:
  - `fx_main`: tiles turned every way and a step up, a box and a door;
  - `fx_mark`: a red marker over plain grass;
  - `fx_flat`: rooms whose ceilings the roof cut takes away;
  - `fx_cave`: a cave dug out of rock;
  - `fx_step`: ground two steps up over black void, which is no cave;
  - `fx_chasm`: plain grass, the chasm, and the pit without chasm terrain, west to east.

The file's header has the details (each tile, each area, what it is for). `cargo run -p nwn-bake --example fixture -- <dir>` writes the
fixture to a folder and prints an `area-iso` command for it, to try the renderer without the game or to smoke-test a release.

`cargo run --release -p nwn-bake --example module_compare -- <module A> <module B>` checks that every area of one module reads the
same from another (its `.are` and `.git`, field by field): for a nasher project's JSON against the `.mod` packed from it.

## The tests

| File | Tests | What they check |
|---|---|---|
| `nwn-bake/src/**` (unit) | 48 (65 with the `creature` feature, as in `--workspace`) | A scene's spec checked before the game is read, its defaults, its camera (framed to the scene from the south at any size, orthographic at one scale near and far, straight down with north up, a position and a target), a facing as the game's, colours, axis markers, the light and a transparent picture's matte; The tileset's absent sections; a draft made into an area's files (its template's own layout byte for byte, grown west and south, objects dropped or outside but never a kept tag, `keep_tags` by tag and prefix, a bare area, drafts refused); a user folder without `hak/` has no haks; a poisoned lock still used, a panic turned into an error with its message; a picture too large refused before anything is allocated; a `.glb` written and read back (welding, normals, instances, node extras, texture URIs, 32-bit indices, a broken file refused); the floor probe and roof-cut rule on hand-made height layouts; the renderer on hand-made scenes (transform in every view, top view, roof cut, water, black void, background, manifest, `Look` files, chasm floors and their distance field); the shadow map's size |
| `nwn-bake/tests/game_texture.rs` | 3 | Opening the game: hak order, a missing hak, MTR resolution |
| `nwn-bake/tests/module.rs` | 5 | `ModuleSource` in every form, a nasher project by target, a plain ERF of areas without haks, a folder of JSON files with or without `nasher.cfg`, JSON against binary |
| `nwn-bake/tests/area_scene.rs` | 4 | A scene on a small made-up tileset; an `.are` past 32 tiles a side, or whose `Tile_List` does not match its size, fails |
| `nwn-bake/tests/fixture_scene.rs` | 7 | On the fixture: the tileset with its absent terrain; every module form gives the same areas, scene digests and `.are`/`.git` digests; tiles turned and lifted, water apart, the material, the box and door; the first hak listed wins; the floor probe and roof cut on each layout; the same scene twice, and a different one when the `.git`, the `.are`, a texel or a vertex changes; the chasm rule: the pit's floor is a chasm (`chasm: true`) on the tile with chasm terrain only, and not where its walkmesh walks |
| `nwn-bake/tests/fixture_iso.rs` | 7 | On the fixture: the marker drawn where `matrix` puts it in all five views (default options, and another scale and quality), within half a pixel; what the pictures show (grass, the box, the pond's water, the rock through its material); black void left out but the size and transform kept; the chasm's floor drawn dark blue-grey, darker in its middle, while the same black floor without chasm terrain stays black and the rest of the picture is unchanged, and the `Look`'s chasm colours and seam used; the roof cut opens the rooms, `keep` keeps the ceilings; the same pixels twice; every manifest written (three sets of options, six areas) validates against `docs/area-iso.schema.json`, and broken manifests do not |
| `nwn-bake/tests/fixture_batch.rs` | 10 | Batches and the cache on the fixture (`nwn_bake::iso::batch`): an area whose work panics fails and the others are drawn, and a report that panics loses nothing, on one thread and three; every area drawn once into `webp/<view>/<area>.webp` and `json/<area>.json`, with `manifest.jsonl` (each line valid against `docs/area-iso-line.schema.json`, its matrix and path the manifest's) and `index.json`; a second run draws nothing and leaves every file as it was, bytes and times, with or without the cache; a changed `.git` draws that area only, and moved back its pictures come from the cache; a hak entry no area reads changes every quick key and no exact key, draws nothing and writes no picture; a changed tileset texture draws the four areas that read it and not the two that do not; a new output folder fills from the cache; one thread and five give the same files byte for byte, and `--force` the same again; the keys are the same in another folder and change with an option or the area, and count the renderer by its name and pictures' version only; WebP pictures (RIFF/WEBP, alpha, the marker where the matrix puts it within a pixel); depth pictures (16-bit, 0 where nothing is drawn, the grass at 0 m and the marker 1 m up within a centimetre); `serve`'s `area_iso` and `area_iso_key` (drawn once, then current; the keys by module and by the area's files agree; another option, another key); a nasher project read lazily gives the same areas and files as the `.mod` |
| `nwn-bake/tests/scene.rs` | 4 | A scene's objects on the fixture and the creature kit, on the CPU (`nwn_bake::scene::parts`): a placeable row, a model with a texture replaced, a creature in its animation wearing an item from the folder (its body's texture replaced, not the item's) with the one found nowhere named with its slot, and an item; a strict lookup names the object and the item; a model, file or row that is not there names the object; `docs/scene.schema.json` and the code take and refuse the same specs |
| `nwn-bake/tests/pictures_version.rs` | 1 | The pictures' version guard (below): the fixture's pictures and manifests, default options, all areas and views, PNG and WebP, have the digests recorded in `tests/pictures.json` for `PICTURES_VERSION` |
| `nwn-bake/tests/fixture_area3d.rs` | 7 | `area-3d` on the fixture (`docs/area-3d.md`, "Tests"): the placements give the scene's triangles bit for bit and the `.glb` drawn from its instances and nodes is the scene; tiles as instances, each mesh stored once; a tile turned any way lands where the scene puts it; the floor file is the walkmesh where the game walks; the door and the box as named nodes and the door list; textures named by content and shared between areas, the same bytes twice and from every module form; the batch's `current`, `cached` and `baked`, a missing texture, another option's key, `--force`; an area whose work panics fails and the others are baked |
| `nwn-bake/tests/glb_version.rs` | 1 | The `.glb`s' version guard (below): the fixture's `.glb`s, manifests and textures have the digests recorded in `tests/glbs.json` for `GLB_VERSION` |
| `nwn-bake-cli/src/**` (unit) | 4 | The command line is well formed (clap's own check); misuse is found before the game is opened; an area file alone names its area; `area-3d`'s texture folder is named relative to the output folder |
| `nwn-bake-cli/tests/area_iso.rs` | 9 | The `nwn-bake` binary end to end: `area-iso` writes PNGs (`--format png`) of all five views and manifests that validate, whose `matrix` puts the marker where the PNG shows it; every module form gives the same PNGs and manifests; the options go into the pictures and the manifest; a missing hak is a note, and `--strict` fails the area with exit code 3; an area that is not there fails and the rest are drawn; wrong options are misuse (exit code 2) and stop it; `--all` writes WebP in folders with `manifest.jsonl`, and a second run draws nothing and leaves the files' times alone; `serve` answers `area_iso` and `area_iso_key`, and an error for an area that is not there; `area-scene` gives the same digest for every form; a batch with stdout and stderr closed ends, exit code 0 |
| `nwn-bake-cli/tests/area_3d.rs` | 3 | The `nwn-bake area-3d` command end to end: real WebP, a second run all `current`, an area file on its own with the textures elsewhere, the exit codes; a batch with stdout and stderr closed ends, exit code 0; `--draft`: an area's own layout gives its key, `.glb` and floor file, grown a tile west (from stdin, under its own name) the door moves and a second run is `current`, a draft without a template, a draft that does not fit (exit 1) and two templates or `--all` (exit 2) |
| `nwn-bake-cli/tests/cli.rs` | 7 | The command line: `--help`, `--version`, a bare run and an unknown command (exit code 2, before the game is opened); a scene's spec, from a file or stdin, checked before the game is opened (exit code 1); the user folder from `NWN_BAKE_USER` or `NWN_HOME`, and an error naming the variable when it is not there; `serve` without a user folder answers area requests, and an unknown field or request, a bad scene or one without `out` is an error; a wrong `--tlk` and a file that is not a talk table are errors, a name without `.tlk` is found and recorded; a wrong `--root` is "not an NWN:EE install", and an install with only `lang/fr` opens; a file that cannot be read is named |
| `nwn-bake-cli/tests/golden.rs` | 5 | Only with `NWN_BAKE_GAME` (below): four areas, and the creatures |
| `nwn-bake-cli/tests/golden_area3d.rs` | 2 | Only with `NWN_BAKE_GAME`: `area-3d` of `q1rural`, its placements against the scene, its counts against `tests/golden/area3d-q1rural.json`, every door of its `.git` a node; `q1rural` as a draft of its own layout bakes to the same `.glb` and floor file, and grown a tile west and south moves every door 10 m |

The schema, `docs/area-iso.schema.json` (JSON Schema 2020-12), is the manifest v1 of `docs/area-iso.md` in machine form. It allows
fields it does not list, as the format does.

## The pictures' version

The cache keys of the area pictures hold a hand-raised number, `nwn_bake::iso::PICTURES_VERSION` (`nwn-bake/src/iso/output.rs`),
instead of the build's commit, so a new build does not make every area stale (`docs/area-iso.md`, "Keys"). **Raise it by one in any
change that alters a picture or a manifest**: the look, the scene, the camera, the encoders or their settings, a manifest field.
Every area is then drawn again on the next run (for a module of 1,300 areas, about an hour and a half on eight cores). Leave it
alone for changes that draw the same pictures.

`tests/pictures_version.rs` guards the rule. It draws the fixture's six areas in all five views with the default options, once as
PNG and once as WebP, and compares a SHA-256 of each picture file and of each manifest with `tests/pictures.json`, which records
them with the pictures' version they belong to (72 digests). The manifests are compared without `renderer.version`,
`renderer.commit` and `quick_key`, which do not describe the pictures. The test:

- passes when the digests are the recorded ones and the version is the recorded one;
- fails, listing what changed, when the digests differ and the version is still the recorded one: raise `PICTURES_VERSION`, then
  write the digests again;
- fails when the version was raised but the digests were not written again.

Write them again (with the `webp` feature, so both formats are recorded) and commit `tests/pictures.json` with the change:

```text
NWN_BAKE_PICTURES=write cargo test -p nwn-bake --features webp --test pictures_version
```

Writing refuses when the digests changed while the version is still the recorded one, so it cannot stand in for the raise. Only
when the change is to the fixture itself (which no real area's pictures depend on) write them with `NWN_BAKE_PICTURES=write-same`
instead, and say so in the PR.

The digests are of the files' bytes. They are the same on every machine of one platform: the fixture is generated by code, the
drawing is deterministic (each view on one thread, in the scene's order, joined in a fixed order) and libwebp encodes on one
thread. They were recorded on x86-64 Linux and match on GitHub's `ubuntu-latest` runners, where CI (`ci.yml`) runs them.
On another platform (`platform` in the file) float results may differ in the last bit, so there the test says it is skipped and
passes; the golden tests below compare pictures with a tolerance instead. Without the `webp` feature (`cargo test -p nwn-bake`)
only the PNG digests are checked; `cargo test --workspace` turns the feature on through the CLI.

## The `.glb`s' version

area-3d's keys hold `nwn_bake::area3d::GLB_VERSION` (`src/area3d.rs`, `renderer.glb` in the manifest) in place of the crate's
version and the build's commit, as the pictures' keys hold `PICTURES_VERSION` (`docs/area-3d.md`, "Keys and the cache"). **Raise
it by one in any change that alters a `.glb`, a texture file, a floor file or a manifest for the same inputs.** Every area is then baked again
on the next run.

`tests/glb_version.rs` guards it as `tests/pictures_version.rs` guards the pictures: it bakes the fixture's six areas with the
default options and compares a SHA-256 of each `.glb` (without `asset.generator`, which names the crate's version), each manifest
(without `renderer.version` and `renderer.commit`), each floor file and each area's texture files with `tests/glbs.json` (24
digests). The
textures go through a stand-in encoder, so the test needs no WebP; a change to the WebP settings must raise the version all the
same. It passes, fails and is written again the same way, with `NWN_BAKE_GLBS` in place of `NWN_BAKE_PICTURES`:

```text
NWN_BAKE_GLBS=write cargo test -p nwn-bake --test glb_version
```

## Golden tests on a real install

```text
NWN_BAKE_GAME=/path/to/nwn cargo test -p nwn-bake-cli --test golden
```

`NWN_BAKE_GAME` is the install (the folder with `data/` and `lang/`). `NWN_BAKE_HOME` may name a user folder; by default an empty
one is used, so nothing in an `override/` changes the result. Without `NWN_BAKE_GAME` each golden test prints that it is skipped
and passes.

They use the base game's own campaign modules, so they need no hak:

| Area | Module | Why |
|---|---|---|
| `m0q1c` | `Prelude.nwm` | A small interior, mostly black void, cut 3.5 m over its floor |
| `map_m1q4e` | `Chapter1.nwm` | An interior that is a cave dug out of rock, cut 2 m over its highest floor, with water |
| `q1rural` | `XP1-Chapter 1.nwm` | A large surface area (16 by 16 tiles) with hills, water, 208 placeables and 12 doors |
| `koboldcaves` | `XP1-Chapter 1.nwm` | Caves (16 by 16 tiles) with water and 397 placeables and 31 doors |

Each area is drawn in all five views with the default options. What is stored in `nwn-bake-cli/tests/golden/area-<area>.json` is
not a picture but a fingerprint:

- for each view, the width, the height and a 32 by 18 grid of the mean colour (RGBA) of each cell, compared with a tolerance:
  - sizes within a pixel;
  - each cell's channels within 8;
  - all of them within 0.5 on average;
- the scene's counts, its kind and the roof cut (to the centimetre), compared exactly.

Floats may differ a little between platforms and compilers, which the tolerance allows for; a change of look, of the roof cut or of
what is drawn does not pass.

`golden/creatures.json` holds five creature blueprints described part by part (`nwn_bake::look::describe`: models, attachments,
textures, colours, compared exactly; on the CPU, no GPU needed). Four are made of parts (`nw_halffnd001`, `nw_tiefling02`,
`x2_mephdrow006`, `x3_dwarfmount001`, mounted) and one is a single model (`nw_drgred001`).

The fingerprints were recorded on x86-64 Linux from the base game at data build r1284. **When a change to the pictures is intended**, write
them again and commit them with the change, saying in the PR why they changed (and raise the pictures' version, above):

```text
NWN_BAKE_GAME=/path/to/nwn NWN_BAKE_GOLDEN=write cargo test -p nwn-bake-cli --test golden
```

Then run the tests again without `NWN_BAKE_GOLDEN` to see them pass.

## What is not covered without the game

- Composed creatures, items and placeable pictures (`creature`, `thumb`, `item-thumb`, `placeable-thumb`, `scene`, `look`, `part`
  and the character and scene requests of `serve`). They need wgpu and real models, animations and palettes. The golden tests
  only cover what `describe` says of a creature, not the pictures; `tests/scene.rs` checks what a scene's objects are made of, not
  the picture.
- `nwn_bake::emitters` (particles of weapons and placeables), `placeables_index`, `model_textures`, the PLT palettes
  (`plt`, `palettes`).
- Binary `.mdl` models and DDS textures: the fixture's models are ASCII and its textures TGA. Moonglow reads both, and the golden
  tests read them from the game.
- Tile animations, lights and day and night meshes, which the scene leaves out anyway.
- A whole module's batch and its timings, and the keys on real haks: measured by hand on a large module with its haks.
- Whether a picture looks right: the fixture's made-up world has no look to judge, and the golden fingerprints (below) only see
  that the look changed. A pull request that changes the look (`nwn_bake::iso::Look`, the light, the camera, what is drawn) shows
  the same few areas before and after, side by side, from the four corners and from the top, drawn from a game install; the
  reviewers judge them there.
