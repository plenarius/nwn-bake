# Code and documentation standard

One standard for every repository that follows it: [nwn-bake](https://github.com/plenarius/nwn-bake) and the projects built on
it, an area editor's Rust engine ([github.com/plenarius/landscaper](https://github.com/plenarius/landscaper)) and its web page
([github.com/plenarius/landscaper-web](https://github.com/plenarius/landscaper-web)). Each repository keeps an identical copy of
this file at its root, and of the shared configuration files listed under [The checks](#the-checks).

**nwn-bake's copy is the master.** Change it there first, then copy it word for word to every other repository that follows this
standard, in one pull request each, on the same day, so they never drift. The other copies follow it and are not edited on their
own.

`scripts/check.sh` runs every check below, the way CI runs them. Run it before you push.

## Writing

This applies to Markdown, comments, doc comments, commit messages and pull requests.

- Plain, direct language. Short sentences. Active voice: "the baker writes a manifest", not "a manifest is written".
<!-- vale Plain.Filler = NO -->
- No filler and no selling: *seamlessly*, *robust*, *leverage*, *simply*, *just*, *very*, *really*, *easily*, *powerful*. Say what
  happens instead.
<!-- vale Plain.Filler = YES -->
- No em dashes. Use a colon, a semicolon, commas or a new sentence.
- Numbers over adjectives: "98 ms against 29 ms", not "much faster"; "1,330 areas", not "many areas".
- One term for one thing, everywhere: *tileset*, *walkmesh*, *hak*, *talk table*, *resref*. The game's own names win over ours.
- Say why, not only what: the reason, the measurement, or the source (a section of a design document, a function of the game).
- British or Canadian spelling in prose (colour, licence as a noun, optimise). Code follows the API it calls (three.js `color`).
- Contractions are fine ("doesn't").

Vale enforces the word lists (`.vale/styles/Plain/`). `vale --minAlertLevel=suggestion <file>` also shows vague amounts, the
passive voice and sentences over 35 words; those are suggestions, not failures.

### Markdown

- Wrap lines at 140 columns, at a word. Tables, code blocks and long links may run longer.
- `#` for the title, then `##` and `###`. One title per file.
- Lists use `-`. Emphasis is `*word*`, bold is `**word**`. Bold the term a list item defines, at its start.
- Code, file names, commands, options and identifiers go in backticks. Code blocks are fenced with ``` and name their language.
- Link to our own files with relative paths, and to a heading with `#anchor`; lychee checks both.
- A table only for things that line up in columns. Otherwise a list.

### READMEs

Every README follows the same skeleton. Leave out a section that has nothing to say; keep the order.

1. `# Name`, then one paragraph: what it is and who it is for. Then a list of what it does, one line each.
2. `## You need`: what must be installed or owned first (the game, a GPU, a Rust version).
3. `## Install` (or `## Run` for a page or a server): the shortest path from nothing to a first result.
4. `## Use`: one `##` section per command or tool, with an example and its options.
5. `## Documents`: the files under `docs/`, one line each.
6. `## Development`: build, test, `scripts/check.sh`, and a pointer to this file.
7. `## Credits`: the projects it is built on or includes, with their licences.
8. `## Licence`: the copyright line, the licence, and anything that differs for the binaries.

## Commits and pull requests

- **Commit messages:** a subject line under 72 characters that says what changed, in the present tense, with the area first
  when it helps (`nwn-bake: area pictures in batches`). A body when the why is not obvious.
- **Pull requests** say what changed and why, then how it was checked: which tests ran, what was measured, on what. Say what was
  **not** checked, too. Link the issue (`Fixes #12`).
- No attribution lines: no `Co-Authored-By`, no "Generated with".
- One topic per pull request. Formatting-only changes go in their own commit, so a reviewer can skip them.

## Rust

### Formatting

`rustfmt.toml` is the same in every workspace:

```toml
max_width = 140
use_small_heuristics = "Max"
```

The code is written wide (a drawing function reads better on one line than on six), and `use_small_heuristics = "Max"` lets
short expressions use the whole width. Stable rustfmt options only, so `cargo fmt` needs no nightly toolchain. rustfmt does not
wrap comments on stable: wrap them by hand at about 135 columns.

`use` lines come in three groups separated by a blank line: `std`, other crates, then `crate`/`super`.

### Lints

Each workspace's `Cargo.toml` has the same `[workspace.lints]` block, and every crate in it has `[lints] workspace = true`. CI
runs `cargo clippy --workspace --all-targets --locked -- -D warnings`, so a warning fails the build. On top of clippy's default
set:

| Lint | Level | Why |
| --- | --- | --- |
| `unsafe_code` | forbid | There is no `unsafe` code. See [unsafe](#unsafe). |
| `rust_2018_idioms` | warn | Elided lifetimes in paths (`Formatter<'_>`) and other old idioms. |
| `unused_qualifications` | warn | `std::path::Path` where `Path` is imported. |
| `clippy::dbg_macro`, `todo`, `unimplemented` | warn | Never in a commit. |
| `clippy::allow_attributes_without_reason` | warn | Every `#[allow]` or `#[expect]` says why: `reason = "..."`. |
| `clippy::mod_module_files` | warn | Modules are `foo.rs` plus `foo/`, never `foo/mod.rs`. |
| `clippy::too_many_arguments` | allow | Drawing and baking functions take their parts as arguments. |
| `clippy::type_complexity` | allow | Some caches have long types; an alias would hide them. |
| `clippy::case_sensitive_file_extension_comparisons` | warn | Game files are often upper case (`.MDL`, `.TGA`). |
| `clippy::cast_lossless` | warn | `f32::from(x)` over `x as f32` where no value is lost, so the casts that remain are the lossy ones. |
| `clippy::cloned_instead_of_copied`, `implicit_clone` | warn | Say what is copied, and how. |
| `clippy::explicit_iter_loop`, `redundant_closure_for_method_calls` | warn | `for x in &v`, `.map(ToString::to_string)`. |
| `clippy::ignored_unit_patterns`, `semicolon_if_nothing_returned` | warn | `Ok(())` over `Ok(_)`; a `;` after a unit expression. |
| `clippy::manual_let_else`, `map_unwrap_or` | warn | `let Some(x) = ... else { ... };` and `map_or` over longer forms. |
| `clippy::stable_sort_primitive`, `uninlined_format_args` | warn | `sort_unstable` for numbers; `"{name}"` over `"{}", name`. |
| `missing_docs` | deny | nwn-bake only: a library others build on documents every public item. |

The block is the same in every workspace, apart from `missing_docs`, which only nwn-bake has: a public item without a doc
comment fails its build. Prefer `#[expect(lint, reason = "...")]` to `#[allow]`: it warns when it is no longer needed.

The rest of clippy's pedantic group is not enforced. Its cast lints (`cast_possible_truncation`, `cast_precision_loss`) would flag
the drawing code's casts, which are on purpose, and `doc_markdown`, `must_use_candidate`, `similar_names` and `too_many_lines`
cost more than they catch here. `scripts/check.sh --pedantic` counts the whole group's hits by lint; a pedantic lint joins the
shared set in all workspaces at once, with its hits fixed in the same pull request.

### Naming

- The Rust API guidelines: `snake_case` for functions, variables and modules; `CamelCase` for types and traits;
  `SCREAMING_SNAKE_CASE` for constants.
- Name a thing for what it is in the game when it has a game name: `resref`, `tileset`, `walkmesh`, `hak`, `tlk`.
- Short names (`i`, `x`, `n`, `k`) in loops and formulas only, where the formula is the documentation. Say what they are in a
  comment when it is not obvious.
- A value in a unit other than metres, seconds or degrees says its unit in the name (`size_px`, `timeout_ms`).

### Errors

- `anyhow` everywhere, in the libraries too: functions that can fail return `anyhow::Result<T>`.
- Add what was being done with `.context("...")` or `.with_context(|| format!(...))`, naming the file, resource or area.
- `bail!` and `ensure!` for checks. `ensure!(cond, "...")` over `if !cond { bail!(...) }`.
- Messages are lower case, with no full stop, and name the thing first: `"tileset {resref}: not found"`.
- A bad input file is an error, never a panic: one broken model must not stop a batch of 1,330 areas.
- `unwrap()` in tests, and where the invariant is on the line above. Elsewhere, `expect("why this cannot fail")`.
- `thiserror` is not used. Add it only when a caller has to tell errors apart (match on a kind), and then only in a library.

### Comments and doc comments

- Every file starts with a `//!` comment: what the module is for and where its rules come from.
- Every public item has a `///` comment. Start with what it is or does ("The tileset `resref`, as the game reads it."), then the
  details a caller needs: units, ranges, what an empty or missing value means.
- Link items with ``[`Item`]``. `cargo doc` runs with `-D warnings`, so a broken link fails the check.
- Comments say why: the reason, the measurement, the game's behaviour, the source. Not what the next line does.
- No commented-out code; git keeps it. A `TODO` names an issue: `// TODO(#12): ...`.

### unsafe

There is none, and `unsafe_code = "forbid"` keeps it that way. If a crate ever needs it: one small module, a `// SAFETY:` comment
on every block saying why it holds, the lint relaxed for that module only with a `reason`, and a test that exercises it.

### Tests

- Unit tests sit at the bottom of their file in `#[cfg(test)] mod tests`, or in `<module>/tests.rs` when they are long.
- Integration tests go in `tests/`, one file per feature, with shared helpers in `tests/support/`.
- Tests need no game install: they write the small made-up game files they read. Tests on a real install run only when an
  environment variable names one, and CI never sets it.
- A test's name says the behaviour it checks: `moonglow_is_pinned_alike`, not `test_pins`.
- A golden file says, next to it or in the test, how to update it and when that is right.

### Module layout and dependencies

- `foo.rs` declares `mod bar;` and the code is in `foo/bar.rs`. No `mod.rs` files.
- One idea per module. Past about 1,500 lines, look for a second idea to move out.
- A dependency used by more than one crate is declared once, in `[workspace.dependencies]`.
- A git dependency is pinned to a whole commit hash (`rev = "..."`), never a branch.
- cargo-machete keeps the list honest: no dependency that the code does not use.

## JavaScript, CSS and HTML

### Formatting and lint

Biome formats and lints JavaScript, CSS and JSON (`biome.json`): 4 spaces, 140 columns, single quotes, semicolons, trailing
commas, `x => ...` without parentheses for one argument. The linter uses Biome's recommended rules with one turned off:
`noDescendingSpecificity` (CSS), which asks for rules ordered by specificity; a stylesheet here is ordered by part of the page.
`vendor/` is never formatted or linted.

html-validate checks the HTML with its recommended rules (`.htmlvalidate.json`).

### Modules

- Native ES modules, loaded by the browser as they are: no bundler and no build step.
- Imports are relative and name the file with its extension (`'./area.js'`). Vendored packages come through the import map
  in `index.html` (`'three'`).
- Named exports only, no `export default`.
- One file per feature (`tools.js`, `things.js`). It starts with a comment saying what the file does.

### Naming

- `camelCase` for functions, variables and DOM ids (`btnNew`); `PascalCase` for classes; `UPPER_SNAKE_CASE` for module-level
  constants (`MAX_RISE`).
- CSS classes and custom properties in `kebab-case` (`.menu-item`, `--panel-2`). Colours are custom properties on `:root`.
- File names are lower case, one word where it fits.

### Comments

The rules for Rust comments apply. A `/** ... */` comment on each exported function and class says what it is and what its
arguments mean, with units. JSDoc type tags only where a type is not obvious from the code.

### Vendored code

Each folder under `vendor/` holds an unmodified copy of one upstream package, its `LICENSE`, an entry in `REUSE.toml`, and a
line in the README that names the package, its version and its licence. To update one, replace the folder and that line in the
same commit. `scripts/check.sh` checks that all three are there.

## Shaders

The game's shaders (`.shd`, GLSL) and shader code in JavaScript strings follow the Rust rules for naming and comments:

- A shader based on one of the game's stock shaders is not kept as a file. The program reads the stock shader (`fslit`) from the
  player's install and makes its change at run time, so no copy of Beamdog's shader is in a repository or a release; the code that
  changes it says what it changes and why. The area editor's engine does this for its shadow shaders.
- A shader written from scratch is ours and starts with the two SPDX lines, then a comment saying what it draws.
- A shader file's name is a resref: lower case, at most 16 characters before the extension, because the game looks it up by
  that name.
- 4 spaces, lines under 140 columns. The game's own macro names (`SHADER_TYPE`, `FOG`) are kept as the game spells them.

No tool formats these files; typos checks their spelling. WGSL, if a repository adds it, follows the same rules.

## Licences and REUSE

Every file says who owns it and under which licence, the [REUSE](https://reuse.software/) way:

- Source code (`.rs`, `.js`, `.css`, `.html`, `.sh`, `.py`, `.java`, and `.shd` written from scratch) starts with two lines in
  its own comment style:

  <!-- REUSE-IgnoreStart -->
  ```rust
  // SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
  // SPDX-License-Identifier: GPL-3.0-or-later
  ```
  <!-- REUSE-IgnoreEnd -->

  A file based on someone else's work keeps their copyright line above ours.
- Everything else (Markdown, configuration, JSON, images, lock files) is covered by `REUSE.toml`, so those files stay clean.
- Vendored code keeps its own licence, recorded in `REUSE.toml`.
- Measured game data (see [Never in these repositories](#never-in-these-repositories)) is Beamdog's: `REUSE.toml` gives it
  `SPDX-FileCopyrightText = "Beamdog"` and `LicenseRef-Beamdog`, a text in `LICENSES/` saying that the repository grants no
  licence for it.
- The licence texts are in `LICENSES/`, one file per SPDX identifier. `LICENSE` at the top stays, for GitHub and for people.

The licences:

- **nwn-bake:** GPL-3.0-or-later.
- **The area editor's engine** ([github.com/plenarius/landscaper](https://github.com/plenarius/landscaper)):
  GPL-3.0-or-later, like nwn-bake. Both build on Moonglow, which is GPL-3.0, so their binaries are GPL-3.0 in any case.
- **The area editor's web page** ([github.com/plenarius/landscaper-web](https://github.com/plenarius/landscaper-web)): MIT. It
  started from ergin3d's MIT-licensed code, and its `LICENSE` keeps ergin3d's copyright line above ours.

Each repository's licence is also in its `REUSE.toml`, `Cargo.toml` (Rust) and README. `cargo-deny` and `cargo-about` check that
every Rust dependency's licence is on the list in `deny.toml` and `about.toml`.

### Never in these repositories

Decompiled game code (a decompiler's output from the game's binaries, cleaned up or not), the game's own files or extracts of
them (models, textures, 2da files, talk tables, scripts, shaders), and anything else taken from the game. None of it is ours to
license. The one exception for data is layout data measured from how the game behaves, when a program can't work without it and
the game has no file to read it from: the area editor's path node layouts, measured from the server and marked as Beamdog's in
that repository's `REUSE.toml`.

The one exception for code is the area editor's route simulator (`src/sim.rs`, `src/sim/` in
[github.com/plenarius/landscaper](https://github.com/plenarius/landscaper)): our own reimplementation of how the NWN:EE game
server plans a creature's walk inside an area, kept so that the areas we export walk the way the server routes them
(interoperability with the game). It holds no decompiler output, binary addresses, the engine's internal names or its data
layouts; the code and its comments are ours and describe what it does in our own words. It reproduces the server's behaviour,
down to the order of its arithmetic, its tie-breaks and its limits, because a route depends on them, and it is checked against
routes recorded on a real server (the editor's `oracle` and `sim-check` commands).

Tests write the small made-up game files they need; notes on how the game behaves are written in our own words and name their
source (a behaviour of the game, a measurement).

## The checks

`scripts/check.sh` runs all of these and exits with 1 if one fails. `--online` adds the web links; `--pedantic` adds clippy's
pedantic report.

| Tool | Version | Checks | Config |
| --- | --- | --- | --- |
| rustfmt, clippy, rustdoc | stable Rust | formatting, the shared lints, doc links (`-D warnings`) | `rustfmt.toml`, `Cargo.toml` |
| [cargo-machete](https://github.com/bnjbvr/cargo-machete) | 0.9.2 | unused dependencies | |
| [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) | 0.20.2 | licences, security advisories, duplicate crates, sources | `deny.toml` |
| [cargo-about](https://github.com/EmbarkStudios/cargo-about) | 0.9.3 | every dependency's licence can be shipped | `about.toml` |
| [Biome](https://biomejs.dev/) | 2.6.6 | JavaScript, CSS and JSON: formatting and lint | `biome.json` |
| [html-validate](https://html-validate.org/) | 11.21.0 | HTML | `.htmlvalidate.json` |
| [typos](https://github.com/crate-ci/typos) | 1.52.0 | spelling in code and text | `_typos.toml` |
| [Vale](https://vale.sh/) | 3.26.0 | the writing rules above | `.vale.ini`, `.vale/styles/Plain/` |
| [markdownlint-cli2](https://github.com/DavidAnson/markdownlint-cli2) | 0.24.0 | Markdown | `.markdownlint.jsonc` |
| [lychee](https://github.com/lycheeverse/lychee) | 0.24.2 | dead links and anchors | `lychee.toml` |
| [REUSE](https://reuse.software/) | 6.3.0 | a copyright and a licence for every file | `REUSE.toml`, `LICENSES/` |
| [gitleaks](https://github.com/gitleaks/gitleaks) | 8.30.1 | secrets in the tree and in every commit | |
| [trufflehog](https://github.com/trufflesecurity/trufflehog) | 3.99.8 | secrets, verified against their services | |

Shared files, identical in every repository: `STYLE.md`, `scripts/check.sh`, `.editorconfig`, `_typos.toml`, `.vale.ini`,
`.vale/styles/Plain/`, `.markdownlint.jsonc`, `lychee.toml`, and in Rust workspaces `rustfmt.toml`, `deny.toml`, `about.toml`
and the `[workspace.lints]` block (apart from `missing_docs`). Each repository has its own `REUSE.toml`, and `biome.json` and
`.htmlvalidate.json` where there is a web page.

Install the tools as release binaries checked against their published checksums, into `~/.local/bin`: cargo-deny, cargo-about,
cargo-machete, typos, Vale, lychee, gitleaks and trufflehog from their GitHub releases, and Biome's standalone binary
(`biome-linux-x64-musl`). REUSE with `uv tool install reuse==6.3.0` (or pipx). html-validate and markdownlint-cli2 with
`npm install -g --prefix ~/.local html-validate@11.21.0 markdownlint-cli2@0.24.0`. Raise a version in this table and in
`scripts/check.sh` together.
