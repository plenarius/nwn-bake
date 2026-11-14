# Changelog

All notable changes to nwn-bake. Versions follow [Semantic Versioning](https://semver.org/); before 1.0, a minor version may
change the library's API. The area manifest has its own version (`docs/area-iso.md`), and the cache keys hold the pictures'
version (`PICTURES_VERSION`), which changes whenever a picture would.

## Unreleased

- **`placeable`: a placeables.2da row as one scene** (`scene.json` and `tex/`, as `creature` writes a character's), for a web page that
  draws furniture in 3D: the model as a placed piece shows it (`placeable-thumb`'s), in its idle animation unless `--anim` says
  otherwise, with `--tex OLD=NEW` replacements (the furniture catalogue's styles). The scene names the row, its model and its name
  (the row's StrRef, else its label). Library: `creature::bake_placeable`; `bake_creature` and it share the scene writer, and a
  creature's `scene.json` is as before.

## 0.3.0 (2026-11-13)

- **Memory: `--cache-mb <MiB>` and `NWN_BAKE_CACHE_MB`**, on every command that opens the game, `serve` included: what the
  model and texture caches keep between uses, a quarter for models and the rest for textures (default 4096, as before: 1 GiB and
  3 GiB). `0` keeps as little as can be; anything but a whole number of MiB is refused (exit code 2 for the option, 1 for the
  variable). The limit covers the whole process: `serve`'s games, one per hak list, share it instead of each keeping the defaults.
  Pictures, `.glb`s and keys do not change (`PICTURES_VERSION` and `GLB_VERSION` stay). The README has a "Memory" section with
  measured peaks.
- Library: `game::CacheBudget` and `game::CacheLimits`, and `game::open_in` to open a game in a budget shared with other games;
  `game::open` keeps a budget of its own per game, as before. `Game::cache_budget`; `Game::set_cache_limits` now sets its budget's
  limits (shared by every game in it) and drops what is kept beyond them at once. `iso::batch::Service::cache`, the budget its
  games share.
- `deny.toml` also allows `https://github.com/plenarius/nwn-bake` as a git source, so programs that take nwn-bake from git at a
  release tag can share the file word for word.

## 0.2.0 (2026-11-12)

The first public release.

- **Area pictures** (`area-iso`): an area drawn straight from its files (the `.are`, `.git`, the tileset, its tile models and
  walkmeshes) on the CPU, from the four corners at 30 degrees and straight down. WebP (quality 85, transparent background) or PNG,
  optional 16-bit depth pictures. Interiors and underground areas are cut open over their floor; dark areas are lifted; black void
  is left out.
- **The manifest** (`nwn-bake/area-iso`, version 1, with JSON Schemas): each picture's transform from game coordinates to pixels,
  the inputs' digests and the cache keys.
- **Batches and a cache**: `--all` renders a whole module in one process on several threads; a quick key (the hak set) and an exact
  key (every file the area reads) keep the pictures current, so a module or hak change redraws only the areas it touches. A panic
  in one area's work fails that area with the panic's message and the batch goes on; a closed stderr or stdout (`| head`, `2>&-`)
  stops nothing. A picture over 16,383 pixels a side or 2^28 samples is refused before anything is allocated; an `.are` past 32
  tiles a side, or whose `Tile_List` does not match its size, fails its area.
- **Areas in 3D** (`area-3d`, `nwn-bake/area-3d` version 2, with a JSON Schema): one glTF binary per area
  (`EXT_mesh_gpu_instancing`, `EXT_texture_webp`), each model stored once and the tiles instanced, the doors and placeables as
  named nodes with their `.git` values, water, and the textures as WebP files named by their content and shared by every area.
  The manifest lists every door with the direction its street side faces. Batches, keys and a cache as for the pictures; the keys
  hold the `.glb`s' version (`GLB_VERSION`), read `development/` too, and know a hak by its SHA-256 with `--cache`, so a touched
  hak bakes nothing. The same inputs give the same files, byte for byte.
- **The walkable floor**: beside each `.glb`, `<area>.floor`, the tiles' walkmeshes sampled every 0.625 m (16 a tile): whether the
  game walks there, the floor's height in centimetres, and whether a `.git` placeable's walkmesh blocks it. For checks such as
  whether a spot is on walkable floor, its height, and which floor can be reached from a door. `placeables` (and `serve`'s
  `placeables_index`) gives each model's `bounds` and its placeable walkmesh's `footprint`.
- **Drafts** (`area-3d --draft`): a layout from a tile painter (a size, a tile list, an origin, template objects to leave out and
  tags to keep) made into an area from its template and baked as any area, with the same keys and cache: a 3D preview before the
  layout is applied. A draft of a template's own layout gives its files byte for byte, so the same `.glb` and floor file.
  `nwn_bake::area::draft`.
- **Both manifests share one shape**: the same names for the shared fields (`renderer`, `area.size_m`, `options.webp_quality`),
  every digest `sha256:` and hex, the commit always 7 characters, and only a talk table that was read recorded. The schemas'
  `$id`s are their raw files.
- **`area-scene`**: an area's scene, its floor statistics and its digest.
- **Characters as parts** (`describe`, `look`, `part`), pictures of creatures, items and placeables (`thumb`, `item-thumb`,
  `placeable-thumb`, `creature`), the placeables index, flattened models and textures as PNG.
- **`serve`**: the character, texture, placeables and area picture requests (`area_iso`, `area_iso_key`) as JSON lines over
  stdin and stdout. `textures_of` and `placeables_index` work without a GPU.
- **Modules in every form**: `.mod`, a folder of binary files, a folder of `nwn_gff` JSON files, a nasher project (read lazily,
  by its default target or by name: `ModuleSource::open_target`, `--target`), or one area file; haks in the module's order.
- **`resources`**: the names of the icons (TGA, PLT and DDS named `i...`) and weapon part models the game has, as JSON, for a
  list of item appearances.
- **The command line** (clap): `--help`, `--version`, a usage text per command. Misuse (an unknown command or option, a value
  that does not parse) exits with 2 before the game is opened; a batch in which an area failed exits with 3. The user folder is
  `--user`, else `NWN_BAKE_USER`, else `NWN_HOME`, in every command; `serve` starts without one. `area-iso`, `area-3d` and
  `area-scene` take areas the same way (a lone area file, `--all`, `--target`).
- **The game**: one way to open it (`nwn_bake::game::open`) and to resolve a texture name (`nwn_bake::texture::resolve`: an EE
  material names another image). A `--root` that is not an install says so; an install with only another language (`lang/fr`,
  `lang/de`) opens in it; haks are found whatever the case of `.hak`; a user folder without `hak/` has no haks. The custom talk
  table is the one asked for: one not found, or a file that is not a talk table, is an error; a name without `.tlk` (as a
  module's `Mod_CustomTlk` gives it) is found. Errors name the file they could not read.
- **The library prints nothing:** warnings and notes come back to the caller. A program that keeps nwn-bake loaded (`serve`, or
  a server that links the library) goes on after a failed request: locks a panic left poisoned are used as they are.
- **Release notices**: `THIRD-PARTY-LICENSES.html` has the licences of the C and C++ libraries in the binaries (libwebp with its
  patent grant, glslang, zstd) and leaves out crates only other platforms use.
- **A blueprint's equipped items** (#122, #123): `thumb`, `creature`, `describe`, `look` and `serve` look up the items a `.utc`
  names by resref, which they used to leave off without a word: in a folder of the module's item blueprints first (`--items`,
  `items` in `serve`; binary `.uti` or nwn_gff's `.uti.json`), then the user's override and haks, then the game. An item found
  nowhere is named with its slot: a warning on stderr, `missing_items` in `serve`'s answers and in the look; `--strict-items`
  (`strict_items`) makes it an error. Library: `nwn_bake::items`; `creature::thumbnail`, `bake_creature`, `look::describe` and
  `look::ensure` take the item lookup, and `thumbnail` returns `creature::Thumbnail`. These commands and `item-thumb` read
  nwn_gff's JSON blueprints too.
- **`keep_tags` has no default** (#121): a draft without `keep_tags` keeps no tag from `drop_objects`. It used to keep
  `hsng_entry` and `ldh_doorbell*`; the caller now names the tags to keep.
- **Scenes** (`scene`, #124): placeables, models, creatures and items in one picture, each with a position, facing, scale and
  texture replacements, at a free camera (a position, or a target with yaw, pitch and distance; perspective or orthographic;
  framed to what is drawn when left out). Lines, axis markers, points projected to pixels, the light (`day` or an area's sun
  colours), the sun's shadows and the background (transparent or a colour). A creature wears its blueprint's items through the
  item lookup, and the items not found are named as for `thumb`. A spec file (`docs/scene.md`, `docs/scene.schema.json`), and
  `serve`'s `scene` request. Library: `nwn_bake::scene`.
- Built on Moonglow 4.0.0 (`ac17ade`).
