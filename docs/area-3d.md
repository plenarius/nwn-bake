# Areas in 3D: `nwn-bake area-3d` (`nwn-bake/area-3d`, version 2)

One glTF binary per area for a web page (three.js's `GLTFLoader`, or any glTF 2.0 viewer with the two extensions below), with each
model stored once, the doors and placeables as named nodes, and the textures as WebP files shared by every area; beside it, the area's
walkable floor. Built from the same scene as the area pictures (`nwn_bake::area::scene`), so the tiles are where the pictures show them.
The code is `nwn_bake::area3d` (the writer), `nwn_bake::area::scene::placements` (the scene as placed models) and
`nwn_bake::area3d::batch` (many areas, the cache).

```text
nwn-bake area-3d <module> <area>... --out <dir>        (or --all for every area of the module)
nwn-bake area-3d <area.are | area.are.json | area.git | area.git.json> --out <dir>
nwn-bake area-3d <module> [<template>] --draft <draft.json | -> --out <dir>     (or a lone template area file; see Drafts)
    [--tex-dir <dir>] [--tex-uri <prefix>] [--max-tex 512] [--quality 70] [--objects on|off]
    [--jobs N] [--cache <dir>] [--force] [--root <game>] [--user <folder>] [--haks a,b,c] [--tlk <name>] [--strict]
    [--target <name>]
```

`<module>` is anything `ModuleSource` opens (a `.mod`, a folder, a nasher project, an area file with a `module.ifo` above it). The
game is opened with the module's haks in its order (or `--haks`). It writes, per area:

- `<out>/<area>.glb`, the area;
- `<out>/<area>.floor`, the walkable floor ([The floor file](#the-floor-file));
- `<out>/<area>.json`, the manifest (below), written last;
- the textures it uses into `--tex-dir` (default `<out>/tex`), `<20 hex digits>.webp`.

| Option | Default | |
|---|---|---|
| `--tex-dir` | `<out>/tex` | Where the textures go; one folder for every area, so areas share them |
| `--tex-uri` | the texture folder relative to `--out` (`tex/`) | What the `.glb` puts before a texture's file name (a relative path or a URL) |
| `--max-tex` | 512 | A texture is the first mip level no wider or higher than this |
| `--quality` | 70 | WebP quality (lossy; alpha kept); 70 rather than the character scenes' 85 keeps a town's textures near 5 MB |
| `--objects` | on | The `.git`'s placeables and doors |
| `--jobs` | half the cores, or `NWN_BAKE_JOBS` | Areas at a time (each encodes its textures on up to 4 threads) |
| `--cache` | none | A folder of baked areas by key, and their textures (`<cache>/<key>/`, `<cache>/tex/`) |
| `--force` | | Bake every area, whatever the keys say |
| `--strict` | | A hak not found fails the area (otherwise it is a line on stderr) |
| `--draft` | | Bake a draft instead of the areas ([Drafts](#drafts)): its JSON, or `-` for stdin |
| `--target` | the target nasher packs by default | A nasher project's areas from this target, by name: for areas the packed module leaves out (its haks and talk table are those of the `module.ifo` the target lists; the keys are the same as from any other source of the same files) |

Progress goes to stderr; at the end one JSON line per area to stdout (`area`, `action`: `baked`, `current`, `cached` or `failed`,
`error`, `key`, `manifest`, `glb_bytes`, `textures`, `textures_copied`, `seconds`). The exit code is 3 when any area failed,
and 2 on misuse (an unknown option, a value that does not parse, `--all` with areas named), found before anything is read.

## Keys and the cache

Each area gets a key before anything is built, as the area pictures' quick key: `sha256:` of the format, the `.glb`s' version
(`renderer.glb`, the constant `nwn_bake::area3d::GLB_VERSION`), the options (`max_tex`, `webp_quality`, `objects`, `tex_uri`), the
area's `.are` and `.git` as binary GFF (a `.mod` and its nasher JSON give the same), the haks and talk table asked for and those
missing, and the game's files as the area pictures' quick key has them: each hak's and the talk table's SHA-256, the base game's
`nwn_base.key`, and each file in the user folder's `override/` and `development/` by name, length and time. Not the crate's version
or the build's commit: a new build that writes the same files keeps every key. With `--cache` the haks' digests are kept in
`<cache>/files.json`, so a hak is read again only when its length or time changed, and a hak touched but not changed keeps every
key. Without `--cache` no hak is read (17 GB of haks would take about a minute each run): its length and time stand for it. An
area is not baked again when:

- `<out>/<area>.json` is of this version and has the key, `<out>/<area>.glb` and `<out>/<area>.floor` have the lengths the
  manifest says, and every texture it lists is in the texture folder (`current`); or
- the cache has `<cache>/<key>/<area>.glb`, `.floor` and `.json` and the textures in `<cache>/tex/`: they are copied to `--out` and
  the texture folder (`cached`).

A manifest of another version, or one whose floor file is missing, is never `current` or `cached`.

Files are written through a temporary file and a rename. A texture file that is already there (same name, same length) is not
written again. Any hak change gives every area a new key; a finer key (the digest of the resources the scene read, as the pictures'
"exact key") can follow.

## Drafts

`--draft <file>` (or `--draft -`, read from stdin) bakes a layout that is not a file yet: a tile painter's draft, for a 3D preview
before it is applied. The draft is made into an area's `.are` and `.git` (`nwn_bake::area::draft`) and then baked as any area is:
the same `.glb`, floor file, manifest, keys and cache. Its area is the one area named (`<module> <area>`), or the lone area file
given (`x.are.json`), which is its **template**; with the module alone there is no template.

```json
{
  "name": "house_tin01",
  "tileset": "tin01",
  "width": 6, "height": 5,
  "tiles": [{"index": 0, "tileid": 311, "orientation": 2, "height": 0}, "... one per cell, row by row from the south-west"],
  "origin_dx": 1, "origin_dy": 0,
  "drop_objects": [{"list": "placeable", "tag": "chair", "x": 15.0, "y": 5.0}]
}
```

| Field | | |
|---|---|---|
| `width`, `height` | required | 1 to 32 tiles a side |
| `tiles` | required | `width * height` tiles in index order (`y * width + x`): `tileid` (0 or more) and `orientation` (0 to 3) required; `index` (when given, its place in the list), `height` (default 0) and `animloop1`..`animloop3` optional. The form of the game's `SetTileJson` |
| `tileset` | without a template | A template's tileset is kept: when given, it must be the template's |
| `origin_dx`, `origin_dy` | 0 | Where the template's south-west corner is, in tiles east and north of the draft's (an area grown 2 tiles west has `origin_dx` 2). At most 1,024 either way |
| `drop_objects` | none | Template objects left out: `list` (`door`, `placeable`, `trigger`, `waypoint`, `sound`), `tag`, and `x`, `y` in template coordinates, matched within half a metre |
| `keep_tags` | none | Tags `drop_objects` never drops: a tag exactly, or `prefix*` for every tag that starts with `prefix` |
| `name` | the template's resref, else `draft` | The resref of the files written (`<name>.glb`) and of the area in them |

Any other field is refused. With a template, the draft is made the way a module that repaints an area by script would build it
from its template, so the preview shows what the game will build:

- the `.are` gets the draft's `Width`, `Height` and `Tile_List`. Each tile starts from the template's tile at the same place once the
  origin is taken off, or from the template's first tile where the template had none (so it keeps the template's lights), and takes
  the draft's `Tile_ID`, `Tile_Orientation`, `Tile_Height` and any `Tile_AnimLoop` given. Every other field is the template's;
- every object of the `.git`'s lists with a position (`X`/`Y` for doors and placeables, `XPosition`/`YPosition` for the rest, an
  encounter's `Geometry` and `SpawnPointList` with it) moves by the origin, 10 m a tile. It is left out when `drop_objects` names it
  or when it then stands outside the new size. An object whose tag `keep_tags` names is never dropped by name (it still leaves
  when it stands outside the new size).

Without a template the area is a bare `.are` (the tileset, the size and the tiles; no name, lights or flags) and no `.git`: tiles
only, and `origin_dx`, `origin_dy`, `drop_objects` and `keep_tags` must be left out (or empty).

A draft is checked before the game is opened; one that does not fit (the tile count, a turn of 4, another tileset than its
template's, an unknown field) fails with exit code 1 and the reason on stderr. Naming more than one template, or `--all`, is misuse
(exit code 2).

The key is the area's as always (the made `.are` and `.git` as binary GFF, with the haks, options and game files), so a draft is a
hash of its input: baked once, then `current` or `cached` (a preview service keeps the result by the key, `<cache>/<key>/`, as the
batch does). A draft of a template's own size, tiles and origin gives the template's files byte for byte, so it has the template's
key and bakes to the same `.glb` and floor file. Use the template's area file as the module (`house_tin01.are.json`): then
opening is the game alone, not a whole module.

## The `.glb`

glTF 2.0, binary. Extensions, all **required**:

- `EXT_mesh_gpu_instancing`: the tiles. three.js's `GLTFLoader` reads it into `InstancedMesh`es (built in since r146; tested with
  three 0.170.0).
- `EXT_texture_webp`: every texture, with no PNG fallback (`GLTFLoader` reads it; every browser that runs the page reads WebP).

**Axes:** glTF is Y-up. A game position (x east, y north, z up, metres, the area's south-west corner at 0) is the `.glb`'s
**(x, z, -y)**. A turn about the game's vertical (a tile's quarter turns, an object's `Bearing`, counter-clockwise) is the same angle
about glTF's +Y. UVs have v down (1 - the model's v); the texture files have their top row first.

**Nodes:** one root, named after the area, `extras: { format, version, area, tileset, tiles: [w, h], axes: "y-up" }`. Its
`version` is the `.glb`'s layout version (`nwn_bake::area3d::GLB_LAYOUT`), 1; it is not the format's version (2), which also counts
the floor file. Its children:

| Node | Name | Mesh | `extras` |
|---|---|---|---|
| Tiles, one node per distinct tile mesh | `tiles_<model>` | instanced: `TRANSLATION` and `ROTATION` per tile | `part: "tiles"`, `model`, `tiles`: the `.are` `Tile_List` indices, in the instances' order |
| A placeable | `placeable_<index>_<tag>` | its model's mesh (shared by every copy) | `part: "placeable"`, `index` (in the `.git`'s `Placeable List`), `tag`, `resref` (`TemplateResRef`), `model`, `x`, `y`, `z`, `bearing` (the `.git`'s values, unrounded, game axes, radians) |
| A door | `door_<index>_<tag>` | the same | as a placeable's (`part: "door"`, `index` in the `Door List`), and `front_deg`, `locked` |
| Water | `water` | the area's water faces in the area's space | `part: "water"` |

A node name keeps only letters, digits, `_` and `-` of the tag (three.js drops some characters). three.js puts a node's `extras` in
`object.userData`. Find a door by its tag and position (within 1 m), not by index alone, so a reordered `.git` can't mismatch.

**Meshes:** each model is written once, in its own space (as the game holds it), one primitive per texture, vertices welded (the same
position and UV are one vertex), indexed (16-bit when they fit), with normals (area-weighted face normals summed at each vertex). A
tile model's water faces are not in its mesh: they are the water node. When stray geometry (more than 15 m outside the area, below
-60 m or above 80 m: the scene's rule) is left out of one placement of a model, that placement gets its own mesh (`<model>~<n>`), so
the file draws exactly the scene's triangles.

**Materials:** one per texture: `pbrMetallicRoughness` with the texture as `baseColorTexture`, metallic 0, roughness 1,
double-sided; `alphaMode: MASK`, `alphaCutoff: 0.4` when the texture has see-through texels (alpha under 250). A texture the game
does not have is a grey material. Water: colour (0.173, 0.416, 0.525), opacity 0.62 (`BLEND`), as the area pictures draw it.
Samplers repeat and mip-map.

**What is not in it:** tile animations, day and night meshes, lights, shadows, fog, particle emitters, creatures, sounds, the roof
cut (interiors keep their ceilings; the pictures cut them).

## The floor file

`<area>.floor`: where the area's floor can be walked, and how high it is, every 0.625 m. It is for checks a page or a server makes
without the game: whether a spot is on walkable floor, the floor's height there, and which floor can be reached from a door. It is
made from the tiles' walkmeshes (`.wok`), as the game walks them, by the same floor probe the area pictures' roof cut uses
(`nwn_bake::area::floor`), at the same points.

**The grid:** 16 samples a tile's side, so a `w` by `h` tiles area has `nx = 16 w + 1` by `ny = 16 h + 1` samples (a 6x4 house:
97 by 65). Rows from the south, each from west to east: sample `(i, j)` is at index `j * nx + i`, at about
`(i * 0.625, j * 0.625)` metres from the area's south-west corner, in game axes. Precisely, each sample is tested 0.0098 m past
that point (the centre of a pixel of a 512-pixel raster a tile), and the samples on the area's north and east edges 0.0098 m short
of it, inside the area. To find the sample nearest a spot `(x, y)`: `i = round(x / 0.625)`, `j = round(y / 0.625)`, clamped to the
grid.

**A sample is walkable** when the topmost face of the tile's walkmesh there is one the game walks (`surfacemat.2da`'s `Walk` is
`1`). A wall's foot, a chasm's floor, deep water or a spot with no face at all is not. Of two faces at one height (a shared edge),
the walkable one wins.

**A sample is blocked** when one of the unwalkable faces of a `.git` placeable's walkmesh (the `.pwk` of its model), placed at the
placeable's position and bearing, overlaps the sample's cell: the 0.625 m square about the sample. The cell, not the point: a fence 0.5 m
thick can lie between two rows of samples, and a flood fill must not pass through it. Doors are not counted (their walkmeshes are `.dwk`
files, and a door opens). With `--objects off` nothing is blocked. The flag is beside the walkable one, not instead: a blocked sample is
still walkable floor.

**Layout** (little-endian; a 6x4 house is 18,931 bytes, a 16x16 town area 198,163):

| Offset | Bytes | What |
|---|---|---|
| 0 | 4 | `NBFL` |
| 4 | 2 | `u16`: the file's version, 1 |
| 6 | 2 | `u16`: samples a tile's side, 16 |
| 8 | 2 | `u16`: `nx` |
| 10 | 2 | `u16`: `ny` |
| 12 | 4 | 0 (reserved) |
| 16 | `2 nx ny` | `i16` a sample: its height in centimetres. On walkable floor, the height of the walkable face; elsewhere the probe's ground (the walkmesh's top, filled in from the walkable faces around it where there is none) |
| `16 + 2 nx ny` | `nx ny` | `u8` a sample: bit 0 (1) walkable, bit 1 (2) blocked by a placeable; the other bits 0 |

The heights come first so they start at an even offset (an `Int16Array` in a browser, `struct.unpack_from` or
`array('h')` in Python, read them in place). A reader checks the magic, the version and the length (`16 + 3 nx ny`).

**Using the floor file:**

- **on walkable floor:** the sample nearest an object's spot has bit 0; its `z` is that sample's height (centimetres over 100);
- **near walkable floor** (say within 0.7 m, for something that stands against a wall): a walkable sample within 0.7 m of its
  spot (look two samples each way);
- **cut-off floor:** a flood fill over the walkable samples, from the sample nearest an entry, through the four neighbours, leaving
  out blocked samples and the samples whose cell an object's footprint overlaps, for the objects whose model has a placeable walkmesh
  (`nwn-bake placeables`: `walkmesh.footprint`, below; the same cell rule as the file's); a walkable sample it does not reach is
  cut off;
- **after a layout change:** an object or a saved position whose nearest sample is no longer walkable needs a new place.

## Placeables' walkmeshes (`nwn-bake placeables`)

A check that places objects needs each placeable appearance's footprint. `nwn-bake placeables <out.json>` (and the `placeables_index`
request of `serve`) gives, for every `placeables.2da` row with a model, besides its `size`:

- `bounds`: the lowest and highest corner of its meshes at rest, `[[x, y, z], [x, y, z]]`, model space, metres to the
  centimetre (x east and y north as the model faces at bearing 0);
- `walkmesh`: its placeable walkmesh (`.pwk` of the model's name), or null when it has none: `blocking_faces`, how many of its
  faces can't be walked, and `footprint`, the ground they cover, `[[x, y], [x, y]]` in model space (null when no face blocks).

A footprint is placed as the game places the object: turned by its bearing (counter-clockwise from east), then moved to its spot.

## Textures

Each texture the meshes use: the image the texture name means (an EE material's `texture0` first, then DDS or TGA, as the game
finds it), the first mip level that fits in `--max-tex`, rows from the top, encoded as lossy WebP at `--quality` (libwebp keeps
alpha only when the image has see-through texels). The file is named by the first 20 hex digits of the SHA-256 of its bytes, so:

- two areas that use the same image share one file (the areas of one tileset share nearly all their tile textures);
- a texture that did not change keeps its name from one bake to the next (the same image and options give the same bytes), so a
  browser caches it for good.

Each image is encoded once per run however many areas use it. Two texture names that mean the same image are one file and one
glTF texture (their materials stay apart).

## The manifest (`<area>.json`)

```json
{
  "format": "nwn-bake/area-3d",
  "version": 2,
  "renderer": { "name": "nwn-bake", "version": "0.5.0", "commit": "abc1234", "glb": 2 },
  "key": "sha256:...",
  "area": { "resref": "town_square", "name": "Town Square", "tileset": "tcn01", "tiles": [16, 16], "size_m": [160.0, 160.0], "flags": 4,
            "kind": "surface", "light": { "ambient": [80, 80, 80], "diffuse": [200, 200, 200] } },
  "axes": "y-up",
  "glb": { "file": "town_square.glb", "bytes": 2512345, "sha256": "sha256:..." },
  "floor": { "file": "town_square.floor", "bytes": 198163, "sha256": "sha256:...", "version": 1, "spacing_m": 0.625,
             "per_tile": 16, "samples": [257, 257], "unit_m": 0.01, "walkable": 52011, "blocked": 1210 },
  "tex_uri": "tex/",
  "textures": [ { "file": "0123456789abcdef0123.webp", "bytes": 21544, "width": 512, "height": 512, "alpha": false } ],
  "counts": { "tiles": 256, "placeables": 40, "doors": 31, "models": 90, "meshes": 90, "tile_nodes": 70,
              "triangles_stored": 60000, "triangles_drawn": 210000, "vertices": 70000, "water_triangles": 0,
              "textures": 60, "textures_missing": 0 },
  "bounds": [[-1.0, -1.0, -5.0], [161.0, 161.0, 22.0]],
  "doors": [
    { "index": 7, "tag": "Inn_Door", "resref": "inn_door", "name": "Strong Door", "model": "inn_door",
      "node": "door_7_Inn_Door", "x": 34.95, "y": 120.78, "z": 0.0, "bearing": 0.0, "bearing_deg": 0.0,
      "front_deg": 270.0, "front": [0.0, 0.0, 1.0], "locked": true, "linked_to": null }
  ],
  "notes": [],
  "inputs": { "are": "sha256:...", "git": "sha256:...", "haks": ["..."], "missing_haks": [], "tlk": "mymodule" },
  "options": { "max_tex": 512, "webp_quality": 70.0, "objects": true }
}
```

The fields the two manifests share are named and typed alike: `format`, `version`, `renderer` (`name`, `version`, `commit`; the
area pictures' adds `pictures`), `key`, `area` (`resref`, `name`, `tileset`, `tiles`, `size_m`, `kind`), `inputs`, `notes`, and
`options.webp_quality`. Every digest is `sha256:` and 64 hex digits. `docs/area-3d.schema.json` is the manifest as a JSON Schema
(2020-12); the tests check the manifests they write against it. Like the format, it allows fields it does not list.

- `renderer`: what baked the area. `version` (the crate's) and `commit` (the build's first 7 characters, null when it was not
  built from a git checkout) are for information only. `glb` is the version of what a bake writes, and is part of the key (above):
  any change that alters a `.glb`, a texture file, a floor file or a manifest for the same inputs raises it (`docs/testing.md`).
- `area.size_m` is the tiles times 10 m. `area.flags` is the `.are`'s `Flags` (1 interior, 2 underground, 4 natural); `kind` follows
  from it as in the area pictures. `area.light` is the area's sun, `SunAmbientColor` and `SunDiffuseColor` as RGB.
- `glb.sha256` is the digest of the `.glb`'s bytes.
- `floor` names the floor file and describes its grid: `version` is the file's own (as in its header), `samples` the samples along
  x and y, `walkable` how many samples are walkable floor and `blocked` how many of those a placeable of the `.git` blocks.
- `options.webp_quality` is `--quality`: the textures' WebP quality, 0 to 100.

- `doors` lists **every** door of the `.git`, drawn or not (`model` and `node` are null when the game has no model for it, or with
  `--objects off`), so a page can answer the door list without opening the `.glb`. `front_deg` is the direction the door's street
  side faces, degrees counter-clockwise from east (the door panel lies along its bearing; the street side faces the bearing less
  90 degrees, as measured on real doors), and `front` the same as a unit vector in the `.glb`'s axes.
- `bounds` are game axes.
- `counts.triangles_drawn` counts every instance; `triangles_stored` each mesh once.
- `notes` are the scene's (a tileset's absent sections, models and textures the game does not have) and textures that could not be
  encoded.

The manifest has no times in it: the same inputs give the same `.glb` and manifest, byte for byte.

## Tests

- `nwn-bake/src/area3d/gltf.rs` (unit, no game): welding and normals, a `.glb` read back with its instances, node extras and texture
  URIs, 32-bit indices, a broken file refused.
- `nwn-bake/src/area/floor.rs` (unit, no game): the walkable floor is the top walkmesh face where it walks (a wall's foot over a
  floor, deep water under a bridge, two faces at one height), the samples' places, a placeable's block, the floor file read back.
- `nwn-bake/tests/fixture_area3d.rs` (the generated fixture): the placements give the scene's triangles bit for bit for every
  fixture area, and the `.glb` drawn from its instances and nodes is the scene (and its water); tiles turned every way and lifted as
  instances, the mesh stored once; the door and the box as named nodes with their `.git` values and the door list; textures named
  by content, shared between two areas, the same bytes twice and from every module form; the batch's `current`, `cached` and
  `baked`, a missing texture, another option's key, `--force`.
- `nwn-bake-cli/tests/area_3d.rs`: the command end to end with real WebP, a second run all `current`, an area file on its own with
  the textures elsewhere, the exit codes; a draft of an area's own layout gives its key, `.glb` and floor file, a draft grown a tile
  west (from stdin, under its own name) moves the door, a draft without a template, and a draft that does not fit.
- `nwn-bake/src/area/draft.rs` (unit, no game): a draft made into an area's files: the template's own layout byte for byte, growing
  west and south (tiles, lights, every list, an encounter's points), objects dropped or outside left out but never the entry, a
  bare area, and the drafts refused.
- `nwn-bake-cli/tests/golden_area3d.rs`: only with `NWN_BAKE_GAME` (as the other golden tests): `q1rural` of the base game, its
  placements checked against the scene, its counts against `tests/golden/area3d-q1rural.json`, and every door of its `.git` as a
  node with its tag and position; and `q1rural` as a draft of its own layout, the same `.glb` and floor file, and grown a tile west
  and south, every door 10 m east and north.

## Measured (2026-11-10, an 8-core server, release build, `--jobs 1`, defaults: 512 px, quality 70)

Five town areas of a large module with custom haks (a nasher project of about 1,300 areas), baked cold one after another into one
texture folder:

| Area | Tileset | Tiles | Doors | Triangles stored / drawn | `.glb` (gzip) | Textures (new here) | Bake (quiet / busy) |
|---|---|---|---|---|---|---|---|
| A | tcn01 | 16x16 | 54 | 48k / 141k | 2.98 MB (0.95) | 92, 4.9 MB (92, 4.9 MB) | 2.8 s / 11.0 s |
| B | tcn01 | 12x12 | 14 | 23k / 44k | 1.59 MB (0.44) | 81, 4.2 MB (18, 0.9 MB) | 0.9 s / 4.5 s |
| C | tno01 | 12x12 | 21 | 55k / 88k | 3.36 MB (1.30) | 121, 7.6 MB (106, 6.8 MB) | 3.0 s / 9.4 s |
| D | tcr10 | 10x10 | 36 | 35k / 68k | 2.15 MB (0.69) | 165, 8.5 MB (117, 6.1 MB) | 3.8 s / 11.8 s |
| E | tcr10 | 21x25 | 34 | 67k / 223k | 4.19 MB (1.37) | 183, 10.2 MB (46, 2.6 MB) | 4.2 s / 11.0 s |

Bake times: "quiet" with the machine's load at 1-3 (measured at quality 85; the quality barely changes the encoding time), "busy" at a
load of 10-17 from other builds. Most of a cold bake is encoding textures (quiet: 0.5-2.7 s); the scene is 0.3-1.9 s, the placements
and their check under 0.3 s, the `.glb` under 0.2 s. A second run finds every area current in about 10 ms each; from the cache, 30
ms. Opening the module and the game took 25-115 s before the first area, once a run. Textures average about 55 KB at 512 px and
quality 70 (4.9 MB for area A; quality 85 was 7.0 MB, `--max-tex 256` at 85 1.8 MB). The second area of a tileset reuses 60-78 % of
its textures. The five areas need 379 files, 21.4 MB.

three.js 0.170.0's `GLTFLoader` in node loads all five (instances, door `userData`, every texture fetched as WebP) in 0.3-0.7 s of
parsing on a quiet machine. Each tile mesh has one primitive per texture, so a town is 600-1,200 `InstancedMesh`es plus 40-120
meshes: that many draw calls.

## Missing textures

A face whose texture the game has no image for is grey, and the manifest's `notes` name the texture. In the areas measured above,
four texture names were grey (184 stored triangles in all). They were absent from the game data, not missed by the bake: no
resource of those names, of any type (MTR, DDS, TGA, PLT), was in the base game's keys and bifs, its `ovr/`, any hak in the user
folder or an `override/`. The base game's own tile models do not name them; a hak's replacements of those tile models do. So the
game has no image for those faces either: a gap in that hak, which the notes make easy to find.
