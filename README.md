# nwn-bake

Pictures and plain data from **Neverwinter Nights: Enhanced Edition**, for websites and tools:

- **Area pictures**: an area drawn from its own files (`.are`, `.git`, the tileset and its tile models) from the four corners at 30
  degrees and straight down, with a documented manifest that maps game coordinates to pixels. A whole module renders in one batch,
  and a cache keeps it current: after a module or hak change, only the areas that changed are drawn again.
- **Characters as parts**: what a character is made of (base model, head, armour, weapons, cloak, wings, with their PLT colours),
  and each part's meshes as a small file, so a web viewer can put the character together, recolour it and swap parts.
- **Areas in 3D**: an area as one glTF binary (`.glb`) for a web page, each model stored once, the tiles instanced, the doors and
  placeables as named nodes, and the textures as WebP files shared by every area.
- **Pictures** of creatures, items and placeables, and a placeable's textures.
- **Scenes**: several placeables, models, creatures and items in one picture at a free camera (perspective or orthographic),
  with lines, axis markers and points projected to pixels: two models side by side, a chess board, a lineup.
- **Flattened models**: a model at rest as triangles grouped by texture, with its walkmesh footprint; textures as PNG.

It is a command, `nwn-bake` (`nwn-bake-cli/`), and the Rust library it is built on (`nwn-bake/`), for builders and admins of
NWN:EE modules and for anyone writing a website or tool that shows them.

## You need

- **The game:** a legally obtained installation of Neverwinter Nights: Enhanced Edition, and your module and haks. nwn-bake contains
  and ships **none** of the game's content: no models, textures, areas or other game files are in this repository or its releases,
  and its tests run on a small made-up game world they write themselves. You can try it on that world without the game (below).
- **A GPU, only for characters:** areas are drawn on the CPU and need no GPU. Characters and the pictures of creatures, items and
  placeables use wgpu, for which a software adapter (Mesa lavapipe) is enough.
- **To build it yourself:** Rust 1.98 or newer, a C and C++ compiler (libwebp is C, glslang is C++) and git. A first release build
  takes about 10 minutes on 4 cores.

nwn-bake is not affiliated with or endorsed by Beamdog or Wizards of the Coast. Neverwinter Nights is a trademark of its owners.

## Install

Download a binary for Linux (x86-64, glibc 2.36 or newer: Debian 12, Ubuntu 22.10 and later), Windows (x86-64) or macOS (one
universal binary for Apple silicon and Intel) from the [releases page](https://github.com/plenarius/nwn-bake/releases), and check
it against `SHA256SUMS`.

The binaries are not signed yet:

- **macOS** stops a downloaded binary the first time. Clear the quarantine flag: `xattr -d com.apple.quarantine ./nwn-bake`.
- **Windows** SmartScreen may warn about an unknown publisher: "More info", then "Run anyway".

Or build it (Rust 1.98 or newer, a C and C++ compiler, git; about 10 minutes the first time):

```sh
cargo build --release -p nwn-bake-cli      # target/release/nwn-bake
scripts/build-debian.sh                    # a Linux binary for Debian 12 and newer, built in Docker
```

## Try it without the game

The source has a small made-up game world, the tests' fixture: a tileset, a module of six areas, a hak and a talk table. From a
checkout of this repository, write it to a folder, then draw two of its areas (the first command prints one that draws all six;
use `target/release/nwn-bake` if you built nwn-bake yourself):

```sh
cargo run --release -p nwn-bake --example fixture -- /tmp/nwn-bake-try
nwn-bake area-iso /tmp/nwn-bake-try/fx.mod fx_main fx_mark --view all,top --out /tmp/nwn-bake-try/out \
  --root /tmp/nwn-bake-try/game --user /tmp/nwn-bake-try/user
```

`/tmp/nwn-bake-try/out` then has five WebP pictures and a manifest per area (`fx_main.json`, `fx_mark.json`).

## Finding the game

Every command needs the game:

- `--root <install>`: the NWN:EE install, the folder with `data/` and `lang/`. Without it, the install is found as the game finds
  it (`NWN_ROOT`, then the usual places).
- `--user <folder>`, else `NWN_BAKE_USER`, else `NWN_HOME`: the user folder (`hak/`, `tlk/`, `override/`), the same rule for
  every command. For areas it is optional (without one, the game's default user folder).
- Haks: areas use their module's hak list, in its order (the first listed wins, as in the game), each found in the user folder's
  `hak/`, then the install's `data/hk/`. `--haks a,b,c` gives another list. The character commands use every `.hak` in `hak/`, by
  name (none when there is no `hak/`), or a module's list with `--module <module>`.
- `--tlk <name or path>` (or `NWN_BAKE_TLK`): the custom talk table, for names: a file in the user folder's `tlk/` or `hak/`
  (`.tlk` may be left out) or a path; not finding it is an error. By default the module's (none when it is not there), or the
  one `.tlk` in the user folder's `tlk/` or `hak/`.
- The install's language: English when it has `lang/en`, else the language it has (a French or German install).
- `nwn-bake --help` lists the commands, `nwn-bake <command> --help` their options. Exit codes: 0 done, 1 an error, 2 misuse
  (found before the game is opened), 3 a batch in which an area failed. `NWN_BAKE_SOFTWARE=1` uses Mesa lavapipe even when there
  is a GPU.

A **module** can be given as a `.mod`, a folder of binary files (`.are`, `.git`, `module.ifo`, ...), a folder of `nwn_gff` JSON
files, a [nasher](https://github.com/squattingmonk/nasher) project, or one area file (`x.are` or `x.are.json`, with its `.git`
beside it and the nearest `module.ifo` above it for the haks): the cheap way to draw one area of a large project.

## Area pictures: `area-iso`

```text
nwn-bake area-iso <module> <area>... --out <dir> [options]
nwn-bake area-iso <module> --all --out <dir> [--jobs N] [--cache <dir>] [options]
```

For each area, one picture per view and a manifest, `<area>.json` (`docs/area-iso.md`): the area's size and kind, the inputs'
digests and the cache keys, the options, and for each view its size and `matrix`, which maps a game point (x, y, z in metres) to a
pixel. With `--all`, every area of the module, into `webp/<view>/<area>.webp` and `json/<area>.json`, plus `manifest.jsonl` (one
line per area and view) and `index.json`.

| Option | Default | |
|---|---|---|
| `--view` (or `--views`) `sw\|se\|ne\|nw\|top\|all`, or a list (`sw,top`) | `sw` | From a corner, 30 degrees up, or straight down (north up). `all` is the four corners |
| `--scale <pixels a metre>` | 8 | |
| `--supersample 1..4` | 2 | Supersampling |
| `--format webp\|png` | `webp` | WebP: lossy, quality 85, alpha kept |
| `--background transparent\|#rrggbb` | `transparent` | |
| `--light day\|area` | `day` | Plain daylight, or the area's own sun colours |
| `--shadows on\|off` | `on` | |
| `--roof auto\|keep\|<metres>` | `auto` | Interiors and underground areas are cut open over their floor; `keep` never cuts |
| `--exposure auto\|off` | `auto` | Dark areas lifted |
| `--objects on\|off` | `on` | Placeables and doors |
| `--depth` | | Also a 16-bit height picture per view |
| `--layout flat\|tree` | `tree` with `--all` | Files side by side, or in folders by kind and view |
| `--name '{area}.{view}.{ext}'` | | File names, for `flat` |
| `--jobs N` | half the cores (`NWN_BAKE_JOBS`) | Threads |
| `--cache <dir>` | | Keep every key's pictures, so a change undone costs nothing |
| `--force` | | Draw even what the keys say is there |
| `--strict` | | A hak of the module's list that is not found fails the area |
| `--target <name>` | the target nasher packs by default | A nasher project's areas from this target: for areas the packed module leaves out |
| `--look <look.json>` | | Calibration constants (`nwn_bake::iso::Look`); written into the manifest and the keys |

An area whose pictures are already in the output folder (or the cache) for its keys is not drawn again. The quick key needs no
scene; when it changed (a hak was rebuilt), the area's scene is built (about half a second) and its exact key, which covers every
file the area reads, decides. Progress goes to stderr, one JSON line per area to stdout at the end. The exit code is 3 when an area
failed, and 2 on misuse (an unknown option, a value that does not parse).

```sh
nwn-bake area-iso mymodule.mod village --view all,top --out pictures/
nwn-bake area-iso ~/projects/mymodule --all --out pictures --cache pictures-cache --jobs 4
```

`--look` tries other values for the constants that set the look without a build. The file lists only the constants it changes
(the fields of `nwn_bake::iso::Look`); the rest keep their defaults:

```sh
echo '{"elevation_deg": 35.0, "exposure_target": 170.0}' > look.json
nwn-bake area-iso mymodule.mod village --look look.json --out try/
```

## Areas in 3D: `area-3d`

```text
nwn-bake area-3d <module> <area>... --out <dir> [options]
nwn-bake area-3d <module> --all --out <dir> [--jobs N] [--cache <dir>] [options]
nwn-bake area-3d <area.are | area.are.json | area.git | area.git.json> --out <dir> [options]
nwn-bake area-3d <module> [<template>] --draft <draft.json | -> --out <dir> [options]
```

For each area, `<area>.glb` (glTF 2.0 with `EXT_mesh_gpu_instancing` and `EXT_texture_webp`, Y up) and a manifest, `<area>.json`
(`docs/area-3d.md`): the area, the counts, the bounds, the textures it uses and every door of the `.git` with its position, bearing
and the direction its street side faces. The textures go into one folder for every area (`--tex-dir`, default `<out>/tex`), named
by their content, so areas share them and a browser can cache them for good. It is built from the same scene as the area pictures.
Beside them, `<area>.floor`: the walkable floor of the tiles' walkmeshes every 0.625 m, with its height and where the `.git`'s
placeables block it, for checks such as "is this spot on the floor" without the game.

| Option | Default | |
|---|---|---|
| `--tex-dir <dir>` | `<out>/tex` | Where the textures go |
| `--tex-uri <prefix>` | `--tex-dir` relative to `--out` | What the `.glb` puts before a texture's file name (a relative path or a URL) |
| `--max-tex <pixels>` | 512 | The first mip level no wider or higher than this |
| `--quality <0..100>` | 70 | WebP quality |
| `--objects on\|off` | `on` | Placeables and doors |
| `--draft <file \| ->` | | Bake a layout that is not a file yet, a tile painter's draft, on the area named as its template (`docs/area-3d.md`, "Drafts") |
| `--jobs N`, `--cache <dir>`, `--force`, `--strict`, `--target <name>` | | As for `area-iso` |

An area whose `.glb`, floor file, manifest and textures are already there for its key is not baked again. The exit code is 3 when an area failed.

## An area's scene: `area-scene`

```text
nwn-bake area-scene <module> <area> [--haks a,b,c] [--objects on|off] [--target <name>] [--root <game>] [--user <folder>]
nwn-bake area-scene <area.are | area.are.json | area.git | area.git.json> [options]
```

Builds the scene an area's pictures are drawn from (the tile models' triangles, water, placeables and doors, and the floor probe)
and says what is in it: counts, timings, the floor's statistics, where the roof would be cut, and the scene's digest (the same for
the same inputs). A summary on stderr, one JSON line on stdout. Useful to see why an area is drawn as it is, and to check that a
change leaves the scene alone.

## Several objects in one picture: `scene`

```text
nwn-bake scene <spec.json|-> <out.png> [--items <dir>] [--strict-items] [--module <module>]
```

Placeables (by `placeables.2da` row), models, creatures (`.utc`, `.bic`, nwn_gff's `.utc.json`) and items, each with a position,
facing, scale and texture replacements, at one camera: from a position, or round a target by yaw, pitch and distance, perspective
or orthographic, framed to what is drawn when left out. Lines, axis markers, the light, the sun's shadows and the background
(transparent or a colour) are in the spec too. A creature wears its blueprint's items, looked up as for `thumb`. Prints the
camera, each object's box and what it left out, and the pixel positions of the spec's `points`, as JSON. See `docs/scene.md`.

## A baker that stays: `serve`

```text
nwn-bake serve [--root <install>] [--user <folder>] [--tlk <name>] [--module <module>]
```

One JSON request a line on stdin, one answer a line on stdout: characters as parts, pictures of creatures, items and placeables, a
placeable's textures, scenes (`scene`), and areas (`area_iso`, `area_iso_key`). The game is opened once. See `docs/serve.md`.

## Other commands

| Command | |
|---|---|
| `creature <file.bic\|.utc> <out-dir> [--anim name] [--t seconds] [--items dir]` | A character as one scene (`scene.json`, `tex/*.png`) |
| `describe <file.bic\|.utc>` | A character as a list of parts (JSON) |
| `look <file.bic\|.utc> <parts-dir>` | `describe`, and writes the parts, textures and palettes the folder does not have |
| `part <spec.json> <out>` | One part's meshes, as `look` writes them (the spec: below) |
| `thumb <file.utc> <out.png> [size [yaw]] [--items dir]` | A picture of a creature |
| `item-thumb <file.uti> <out.png> [width [height [yaw]]]` | A picture of an item |
| `placeable-thumb <row> <out.png> [width [height [yaw]]] [--tex old=new ...]` | A picture of a `placeables.2da` row |
| `placeables <out.json>` | Every `placeables.2da` row with a model: origin, size and bounds, its walkmesh's footprint, particles, light, textures |
| `textures <row>` | The textures a placeable's model draws with |
| `kind <texture> <0\|1>`, `plt <texture> <out.png>`, `plain <texture> <out.png>`, `palettes <out-dir>` | Textures and PLT palettes as PNG |
| `resources <out.json>` | The names of the icons (TGA, PLT and DDS named `i...`) and weapon part models the game has, for a list of item appearances |

A creature blueprint (`.utc`) names the items it wears by resref; a character (`.bic`) carries them. `creature`, `describe`,
`look` and `thumb` look such an item up in the folder `--items` names first (`<resref>.uti` or nwn_gff's `<resref>.uti.json`,
subfolders too: a module's own items, as a nasher project keeps them), then in the user folder's override and haks, then in
the game. An item found nowhere is left off the creature, and a line on stderr names it and its slot
(`warning: equipped item not found, drawn without it: nw_ashsw001 (lefthand)`); `--strict-items` makes that an error. These
commands, and `item-thumb`, read nwn_gff's JSON (`.utc.json`, `.uti.json`) as well as binary files.

`part`'s spec is a pose (`anim`, `t`) and a `base` and a `part` as `describe` and `look` print them (`nwn_bake::look::PartSpec`:
`model`, `attach`, `scale`, `textures`, `env_map`, `animated`, `colored`; their `id` and `colors` are ignored). With `part` null,
the meshes are the base's own:

```json
{"anim": "pause1", "t": 0,
 "base": {"model": "pmh0", "attach": null, "scale": 1, "textures": {}, "env_map": null, "animated": false, "colored": true},
 "part": {"model": "pmh0_head001", "attach": "head_g", "scale": 1, "textures": {}, "env_map": null, "animated": false, "colored": true}}
```

## Documents

- `docs/area-iso.md`: the manifest (format `nwn-bake/area-iso`, version 1), the batch layout, the keys and the cache. JSON Schemas:
  `docs/area-iso.schema.json`, `docs/area-iso-line.schema.json`.
- `docs/area-3d.md`: areas as glTF (format `nwn-bake/area-3d`, version 2): the `.glb`'s nodes and axes, the textures, the floor file,
  the manifest and the keys. JSON Schema: `docs/area-3d.schema.json`.
- `docs/scene.md`: a scene's spec and answer (`scene`, and `serve`'s `scene` request). JSON Schema: `docs/scene.schema.json`.
- `docs/serve.md`: the `serve` protocol.
- `docs/testing.md`: the tests, the fixture, the pictures' version and the golden tests on a real install.
- `CHANGELOG.md`, `MOONGLOW.md`.

## Development

`cargo test --workspace` needs no game: it writes a small game world of its own. With an install,
`NWN_BAKE_GAME=/path/to/nwn cargo test -p nwn-bake-cli --test golden` also draws a few areas of the game's own campaigns and
compares them with stored fingerprints. See `docs/testing.md`.

`scripts/check.sh` runs every check CI runs (formatting, clippy, rustdoc, cargo-deny, cargo-about, spelling, writing, Markdown,
links, REUSE and the secret scans). `STYLE.md` is the code and documentation standard.

## Credits

nwn-bake is built on the [Moonglow Toolset](https://github.com/jadzziaa/moonglow-toolset) by **jadzziaa** (GPL-3.0), a module
toolset for NWN:EE written in Rust, which reads every NWN format nwn-bake uses. nwn-bake uses its crates `mg-core`, `mg-resman`,
`mg-rules` (resources and the game's data), `mg-gff`, `mg-erf`, `mg-tlk`, `mg-set`, `mg-module` (file formats and modules),
`mg-mdl`, `mg-image` (models and textures), and with the `creature` feature `mg-preview` and `mg-render` (characters). Thank you.

`MOONGLOW.md` says which Moonglow revision is used and how to move to another.

The binaries also include C and C++ libraries, built from the source their crates bundle: libwebp (BSD-3-Clause; Google's
patent grant is in the source archive), glslang through Moonglow's shader compiler (BSD-3-Clause, BSD-2-Clause, MIT, Apache-2.0
and NVIDIA's licence) and zstd (BSD-3-Clause). Each release's `THIRD-PARTY-LICENSES.html` has their licence texts and those of
every Rust crate in the binary.

## Licence

Copyright (C) 2026 plenarius

This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as
published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License along with this program. If not, see
<https://www.gnu.org/licenses/>.

- nwn-bake's own source code is licensed under the **GNU General Public License, version 3 or (at your option) any later
  version** (`GPL-3.0-or-later`). The full text is in `LICENSE`.
- Moonglow is licensed under **GPL-3.0 only**. The `nwn-bake` binaries include Moonglow, so the **released binaries are under
  GPL-3.0** (version 3 only).
- Each release has the complete source of its binaries, every dependency included (`nwn-bake-<version>-source.tar.gz`), and the
  licences of the third-party crates (`THIRD-PARTY-LICENSES.html`).
