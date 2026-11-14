# `nwn-bake serve`: the protocol

`nwn-bake serve` is a baker that stays running: the game (and, for characters and pictures, a GPU device) is opened once, and
requests are answered one after another. A program that asks for many pictures (a website's backend, a batch script) starts it once
and keeps it.

```text
nwn-bake serve [--root <game install>] [--user <user folder>] [--tlk <name or path>] [--module <module>] [--cache-mb <MiB>]
```

- `--root` is the NWN:EE install (the folder with `data/` and `lang/`); without it the install is found as the game finds it.
- `--user` is the user folder: its `hak/`, `tlk/` and `override/` are laid over the install. Without it, `NWN_BAKE_USER`, then
  `NWN_HOME`, then the game's own default, the same rule as every other command. Characters, items and placeables are read with
  every `.hak` in `hak/`, by name. With `--module` (anything `area-iso` takes), that module's hak list is used instead, in its
  order (the first listed wins, as in the game).
- Without any user folder, `serve` still starts: the area requests work (each finds its module's haks in the install's
  `data/hk/`), and the character requests answer with an error that says why the game could not be opened for them. A user
  folder that is named but not there is an error at the start.
- `--tlk` (or `NWN_BAKE_TLK`) names the custom talk table: a file name in the user folder's `tlk/` or `hak/` (`.tlk` may be left
  out) or a path. Not finding it, or a file that is not a talk table, is an error. By default the module's (none when it is not
  there), else the one `.tlk` in the user folder's `tlk/` or `hak/`.
- `--cache-mb` (or `NWN_BAKE_CACHE_MB`) bounds, in MiB, what the model and texture caches keep between requests: a quarter for
  models, the rest for textures, the entries used least recently dropped first. The default is 4096; `0` keeps as little as can be.
  It is one budget for the whole process: the game the character requests use and every game the area requests open (below) share
  it, so it bounds them together, however many hak lists are asked for. It never changes an answer, a picture or a key. What a
  request is drawing is not counted. See the README's "Memory".

## Lines

- When it is ready, `serve` writes `{"ready": true}`.
- Then each line on stdin is one request, a JSON object, and gets exactly one line on stdout, its answer, in the same order. Empty
  lines are skipped.
- A request that fails answers `{"error": "..."}`, and `serve` goes on with the next one.
- It ends when stdin ends.

Paths in requests are the server's own (absolute paths are safest). Files are written whole: to a temporary name, then renamed.

## Character requests (need a GPU or Mesa lavapipe)

Without a GPU or a software adapter, these answer with an error; the placeable and area requests below draw nothing.

**A look:** what a character is made of, and every file a viewer needs to show it that the folder does not have yet.

```json
{"in": "/path/to/character.bic", "dir": "/path/to/parts", "anim": "pause1", "t": 0}
```

- `in`: a `.bic` or `.utc`. `dir`: the parts folder, shared by every character: `chunk/<id>` (a part's meshes, see
  `nwn_bake::look`), `tex/<name>.png` (plain textures), `plt/<name>.png` (PLT textures), `palette/<name>.png`. A file there is never
  changed: it is named for what is in it.
- `anim` (default `pause1`) and `t` (seconds, default 0): the pose.
- `items` (a folder) and `strict_items` (default false): where the items a `.utc` names by resref are looked up first, as the
  `--items` and `--strict-items` options do (see below).
- The answer is the look (`nwn_bake::look::Look`): `anim`, `t`, `base` and `parts`, each part with its `id`, model, attachment,
  scale, textures, colours and whether it is animated, and `missing_items`.

**Equipped items.** A `.bic` carries the items it wears; a `.utc` names them by resref. Each is looked up in the request's
`items` folder first (`<resref>.uti`, or nwn_gff's `<resref>.uti.json`, in it or its subfolders), then in the user folder's
override and haks, then in the game. The folder is read again for every request, so a new item file is seen at once. An item
found nowhere is left off the creature and listed in the answer's `missing_items`, one object for each, in the order of the
blueprint's `Equip_ItemList`: `{"slot": "lefthand", "resref": "nw_ashsw001"}`, with `"error"` too when a file was found that does
not read. The slot is named as the game's `INVENTORY_SLOT_*` constants name it, lower case (`head`, `chest`, `righthand`...).
Every slot is checked, those a picture does not show (rings, boots) too. With `"strict_items": true`, a missing item fails the
request instead (`{"error": "equipped items not found: nw_ashsw001 (lefthand)"}`). `in` may be nwn_gff's JSON (`.utc.json`).

**A picture of a creature:**

```json
{"in": "/path/to/creature.utc", "thumb": "/path/to/out.png", "size": 256, "anim": "pause1", "t": 0, "yaw": 60}
```

Answers `{"ok": true, "missing": [models not found], "missing_items": [...]}`; a model the game has that does not read is listed
as `name: why`, and `missing_items` is as for a look. `items` and `strict_items` are as for a look. `size` is the side in pixels
(default 256), `yaw` the turn in degrees (default 60).

**A picture of an item on its own:** as a creature's, with `"item": true` and `in` a `.uti`. `size` is the width, `height` the
height (default `size`), `yaw` default 0.

**A picture of a placeable:**

```json
{"placeable": 42, "thumb": "/path/to/out.png", "size": 256, "height": 256, "yaw": 0, "textures": [["old", "new"]]}
```

`placeable` is a row of `placeables.2da`; `textures` replaces textures by name, as `ReplaceObjectTexture` does. Answers
`{"ok": true, "missing": [...], "particles": true|false}`.

**Several objects in one picture:**

```json
{"scene": {"objects": [{"placeable": 42}, {"creature": "/path/to/a.utc", "position": [2, 0, 0]}]}, "out": "/path/to/out.png"}
```

`scene` is a scene's spec (`scene.md`), `out` the PNG to write; `items` and `strict_items` are as for a look. Answers what the
`scene` command prints (the camera, the box round what is drawn, the points' pixel positions, and each object's box, missing
models and `missing_items`), with `"ok": true`.

## Placeable requests (no GPU needed)

These read the game's data and draw nothing, so they work on a machine with no GPU and no software adapter.

**A placeable's textures:** `{"textures_of": 42}` answers `{"model": "...", "textures": [...]}` (`nwn_bake::model_textures`: each
texture's name, the image it means, its kind and how many triangles use it).

**Every placeable:** `{"placeables_index": "/path/to/placeables.json"}` writes every `placeables.2da` row with a model, described
(`nwn_bake::placeables_index`), to that file and answers `{"ok": true, "rows": n}`.

## Area requests (no GPU needed)

```json
{"area_iso": {"module": "/data/mymodule", "area": "village", "views": ["sw", "se", "ne", "nw", "top"], "out": "/data/area-iso",
              "cache": "/data/area-iso-cache"}}
{"area_iso_key": {"module": "/data/mymodule", "area": "village", "views": ["sw"], "out": "/data/area-iso", "cache": "/data/area-iso-cache"}}
```

Instead of `module` (anything `area-iso` takes) and `area`, a request may give the area's files: `are` and `git` (paths, binary
or JSON; `git` defaults to the one beside the `.are`), with `haks` and `tlk` (else those of the nearest `module.ifo` above the area).
The options are `area-iso`'s, by the same names (but `quality`, the supersampling, which is `--supersample` there) and with the same
defaults: `views` (a list, or `"all,top"`), `scale`, `quality`, `light`, `shadows` (true or false), `roof`, `exposure`, `objects`,
`background`, `depth`, `format`, `look` (the constants that differ, as an object), `region` (`[x0, y0, x1, y1]`), `marks` (a list
of strings as `--mark` takes them, `"41.2,17.5,#ff8000"`), `layout` (`tree`, the default, or `flat`), `force` and `strict`. Unknown fields are an error.

- `area_iso` checks, and draws when it must, as a batch does, the views each on its own thread; it answers the area's manifest
  (`area-iso.md`), with `action` (`current`, `cached` or `rendered`) and `manifest_path` added. It does not touch `manifest.jsonl`:
  a line holding the manifest (as answered) can be appended to it, and a reader of `manifest.jsonl` takes such a line as one line
  per view.
- `area_iso_key` answers `{"area", "quick_key", "key", "resources", "cached", "current"}`: whether the cache, and the output
  folder when `out` is given, have pictures of that exact key. It builds the scene but draws nothing. A program can use it after a
  module or hak change to find, cheaply, which areas need drawing again.

For areas, the game is opened once for each hak list and talk table (at most four kept), all of them within the one cache budget
(`--cache-mb`), and the haks' digests are kept; the module
is read again for each request, so a change to its files is seen at once (a nasher project is listed, well under a second for one of
1,300 areas, and only the area's two files are read).

## Versions

The answers are the documented formats (the look, the manifest), and fields may be added to them; readers ignore fields they do not
know. A request field that is not known is an error, and so is a request with none of `in`, `textures_of`, `placeables_index`,
`placeable` (with `thumb`), `scene`, `area_iso` or `area_iso_key`.
