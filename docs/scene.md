# Scenes: several objects in one picture

`nwn-bake scene <spec.json> <out.png>` draws placeables, models, creatures and items together, each placed, turned and scaled, at
one camera: two models side by side at the same scale, a chess board, a lineup of creatures. The other pictures (`thumb`,
`placeable-thumb`, `item-thumb`) are one object each, framed by itself at a fixed tilt.

```text
nwn-bake scene <spec.json|-> <out.png> [--items <dir>] [--strict-items] [--module <module>] [--root <install>] [--user <folder>] [--tlk <name>]
```

The spec is JSON, read from the file or from stdin (`-`). Its schema is `scene.schema.json`. `serve` takes the same spec as a
request (below). The picture is drawn with Moonglow's renderer, as the other pictures are, so it needs a GPU or Mesa lavapipe.

## The spec

```json
{
  "width": 1200, "height": 800,
  "objects": [
    {"placeable": 112, "position": [0, 0, 0]},
    {"creature": "blueprints/nw_goblina.utc", "position": [-1.5, 0, 0], "facing": 270},
    {"item": "blueprints/nw_wswls001.uti", "position": [1.5, 0, 1], "scale": 2},
    {"model": "plc_a01", "position": [3, 0, 0], "textures": [["plc_a01", "plc_a02"]]}
  ],
  "camera": {"yaw": -90, "pitch": 30},
  "lines": [{"from": [0, 0, 0], "to": [8, 0, 0], "colour": "#ffcc00"}],
  "axes": [{"at": [0, 0, 0], "length": 1}],
  "points": [[0, 0, 0], [8, 8, 0]],
  "light": "day",
  "shadows": false,
  "background": "#1a1c24"
}
```

Coordinates are the game's: metres, x east, y north, z up. Paths are read as given (relative to the current folder). Everything
but one object, line or axis marker may be left out. Unknown fields are an error, and so is a spec that does not check: it is
checked before the game is opened.

- `width`, `height`: the picture, 1 to 8,192 pixels a side (1024 by 768).
- `objects`: each one of
  - `placeable`: a row of `placeables.2da`, drawn as a placed placeable shows it (its idle animation and its particles);
  - `model`: a model by resref, on its own;
  - `creature`: a creature blueprint (`.utc`, or nwn_gff's `.utc.json`) or a character (`.bic`), dressed in what it wears;
  - `item`: an item blueprint (`.uti` or `.uti.json`), as `item-thumb` draws it.

  And for each:
  - `position`: its model's origin (`[0, 0, 0]`);
  - `facing`: degrees counter-clockwise from east, as the game's `GetFacing` (270: south, towards the default camera). The game's
    models face north unturned, so a model is turned by its facing less 90. A placeable of a `.git` with `Bearing` b (radians)
    faces b in degrees plus 90;
  - `scale`: its size, 1 as the game draws it;
  - `textures`: `[old, new]` texture names, as `ReplaceObjectTexture` replaces them: on a creature's body, not on what it wears;
  - `anim`, `t`: the animation and the seconds into it. A creature or item stands in `pause1`, a placeable or model in its idle
    state (else `default`).
- `camera`: where the picture is taken from. Every field may be left out:
  - `target`: the middle of the picture (the middle of the box round everything drawn);
  - `position`: where the camera is. With `target`, the camera looks from here at the target. Without, it looks from here along
    `yaw` and `pitch`, at a target `distance` (10) metres away;
  - `yaw`: the camera's side of the target, degrees counter-clockwise from east (-90: from the south, looking north);
  - `pitch`: the camera's height above the horizon, -89.9 to 89.9 (30, as `area-iso`'s corner views). 89.9 looks straight
    down; with a yaw of -90, north is up;
  - `distance`: metres from the target (as near as shows everything drawn, with 2.5% to spare a side);
  - `fov`: the field of view, top to bottom, in degrees (40, as the other pictures);
  - `orthographic`: true for a picture in which a thing is the same size near and far;
  - `scale`: in an orthographic picture, pixels a metre (as large as shows everything drawn).
- `lines`: straight lines a pixel wide, `from`, `to` and `colour` (`#rrggbb` or `#rrggbbaa`, white when left out). An object in
  front of a line hides it.
- `axes`: axis markers: lines from `at`, `length` metres (1) east in red, north in green and up in blue.
- `points`: points to find in the picture. Nothing is drawn for them; the answer gives their pixel positions.
- `light`: `"day"`, the light of the other pictures (the default), or the sun as an area has it:
  `{"ambient": "#rrggbb", "diffuse": "#rrggbb", "toward": [x, y, z]}`, the colours as the `.are` gives them (`SunAmbientColor`,
  `SunDiffuseColor`) and the direction towards the sun (the game's default sun's when left out).
- `shadows`: the sun's shadows, cast by every object onto the placeables and models (a board or a floor under the others), at
  the Shadow Opacity of 60 most of the game's areas have. Off by default.
- `background`: `"transparent"` or `"#rrggbb"`, as `area-iso`'s `--background` (`#1a1c24`, the other pictures' dark grey).

The framing takes in the objects as they stand (each one's box in its animation), the lines and the axis markers, but not the
points.

### Orthographic pictures

Moonglow's renderer draws in perspective only. An orthographic picture is taken through a long lens: the camera stands 2,000 m
plus 200 times the scene's radius from the target, with a field of view narrow enough for the picture's height in metres. A thing
at the far side of the scene is drawn under 1% smaller than at the near side. With `position`, only its direction from the target
counts.

### Transparent pictures

The renderer clears to a colour. A transparent picture is drawn twice, on black and on white, and each pixel's alpha is how little
of the white shows through; its colour is the one on black over its alpha. It takes twice as long.

## Equipped items

A creature blueprint names the items it wears by resref. They are looked up as for `thumb`: in the folder `--items` names first
(`<resref>.uti` or `<resref>.uti.json`, subfolders too), then the user folder's override and haks, then the game. An item found
nowhere is left off the creature, with a warning on stderr that names the object, the item and its slot, and in the answer's
`missing_items`. With `--strict-items`, it fails the picture instead.

## The answer

`scene` prints one line of JSON on stdout:

```json
{
  "width": 1200, "height": 800,
  "camera": {"eye": [0.0, -14.2, 9.1], "target": [0.0, 0.0, 0.9], "fov": 40.0, "orthographic": false, "scale": 58.3},
  "bounds": [[-2.1, -0.6, 0.0], [8.0, 0.7, 2.3]],
  "points": [[600.0, 455.2], [1012.7, 301.4]],
  "objects": [
    {"kind": "placeable", "bounds": [[-0.5, -0.5, 0.0], [0.5, 0.5, 1.1]], "missing_models": [], "missing_items": []}
  ]
}
```

- `camera`: where the camera was (`eye`), what it looked at (`target`), its field of view in degrees (the long lens's, when
  orthographic) and the pixels a metre at the target (everywhere, when orthographic).
- `bounds`: the box round everything drawn, lowest corner then highest.
- `points`: each point's pixel position from the picture's top-left corner, in the spec's order; `null` for a point behind the
  camera.
- `objects`: one for each object, in the spec's order: its kind, its box, the models it names that the game does not have or
  cannot read (`name`, or `name: why`), and the equipped items not found (`{"slot", "resref"}`, as `serve` gives them).

## In `serve`

```json
{"scene": {"objects": [{"placeable": 112}]}, "out": "/path/to/out.png", "items": "/path/to/uti", "strict_items": false}
```

`scene` is the spec, `out` the PNG to write, `items` and `strict_items` as for a creature's picture. The answer is the one above,
with `"ok": true`.
