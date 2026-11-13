# DraftCraft — instructions for agents

DraftCraft is an open-source, pure-Rust 2D CAD and drafting application with a Windows-style ribbon
interface on macOS, Windows and Linux, plus a browser build (WASM). It reads and writes DWG and DXF, uses
AutoCAD's command names at its command line, and can be driven by agents (MCP, control channel, CLI).
Its upstream's copyright notice (MIT OR Apache-2.0) stays in `LICENSE-MIT` and `NOTICE`: the licence
requires it. The DraftCraft™ name and logo are WithShalom Studios' marks, used by agreement
(`docs/brand/LICENSE-brand.txt`).

## Never crash
People trust DraftCraft with their drawings; a crash loses their work. **This outranks feature work.**
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`).
- **Errors are `Result<T, E>`** through the crate's error type and `?`. An unfinished feature returns an error or reports "not available yet"; it never panics.
- **Input-derived numbers are hostile** (DXF/DWG files, command-line text, MCP/control params): `get()` not `[i]`, checked arithmetic, no NaN casts, cap input-sized allocations and loop counts.
- **Bound recursion** (nested/cyclic block references: `MAX_BLOCK_DEPTH`).
- **Last-resort guard:** `Session::execute` and every step of an interactive command (`crates/engine/src/guard.rs`) run under `catch_unwind`; an escaped panic restores the drawing and reports an error. It does nothing on wasm, where a panic aborts.
- **Prove it:** every crash fix lands with a regression test (see `crates/engine/tests/panic_hunt.rs` and `hostile_dxf_does_not_panic` in `crates/io/src/tests.rs`).
- Every workspace crate manifest carries `[lints] workspace = true`; every production crate root carries `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`.

## Non-negotiables
- **Clean-room.** Take no proprietary code or information directly from the application software. AutoCAD may only be *observed* black-box (run it, look at its UI with your own drawings). Never read, disassemble or copy anything inside an Autodesk install; never copy Autodesk fonts (SHX), hatch patterns (`.pat`), linetypes (`.lin`), templates, CUI/PGP files, icons, artwork or help text; never commit screenshots of AutoCAD. DWG and DXF files are a different matter, whichever program saved them, AutoCAD included: they are the files people exchange, and reading them, testing against them and keeping them in the repository as test drawings is interoperability and allowed (a drawing kept here needs its row in `ATTRIBUTION.md` like any other file asset). File formats come from public specifications (Autodesk's published DXF Reference, the Open Design Alliance's public DWG specification), from DWG and DXF files themselves, and from our own tests. Never copy or link GPL/LGPL/AGPL code (LibreDWG, LibreCAD, QCAD, FreeCAD's planegcs, SolveSpace, OpenCASCADE) — don't even read their sources.
- **Assets.** DraftCraft contains **no Autodesk, Adobe or Avid iconography, images, fonts, patterns or artwork — ever**, and no brand marks other than DraftCraft's own. Every icon is drawn in code (`crates/ui-egui/src/icons.rs`), every hatch pattern and linetype is our own definition (`crates/doc/src/library.rs`), the drafting font is our own (`crates/fonts/src/stroke.rs`). Any file asset must be original, public domain/CC0, OSI-licensed or redistributable Creative Commons, and **must have a row in `ATTRIBUTION.md`** with author, source and licence (`cargo xtask assets` enforces it).
- **Fonts** come from [`storytold/craft-fonts`](https://github.com/storytold/craft-fonts), an optional build input (`CRAFT_FONTS_DIR`, embedded by `crates/fonts/build.rs`), never a `Cargo.toml` dependency and never committed here. Code and tests must work without it.
- **Everything is a command.** User-visible behaviour = a command in `crates/engine/src/cmd/*` (`CommandSpec`: id = AutoCAD's command name in lower case, label, menu path, shortcut, aliases, params doc, `enabled`, JSON `run`, optional `interactive` prompt machine) + tests. UI-only commands live in `crates/ui-egui/src/menus.rs` (`UI_COMMANDS`). The ribbon, menus, command line, scripts, CLI, control channel and MCP all reach the same commands.
- **Programmatic calls never open dialogs.** `engine.execute` runs the JSON form with defaults; only ribbon/menu invocation starts the interactive prompt sequence.
- **Layering** is enforced by `cargo xtask layers`: L0 `geom`, `dxf` → L1 `color`, `doc` → L2 `fonts`, `render`, `constraints` → L3 `io` → L4 `engine` → L5 `ui-egui`, `mcp` → apps. Nothing below L5 depends on egui/eframe/winit/wgpu/rfd.
- **The UI is thin**: panels read engine state and act through `app.run(id, params)` / `app.start(id)` / `app.cmdline(text)`. Colours come from `theme::Tokens`.
- **Time goes through `chrono`.** Never hand-roll calendar conversion.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).

## The interface is Windows-style
- The window top is `crates/ui-egui/src/ribbon.rs`: application button and menu, Quick Access Toolbar,
  search box, ribbon tabs (`TABS`) and their panels. A new user-facing command gets a place there, in the
  panel AutoCAD on Windows puts it in, with an icon drawn in `icons.rs`.
- No macOS-style tool bar or Tool Sets strip: the Tool Sets and the docked palettes are optional panels,
  closed by default and opened from the ribbon's View tab.
- Every new interface string gets an Italian translation in `crates/ui-egui/src/i18n/it.tsv`; menu
  entries also need one in `zh.tsv` (a test checks it).

## Running and looking at the app
- `cargo run --release -p draftcraft -- --sample --control PORT` (sample drawing + control channel). Pick a free port.
- Drive it with JSON lines on `127.0.0.1:PORT`; each request carries the token from `~/.cache/draftcraft-control-PORT.token`:
  - `{"id":1,"method":"cmdline.input","params":{"text":"circle 0,0 5"},"token":"…"}` — type at the command line exactly like a user.
  - `{"id":2,"method":"engine.execute","params":{"command":"line","params":{"points":[[0,0],[10,0]]}},"token":"…"}` — JSON form.
  - `{"id":3,"method":"ui.screenshot","params":{"path":"/tmp/shot.png"},"token":"…"}` then read the PNG.
  - Methods: `crates/ui-egui/src/control.rs`; protocol doc: `docs/control-protocol.md`.
- **For UI work, look at the result** (screenshot, read the PNG). `DRAFTCRAFT_CONFIG_DIR=<empty dir>` starts with fresh settings; `DRAFTCRAFT_LOCALE=en` forces the language.
- Headless: `draftcraft-cli run --sample --script 'LINE 0,0 10,10\n' --export out.png`; `draftcraft-cli info file.dxf`; `draftcraft-cli mcp`.
- Shell gotcha: `mv`/`cp` may be aliased interactive — use `/bin/mv -f` / `/bin/cp -f`.

## Quality gates
- Before every commit: `cargo xtask ci` (fmt, clippy -D warnings, tests, assets, layers, wasm).
- Three tests fail only on macOS and pass on Linux CI: two need file names that APFS refuses
  (`read_file_refuses_a_name_that_is_not_unicode`, `usage::non_unicode_output_names`) and one needs an
  installed Thai font (`thai_marks_stack_in_an_installed_font`).
- `.github/workflows/ci.yml` runs the checks on every push; `.github/workflows/release.yml` builds every
  package (macOS DMG, Windows, Linux) when a `v*` tag is pushed (see `docs/releasing.md`).
