# Area pictures: the manifest (`nwn-bake/area-iso`, version 1)

`nwn-bake area-iso <module> <area>... --out <dir>` draws each area from the views asked for and writes, in `<dir>`:

- one picture per view, named by `--name` (default `{area}.{view}.{ext}`: `village.sw.webp`);
- with `--depth`, one depth picture per view (`{area}.{view}.depth.png`);
- `<area>.json`, the manifest described here.

`nwn-bake area-iso <module> --all --out <dir>` draws every area of the module (see "Batches" below) and lays the folder out by
kind and view, for a website or service to read (`--layout tree`, the default with `--all`; `--layout flat` is the side-by-side
layout above):

- `webp/<view>/<area>.webp` (or `png/...` with `--format png`), `depth/<view>/<area>.png` with `--depth`;
- `json/<area>.json`, each area's manifest;
- `manifest.jsonl`, one line per area and view (below), and `index.json`, each area's status and keys.

The pictures are WebP by default: lossy, quality 85, the alpha kept (libwebp, method 4, as Pillow writes them), with a transparent
background. `--format png` writes PNG (RGB with `--background #rrggbb`, else RGBA).

## Views

| View | Camera |
|---|---|
| `sw` | From the south-west corner, looking north-east, 30 degrees above the horizon |
| `se`, `ne`, `nw` | The same from the other corners: each a quarter turn more than the one before |
| `top` | Straight down, north up, east right |

`--view all` is the four corners. Every view is drawn at the same scale (`--scale`, pixels a metre). The light is set relative to the
camera, so the four corner views are lit alike; the top view is lit as `sw` is.

## Example

```json
{
  "format": "nwn-bake/area-iso",
  "version": 1,
  "renderer": { "name": "nwn-bake", "version": "0.2.0", "commit": "abc1234", "pictures": 1 },
  "area": { "resref": "village", "name": "The Village", "tileset": "ttr01", "tiles": [8, 8], "size_m": [80.0, 80.0], "kind": "surface" },
  "inputs": { "are": "sha256:...", "git": "sha256:...", "haks": ["hak_a", "hak_b"], "missing_haks": [], "tlk": "mytlk" },
  "resources": null,
  "key": null,
  "options": { "scale": 8.0, "quality": 2, "light": "day", "shadows": true, "roof": "auto", "exposure": "auto", "objects": true,
               "background": "transparent", "look": "default" },
  "roof_cut_m": null,
  "views": [
    {
      "view": "sw", "file": "village.sw.png", "depth": null,
      "width": 1827, "height": 1003, "scale": 8.0, "elevation_deg": 30.0,
      "rotation": [[1, 0], [0, 1]], "offset": [914.6, 640.2],
      "matrix": [[5.657, -5.657, 0.0, 914.6], [-2.828, -2.828, -6.928, 640.2]]
    }
  ],
  "notes": ["texture tdm01_xyz: not found", "hak hak_c not found"]
}
```

## Fields

- `renderer`: what drew the pictures. `pictures` is the pictures' version (see "Keys"); `version` (the crate's) and `commit` (the
  build's first 7 characters, null when it was not built from a git checkout) are for information only and are not part of the
  keys.
- `area.kind`: `surface`, `interior` or `underground`, from the `.are`'s `Flags` (bit 0 interior, bit 1 underground); an area on an
  interior tileset counts as an interior. `size_m` is the tiles times 10 m.
- `inputs.are`, `inputs.git`: SHA-256 of the `.are` and `.git` as binary GFF (the same for a `.mod` and for its nasher JSON). `git` is
  null when the area has none. `haks` is the list the game was opened with, first highest; `missing_haks` those found nowhere.
- `resources`, `key`, `quick_key`: the digest of every resource the scene looked up, the exact key and the quick key (see "Keys"
  below). `quick_key` is null when the run made no quick keys (named areas with no `--cache`).
- `options`: the options in effect. `quality` is the supersampling, set with `--supersample`. `roof` is `"auto"`, `"keep"` or the
  metres over the floor given; `look` is `"default"`, or the whole set of calibration constants when `--look` changed any;
  `format` is `"webp"` or `"png"`, with `webp_quality` for WebP; `depth` whether there are depth pictures.
- `roof_cut_m`: the height (metres) above which everything was cut away, or null when the roof was kept. Interiors and underground
  areas are cut 3.5 m over their median floor, or, when they are a cave dug out of rock (more than a quarter of the ground at the top,
  at least 8% drawn floor 2 m or more under it), 1 to 2 m over their highest floor.
- A tileset's black void (tile faces whose texture is black all over) is not drawn, so it shows the background. The picture's size and
  `matrix` still take it into account, as if it were drawn.
- A chasm's floor is drawn in a dark blue-grey, darker the further it is from the nearest ground (`chasm_edge`, `chasm_deep`,
  `chasm_reach` and `chasm_seam` in the look), instead of its texture's black. A chasm's floor is a tile face on a tile with a corner
  of chasm terrain (named `Chasm...` or `Chasym`), level, at least 1 m below the tile's floor, black where it is drawn (every sample
  under it adds up to 30 or less over its three channels) with a texture that is not black all over, with no walkable walkmesh face
  of the tile within 1 m of its height over it and no water face over it. The exposure is measured as if it were drawn black, so
  nothing else in the picture changes. In the base game this is the underdark's chasms (ttu01) and a few faces in trm02 and
  ttd01; community tilesets with chasms have more.
- `views[].bytes`: the picture file's size.
- `views[].depth`: the depth picture's file, null without `--depth`. It is a 16-bit grey PNG as big as the picture: 0 where nothing
  is drawn, else the game height z of what the pixel shows, z = `depth_range[0]` + v / 65535 * (`depth_range[1]` - `depth_range[0]`)
  (drawn pixels are 1 or more, so the lowest height is a 65535th of the range off). Where a pixel's samples show different things
  the one nearest the camera counts, and water counts where it is over what is under it.
- `notes`: what could not be read or looked wrong: a tileset's absent sections, missing models and textures, haks not found, a texture
  drawn nearly white over much of a picture.

## Game coordinates and pixels

- Game coordinates are metres as in the `.git`: x east, y north, z up, the area's south-west corner at (0, 0).
- `matrix` (2 by 4) maps a game point (x, y, z, 1) to a pixel position (px, py) measured from the picture's top-left corner:
  `px = m[0][0] x + m[0][1] y + m[0][2] z + m[0][3]`, `py = m[1][0] x + m[1][1] y + m[1][2] z + m[1][3]`. Pixel (i, j) covers
  [i, i+1) by [j, j+1), so its centre is at (i + 0.5, j + 0.5). **`matrix` is the normative part.**
- For `top`, `m[0][2]` and `m[1][2]` are 0: a pixel does not depend on the height, and x and y follow from the pixel directly.
- `rotation`, `offset`, `scale` and `elevation_deg` describe how the matrix was made: the view's quarter turn (`x' = r[0][0] x + r[0][1] y`,
  `y' = r[1][0] x + r[1][1] y`), then for a corner view the 45-degree axes a = (x' - y') / sqrt 2 and b = (x' + y') / sqrt 2 (for `top`,
  a = x' and b = y'), then px = a scale + offset[0] and py = -(b sin e + z cos e) scale + offset[1], e being `elevation_deg`. They
  give the same pixels as `matrix`, for a reader that builds its own transform from them.
- From a pixel back to the game needs the height: with z known (from the depth picture), x and y solve the 2 by 2 system of the
  matrix's first two columns.

## `manifest.jsonl`

One JSON object a line, one line per area and view, in resref and view order (`area-iso-line.schema.json`). What a reader needs first
is at the top: `resref`, `view`, `path` (the picture, relative to the folder), `width`, `height`, `bytes`, `status` (`ok`) and
`transform` (`view`, `elevation_deg`, `scale`, `width`, `height`, `rotation`, `offset`, `matrix`). Then the area's manifest without
its `views` (`format`, `version`, `renderer`, `area`, `inputs`, `resources`, `key`, `quick_key`, `options`, `roof_cut_m`, `notes`),
and the view's own entry as `view_entry`. An area that failed keeps the lines of the pictures it already had; one with none has a
line per view with `status` `failed`, `version` 1 and the `error` (a reader that takes the last `ok` line of each area and view
ignores those). The file is written whole, and only when it changes.

## Keys

Every area has two keys, SHA-256 in hex after `sha256:`. Both start from the same part:

- the key format's own tag, the manifest's `version`, and the renderer: its name and the **pictures' version**
  (`renderer.pictures`, the constant `nwn_bake::iso::PICTURES_VERSION`). Not the crate's version or the build's commit: a new build
  that draws the same pictures keeps every key, so a release of nwn-bake does not draw every area again;
- the options in effect, as the manifest's `options` (scale, quality, light, shadows, roof, exposure, objects, background, look,
  format and WebP quality, depth) and the views asked for, in order;
- the area's resref and its `inputs` (the `.are` and `.git` as binary GFF, so a `.mod` and its nasher JSON give the same; the haks
  asked for, in order; the haks not found; the talk table's name).

The **quick key** adds what the game's files are, which needs no scene: for each of the game's resource layers in order (the haks,
the user folder's `override/` and `development/`, the base game) its label and kind, with a hak's (or any single file's) SHA-256,
a folder's list of files (name, size, time); the SHA-256 of the base game's `data/nwn_base.key` and of the custom talk table. Any
change to any hak changes it. Reading 17 GB of haks takes a minute or two the first time; the digests are kept in the cache folder
(`v1/files.json`, by the file's real path, size and time) and read again only for files that changed.

The **exact key** adds, instead, the area's name as the manifest gives it (from the talk table) and `resources`: the digest of every
resource the scene looked up, found or not, sorted by name and how it was looked up. For each, its name, the lookup (`set`, `2da`,
`mdl`, `wok`, `pwk`, `mtr` or `texture`) and the SHA-256 of what the game finds (for `texture`, the type found, DDS or TGA, and its
bytes), or that it finds nothing. They are: the tileset (`.set`); `surfacemat.2da`; each tile model's `.mdl`, `.wok` and `.pwk`;
the `placeables`, `doortypes` or `genericdoors` 2DA of each placeable and door, and the model its row names (`.mdl` and `.pwk`);
for each texture name a mesh uses, its material (`.mtr`) and the image it means. Which hak or folder a resource comes from does
not matter, only its bytes. Working the exact key out needs the scene (about a second) but no drawing.

**When to raise the pictures' version.** Raise `PICTURES_VERSION` by one in any change that alters a picture or a manifest: the
look or its constants, the scene (what is drawn, how it is lit, cut or placed), the camera or the transform, the encoders and their
settings, a field of the manifest. Every key then changes and every area is drawn again on the next run. A change that alters
neither (speed, logging, the cache's own files, the CLI) keeps it. A test guards the rule: `tests/pictures_version.rs` draws the
fixture's six areas in all five views with the default options, in PNG and WebP, and compares the digests of the pictures and
manifests with those recorded in `tests/pictures.json` for the current version. When they differ and the version is still the
recorded one, it fails and says to raise the version; after raising it, write the digests again with
`NWN_BAKE_PICTURES=write cargo test -p nwn-bake --features webp --test pictures_version` (see `docs/testing.md`).

The same inputs give the same keys and the same files: the scene is built in a fixed order, each view is drawn on one thread in the
scene's order, the views and areas are put together in a fixed order whatever the number of threads, and libwebp encodes on one
thread.

## Batches and the cache

`--all` (or several areas by name) works on `--jobs` threads: by default half the cores, or `NWN_BAKE_JOBS` (a batch
usually shares its machine). An area is first checked, then drawn, its views on several threads at once; the threads draw
the views of the areas already started before starting more, so few areas are in memory at once. For each area:

1. The quick key (with `--all` or `--cache`): when the area's manifest in the output folder has this quick key and an exact key,
   and its pictures are there with the sizes it says, the area is **current**: nothing is read or written.
2. With `--cache`, when the cache knows the exact key of this quick key and has its pictures, they are put in the output folder
   (hard links where they can be, else copies): **cached**.
3. Otherwise the scene is built and the exact key worked out. When the output folder's manifest has this exact key, only its
   `quick_key` is written (the pictures keep their files and times): **current**. When the cache has it: **cached**.
4. Otherwise the area is drawn and written to the output folder and the cache: **rendered**.

`--force` draws every area anyway. Files are written through a temporary name beside them, so a reader never sees half a file.

An area **fails** with a message and the batch goes on when its files do not read, when its `.are` is past 32 tiles a side or its
`Tile_List` does not match its `Width` and `Height`, when a view would be over 16,383 pixels a side or 2^28 samples (the scale
times the supersampling; checked before anything is allocated), and when its work panics (the error starts `panicked:`). A
progress line that cannot be written (stderr or stdout closed) is dropped.

The cache folder (`--cache <dir>`) holds `v1/exact/<ab>/<key>/` (each view's picture `<view>.<ext>`, depth picture
`<view>.depth.png`, and `manifest.json`), `v1/quick/<ab>/<key>` (the exact key a quick key was found to have) and `v1/files.json`.
Nothing is removed from it; delete it to start again.

Each finished area is a line on stderr (`[n/total] area: rendered (views and sizes); check, scene, drawing and writing seconds`),
and stdout has one JSON line per area at the end, in the order asked: `area`, `status` (`ok` or `failed`), `action`, `manifest`,
`key`, `quick_key`, `roof_cut_m`, `views` (each drawn view's seconds and exposure gain) and `seconds`. The exit code is 3 when any
area failed, and 2 on misuse (an unknown option, a value that does not parse, `--all` with areas named), found before anything
is read.

## serve

`nwn-bake serve` answers the same requests for single areas (`area_iso` and `area_iso_key`), with the same options and the same
manifests: see `serve.md`.

## Schemas

`area-iso.schema.json` beside this file is the manifest as a JSON Schema (2020-12), and `area-iso-line.schema.json` a line of
`manifest.jsonl`. The tests check every manifest and line they write against them. Like the format, they allow fields they do not
list.

The areas in 3D have a manifest of their own (`area-3d.md`, `area-3d.schema.json`); the fields the two share are named and typed
alike.

## Versions

`version` changes when a field changes meaning or goes away. Added fields do not change it; readers ignore fields they do not know.
