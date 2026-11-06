<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/draftcraft-logo-dark.svg">
    <img src="docs/images/draftcraft-logo.svg" height="140" alt="DraftCraft™: a drafting compass that has just drawn an arc, on a dark blueprint-blue tile with a drafting grid, beside the name DraftCraft with Draft in bold">
  </picture>
</p>

<p align="center">
  <b>Open-source drafting that keeps your DWG drawings whole, built in pure Rust.</b>
</p>

<p align="center">
  DWG and DXF drawings opened, edited and saved without losing what you didn't touch: the command line,
  object snaps, layers, dimensions, hatches, blocks and layouts you already know. It runs natively on macOS,
  Windows, Linux and FreeBSD, and in the browser via WebAssembly. Built on CADCraft by the ArtCraft team.
</p>

<p align="center">
  <a href="https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-windows-x64.msi"><img alt="Download for Windows" src="https://img.shields.io/badge/Windows-Download-0078D6?style=for-the-badge&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCIgZmlsbD0id2hpdGUiPjxwYXRoIGQ9Ik0zIDNoOC41djguNUgzek0xMi41IDNIMjF2OC41aC04LjV6TTMgMTIuNWg4LjVWMjFIM3pNMTIuNSAxMi41SDIxVjIxaC04LjV6Ii8%2BPC9zdmc%2B&logoColor=white"></a>
  <a href="https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-macos-universal.dmg"><img alt="Download for macOS" src="https://img.shields.io/badge/macOS-Download-333333?style=for-the-badge&logo=apple&logoColor=white"></a>
  <a href="https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-x86_64.AppImage"><img alt="Download for Linux (AppImage)" src="https://img.shields.io/badge/Linux-AppImage-E95420?style=for-the-badge&logo=linux&logoColor=white"></a>
  <a href="https://draftcraft.dev/"><img alt="Use it in your browser" src="https://img.shields.io/badge/Web-Use_in_browser-0e3463?style=for-the-badge&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCIgZmlsbD0ibm9uZSIgc3Ryb2tlPSJ3aGl0ZSIgc3Ryb2tlLXdpZHRoPSIyIj48Y2lyY2xlIGN4PSIxMiIgY3k9IjEyIiByPSIxMCIvPjxwYXRoIGQ9Ik0yIDEyaDIwTTEyIDJhMTUgMTUgMCAwIDEgMCAyME0xMiAyYTE1IDE1IDAgMCAwIDAgMjAiLz48L3N2Zz4%3D&logoColor=white"></a>
</p>

<p align="center">
  <img alt="Status: in active development" src="https://img.shields.io/badge/status-in%20active%20development-0e3463?style=flat-square">
  <img alt="Written in pure Rust" src="https://img.shields.io/badge/pure-Rust-b83a24?style=flat-square&logo=rust&logoColor=white">
  <img alt="Runs on macOS, Windows, Linux, FreeBSD and the web" src="https://img.shields.io/badge/runs%20on-macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-555555?style=flat-square">
  <img alt="MCP server for agents" src="https://img.shields.io/badge/agents-MCP%20server-555555?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-555555?style=flat-square">
</p>

<p align="center">
  <a href="https://draftcraft.dev/"><b>Try DraftCraft in your browser</b></a> ·
  <a href="#screenshots">Screenshots</a> ·
  <a href="#downloads">Downloads</a> ·
  <a href="#why-draftcraft">Why DraftCraft</a> ·
  <a href="#what-works-today">What works today</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#built-on-cadcraft">Built on CADCraft</a> ·
  <a href="#license-and-credits">License</a>
</p>

> [!NOTE]
> **DraftCraft is a fork.** It's [CADCraft](https://github.com/storytold/cadcraft), the ArtCraft team's
> open-source CAD application, grown toward native, lossless DWG by [WithShalom Studios](https://github.com/withshalom).
> It is not made, endorsed or supported by the ArtCraft team, and it is not affiliated with Autodesk, Inc.
> AutoCAD and DWG are Autodesk's trademarks, used only to say which files and workflows DraftCraft works with.

<p align="center">
  <img src="docs/images/ui-apartment.png" alt="DraftCraft with an apartment floor plan open: hatched grey walls, blue windows, green door swings, furniture outlines, yellow room names with areas, a yellow room schedule table, a multileader note and cyan dimensions with architectural ticks; Tool Sets on the left, Layers and Properties on the right, the command line at the bottom" width="100%">
  <br><sub><b>Apartment plan</b> (<a href="examples/apartment.dxf">examples/apartment.dxf</a>): walls with pick-point hatching, TrueType MTEXT room labels, a TABLE, a multileader and architectural dimensions, built entirely from commands.</sub>
</p>

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/images/ui-layout.png" alt="A paper-space layout in DraftCraft: a white sheet with a dashed printable area and a viewport showing the apartment plan at scale, the status bar showing the A1 Plan layout tab and a PAPER toggle"></td>
    <td width="50%"><img src="docs/images/ui-bracket.png" alt="DraftCraft with a mounting bracket part drawing: front view with bolt holes, centre lines and dimensions, a hatched section view, notes and a title block"></td>
  </tr>
  <tr>
    <td><sub><b>Layouts</b>: paper space with viewports, page setups and PLOT to PDF. Double-click a viewport to work in model space through it.</sub></td>
    <td><sub><b>Mounting bracket</b>: a two-view part drawing with centre lines, hidden lines, an ANSI31 section hatch and a title block.</sub></td>
  </tr>
</table>

## Why DraftCraft

- **The workflow you know.** Type `L`, click two points, type `@5<45`, press Enter. The command
  line, prompts with clickable `[Keywords]`, AutoComplete, object snaps, polar tracking, ortho,
  direct distance entry, window and crossing selection, grips, and right-click-to-repeat behave
  the way decades of drafting habit expect.
- **Open files.** DXF is read and written natively (ASCII and binary, R12 through 2018), and DWG
  files (R13 through 2018) open and save through WithShalom's open-source DWG and DXF crates first
  and the acadrust library second. Export to SVG and PNG today.
- **Fast and native.** Pure Rust and egui, no Electron, no web view. One binary on macOS
  (universal), Windows, Linux and FreeBSD, plus a WebAssembly build for the browser.
- **Built for agents.** Every menu item, tool and prompt is a command. Agents can type at the
  command line exactly like a person, call any command with JSON, inspect the drawing to verify
  their work, and render it — over MCP, a JSON control channel, or the CLI.
- **Free.** MIT OR Apache-2.0, with no account and no subscription.

## What works today

DraftCraft is in early, fast development. Honest status (see [ROADMAP.md](ROADMAP.md) for parity
numbers):

| Area | Status |
|---|---|
| Drawing area | Model space with adaptive grid, axes, pan/zoom (wheel, middle-drag, pinch), crosshair cursor with pickbox, UCS icon, ViewCube, viewport label |
| Command line | Prompts with keywords, history, AutoComplete, aliases, `@dx,dy`, `@d<a`, `#x,y`, direct distance entry, Enter/space/right-click to repeat, transparent commands, `.scr`-style scripts |
| Draw | LINE, PLINE (arcs, widths), CIRCLE (center/radius/diameter, 2P, 3P), ARC, RECTANG (fillet/chamfer), POLYGON, ELLIPSE (+arcs), SPLINE, POINT, XLINE, RAY, DONUT, TEXT, MTEXT |
| Modify | ERASE, MOVE, COPY, ROTATE, SCALE, MIRROR, STRETCH, OFFSET, TRIM, EXTEND, FILLET, CHAMFER, BREAK, JOIN, EXPLODE, rectangular/polar ARRAY, draw order, OVERKILL |
| Precision | Object snaps (endpoint, midpoint, center, geometric center, node, quadrant, intersection, insertion, perpendicular, tangent, nearest; deferred tangent/perpendicular for the first point of a line, e.g. belt lines tangent to two circles), polar tracking, ortho, grid snap |
| Layers & properties | Layers palette and Layer Properties Manager (on/off, freeze, lock, plot, colour, linetype), layer tools (isolate, freeze, off, lock, match, previous), Properties palette with per-object editing, linetypes, lineweights, colour index and true colour |
| Annotation | All DIM* commands with full DIMSTYLE variables, overrides and every arrowhead, associative dimensions that follow geometry, MLEADER, TABLE, TrueType fonts (`.ttf`, `.otf`, `.ttc` collections), MTEXT formatting codes (fonts, heights, colours, stacked fractions), our own single-stroke drafting font |
| Hatch & blocks | Pick-point hatch boundaries with islands, pattern, solid and gradient fills from our own pattern library, BLOCK/INSERT, attributes and nested blocks |
| Files | DXF read (R12–2018, ASCII and binary) and write (R2000), including dimension styles, associativity, tables and constraints; DWG read and write (WithShalom's open DWG crates first, the acadrust library second); PDF plotting, SVG and PNG export. Entities DraftCraft doesn't model yet are not kept on save: see [file-format parity](docs/file-format-parity.md) |
| Layouts & plotting | Paper-space layouts, viewports (scale, lock, per-viewport layer freeze), MSPACE/PSPACE through viewports, page setups, PLOT and EXPORTPDF |
| Parametric | Geometric and dimensional constraints, AUTOCONSTRAIN, PARAMETERS with expressions, conflict detection; constraints re-solve after every edit |
| Grips | Hot grips with Space to cycle Stretch, Move, Rotate, Scale and Mirror |
| Interface languages | English, Ukrainian (Українська), Brazilian Portuguese (Português (Brasil)), German (Deutsch), Spanish (Español), Czech (Čeština), Russian (Русский), French (Français), Simplified Chinese (简体中文, with an installed CJK font), Italian (Italiano), Korean (한국어, with an installed Hangul font), Swedish (Svenska), Arabic (العربية, with an installed Arabic font), Hungarian (Magyar) and Turkish (Türkçe), live switching and a remembered Auto/system-language choice; [details](docs/localization.md) |
| Automation | MCP server, JSON control channel, `cadcraft-cli` (info, convert, run, commands, mcp) |

## Quick start

DraftCraft requires Rust 1.95 or later.

```sh
git clone https://github.com/withshalom/draftcraft
cd draftcraft
cargo run --release -p cadcraft -- --sample       # opens the sample drawing
```

Then try typing at the command line:

```text
line 0,0 @10,0 @0,5 c          a closed triangle
circle 5,2 1                    a circle
offset 0.25                     then pick the circle and a side
zoom e                          zoom to extents
```

The web build: `cd apps/cadcraft-web && trunk serve` (needs [trunk](https://github.com/trunk-rs/trunk)).

### Fonts

DraftCraft has no fonts in its repository; it uses the TrueType/OpenType fonts installed on your
system. Builds made with the optional [craft-fonts](https://github.com/storytold/craft-fonts)
input (`CRAFT_FONTS_DIR=<absolute path to a checkout> cargo build --release`) also embed its
fonts: BIZ UDPGothic, BIZ UDMincho and Shippori Mincho (Japanese) and Noto Sans Arabic (the web
build, which has no system fonts, only BIZ UDPGothic Regular and Noto Sans Arabic). A text style
can use them by family name, and they draw characters a text's font lacks when no installed
wide-coverage CJK font has them. Without `CRAFT_FONTS_DIR` nothing is embedded. SHX
fonts (including big fonts such as `chineset.shx`) aren't read yet: text in an SHX style is drawn
with our own stroke font, and any character a text's font lacks (CJK, for example) is drawn from
an installed font that has it, preferring wide-coverage fonts such as Noto Sans CJK, Source Han
Sans, WenQuanYi, PingFang or Microsoft JhengHei. Two settings, set with `SETVAR` or (for every
run) an environment variable, steer this:

| Variable | Environment | Meaning |
|---|---|---|
| `FONTALT` | `CADCRAFT_FONTALT` | Font used instead of a text style font that can't be found (an SHX file, a missing TTF), by file or family name, e.g. `Noto Sans CJK TC`. Empty (default) = our stroke font. |
| `FONTFALLBACK` | `CADCRAFT_FONTFALLBACK` | Fonts tried first for characters a text's font lacks, comma-separated, e.g. `Noto Sans CJK TC, msjh.ttc`. |

`.` clears either one.

**Your own fonts.** Any `.ttf`, `.otf` or `.ttc` font works: install it in the system font folder
(Windows `C:\Windows\Fonts` or per-user fonts, macOS `/Library/Fonts` or `~/Library/Fonts`,
Linux and the BSDs `/usr/share/fonts`, `/usr/local/share/fonts`, `~/.local/share/fonts` or
`~/.fonts`) and name it in a text style (`STYLE`, or `style {"name":"FA","font":"Vazirmatn"}`)
by file name (`Vazirmatn-Regular.ttf`) or family name (`Vazirmatn`), or inline in MTEXT with
`\fVazirmatn;`. Restart DraftCraft after installing a font: the font folders are scanned once.

**Right-to-left text (Persian, Arabic, Urdu, Hebrew).** TEXT and MTEXT lines are laid out with the
Unicode Bidirectional Algorithm (the paragraph direction comes from its first letter, so numbers
and Latin words inside Persian text keep their own order), and Arabic-script text is shaped with
OpenType (contextual letter forms, lam-alef and other ligatures, ZWNJ / ZWJ, mark positioning).
Justification (left, centre, right, MTEXT attachment and wrapping) still refers to the text's box.
Our stroke font has no Arabic letters, so Persian in a stroke-font style is drawn from an
installed font that has them: Vazirmatn, Noto Naskh Arabic, Noto Sans Arabic, Segoe UI, Tahoma,
Arial and DejaVu Sans are looked for first. To choose the font, install a free Persian font such
as [Vazirmatn](https://github.com/rastikerdar/vazirmatn) (SIL Open Font License) and either use it
in the text style or set `FONTFALLBACK` (`CADCRAFT_FONTFALLBACK="Vazirmatn"`). Formatting changes
inside a word (an MTEXT colour change in the middle of a Persian word) break the joining at that
point.

### Logs

The desktop app writes its `log` records to standard error and to `logs/cadcraft.log` in the
settings directory (Linux and the BSDs `~/.config/cadcraft/logs/`, or
`$XDG_CONFIG_HOME/cadcraft/logs/`; macOS `~/Library/Application Support/CADCraft/logs/`; Windows
`%APPDATA%\CADCraft\logs\`; or under `CADCRAFT_CONFIG_DIR`). A start from a desktop menu or the
Dock has no terminal, so this file is what to attach to a bug report. Each launch moves the
previous log to `cadcraft.1.log` (and that one to `cadcraft.2.log`), so the log of a run that
crashed survives the next start. The file stops growing at 16 MiB. `--version` writes no file.

| Variable | Effect |
|---|---|
| `CADCRAFT_CONFIG_DIR` | Use this directory instead of the platform settings directory (the log goes to its `logs/`); empty means no settings folder (no log file, autosave, workspace or plotters) |
| `RUST_LOG` | Log levels for standard error and the log file. Default: `info` for DraftCraft's own crates, `warn` for everything else. env_logger-style directives replace that, e.g. `RUST_LOG=debug`, `RUST_LOG=warn,cadcraft_io=trace` or `RUST_LOG=info,wgpu_core=warn`; a directive ending in `*` covers every target starting with it (`cadcraft*=debug`). `cadcraft_engine=debug` also logs every command-line message. |

The logger is `apps/cadcraft/src/logging.rs`; the web build logs to the browser console instead.

## Drive it from agents, MCP and the CLI

```sh
# MCP server on stdio, headless:
cadcraft-cli mcp
# …or bridged to the running app (start it with --control 7979):
cadcraft-cli mcp --connect 127.0.0.1:7979

# One-shot headless runs:
cadcraft-cli run --sample --script 'CIRCLE 22,3 1\n' --save out.dxf --export out.png
cadcraft-cli info drawing.dxf
cadcraft-cli convert drawing.dxf drawing.svg

# Frame part of the drawing (model space, drawing units, lower-left corner first):
cadcraft-cli convert drawing.dwg detail.png --window 0,0,20000,15000
cadcraft-cli run drawing.dwg --cmd 'zoom {"mode":"window","p1":[0,0],"p2":[20000,15000]}' --export detail.png
cadcraft-cli run drawing.dwg --cmd 'select {"window":[[0,0],[20000,15000]]}' --cmd 'zoom {"mode":"object"}' --export sel.png

# Act on what an earlier --cmd made: {last} is the last handle it returned.
cadcraft-cli run --metric --cmd 'layout.set {"name":"Layout1"}' \
  --cmd 'mview {"p1":[20,20],"p2":[200,150]}' --cmd 'viewport.set {"handle":"{last}","scale":1}'
```

How image exports (PNG 2400×1600, SVG, PDF) are framed: `convert` fits the model-space extents.
`run --export` shows the current view when the script or commands changed it (`zoom`, `pan`,
`view.set`, …) and otherwise fits the extents too. `--window x0,y0,x1,y1` shows exactly that
rectangle, centred and scaled without distortion; `--view extents` or `--view current` forces
either framing. To frame a selection set, `zoom {"mode":"object"}` after selecting.

MCP tools include `command_line` (type at the prompt), `execute` (any command with JSON),
`inspect_drawing`, `query_entities`, `render` (returns a PNG) and, when connected to the app,
`screenshot` and `ui_click`. The desktop app's JSON control channel is documented in
[docs/control-protocol.md](docs/control-protocol.md).

To run LISP routines, scripts and alias files, or to drive DraftCraft from your own programs, see
[docs/customization.md](docs/customization.md).

`cadcraft-cli --help` documents every subcommand and flag; exit codes, streams, environment
variables and its RFC-0012 conventions are in [docs/cli.md](docs/cli.md).

## Architecture

```text
geom ─┐                       f64 geometry: arcs, bulges, splines, intersections, offsets
dxf   │  (standalone)         DXF tag reader/writer
color ┤                       colour index, true colour
doc   ┤                       drawing database, copy-on-write entity store (cheap undo)
fonts ┤ render               stroke font + TEXT/MTEXT layout │ display lists, linetypes, hatches, dims, CPU raster
io    ┤                       DXF mapping, SVG/PNG export
engine┤                       sessions, commands, command line + prompts, snaps, selection, undo
ui-egui · mcp                 the swappable egui front end · MCP server
apps: cadcraft · cadcraft-cli · cadcraft-web
```

Nothing below `ui-egui` knows about egui, so the front end can be replaced. `cargo xtask ci`
checks formatting, clippy, tests, asset attribution, the crate layering and the wasm build.

Drafting and layer-palette icon buttons use the pinned `craft-ui` button component for interaction,
accessible labels and keyboard focus. DraftCraft keeps its multicolor icon painter, dark palette,
disabled tint and tool commands. The native host saves workspace visibility and toolset choices
atomically; `CADCRAFT_CONFIG_DIR` selects an isolated profile. Compact toolbar groups move into a
More menu, and status readouts truncate with a full-value tooltip before they reach layout tabs.
Painted toolbar and tab controls
have accessible labels and visible keyboard focus. Property controls remain application-specific.

## Accessibility

### System cursor

Over the drawing area DraftCraft hides the system cursor and draws its own crosshair. Tools that follow the system cursor (Windows Magnifier set to follow or centre the mouse pointer, other screen magnifiers, screen recorders, remote desktops) lose track of it there. Turn on **View ▸ Accessibility ▸ Show System Cursor** to keep a small system crosshair visible at the centre of the drawn one, or start DraftCraft with the environment variable `CADCRAFT_SYSTEM_CURSOR=1` (on Windows, set it once with `setx CADCRAFT_SYSTEM_CURSOR 1` and start DraftCraft again). Agents can switch it with the control channel: `ui.set {"systemCursor": true}`.

### Animation effects

DraftCraft follows the operating system's animation setting. When the system is set to show no animations, dialogs, menus and tooltips appear at once instead of fading in, collapsing sections and scroll bars change at once, and the mouse wheel scrolls lists a whole step at a time instead of gliding. Zooming the drawing with the wheel always moves a whole step; with Ctrl held (a touchpad pinch, on Windows) it glides to the new zoom only while animation effects show.

| System | Where the setting is |
|---|---|
| Windows | Settings ▸ Accessibility ▸ Visual effects ▸ Animation effects |
| macOS | System Settings ▸ Accessibility ▸ Reduce motion |
| Linux, BSD | the desktop's reduced-motion (or animations) setting, read through the XDG Desktop Portal |
| Web | the browser's `prefers-reduced-motion` |

A change to the system setting is picked up while DraftCraft runs, within a few seconds. To choose for DraftCraft alone, use **View ▸ Accessibility ▸ Animation Effects**: **Use System Setting** (the default), **On** or **Off**. Options ▸ Display has the same choice. Agents can set it with the control channel: `ui.set {"animations": "off"}` (`"system"`, `"on"` or `"off"`), and `ui.inspect` reports whether animations show (`animations`).

Drafting feedback is not an animation effect and stays as it is: rubber bands and previews, snap markers, the pause that acquires an object snap tracking point, the text cursor's blink, the spinner shown while a drawing opens, and pan and zoom, which follow the pointer and the wheel directly.

## Roadmap

See [ROADMAP.md](ROADMAP.md) for the stage, measured parity and the estimate of remaining work, and [docs/gaps.md](docs/gaps.md) for everything still missing.

## Downloads

One click downloads the installer from the [latest release](https://github.com/withshalom/draftcraft/releases/latest):

| System | Download |
|---|---|
| Windows 10 and 11 | [64-bit (x64)](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-windows-x64.msi) · [ARM64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-windows-arm64.msi) · [32-bit (x86)](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-windows-x86.msi) |
| macOS 11 or later (Apple silicon and Intel) | [Disk image](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-macos-universal.dmg) |
| Linux, any distribution | [AppImage x86_64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-x86_64.AppImage) · [AppImage ARM64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-aarch64.AppImage) |
| Debian, Ubuntu, Mint, Pop!_OS | [.deb x86_64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-x86_64.deb) · [.deb ARM64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-aarch64.deb) |
| Fedora, openSUSE, RHEL | [.rpm x86_64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-x86_64.rpm) · [.rpm ARM64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-aarch64.rpm) |
| Flatpak | [x86_64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-x86_64.flatpak) · [ARM64](https://github.com/withshalom/draftcraft/releases/latest/download/draftcraft-linux-aarch64.flatpak) |
| Browser | Nothing to install: [draftcraft.dev](https://draftcraft.dev/) |

The [release page](https://github.com/withshalom/draftcraft/releases/latest) also has the Windows portable
zips, the Arch Linux packages, `.tar.gz` builds (including RISC-V), the macOS command-line tool, the web build
and `SHA256SUMS.txt`. The Windows and Linux downloads include `draftcraft-cli`, the command-line and MCP tool,
next to the app; on macOS it's the separate `draftcraft-cli-…-macos-universal.zip`.

The downloads aren't code-signed yet. On Windows, if SmartScreen warns, choose **More info**, then **Run
anyway**. On macOS, the first time, right-click DraftCraft in Applications and choose **Open**.

DraftCraft tells you when a newer version is out (it checks when it starts; Help › Check for Updates… checks
now). The AppImage, `.deb`, `.rpm` and Windows portable copies install it in one click; with the others,
download the new installer and run it over the old one.

## Built on CADCraft

DraftCraft is [CADCraft](https://github.com/storytold/cadcraft), one of the ArtCraft team's Crafting Apps, with
WithShalom Studios' DWG work on top:

- **DWG through WithShalom's open DWG and DXF crates first**, and the acadrust library second, in every build
  including the browser's.
- **Lossless round trips**, on this repository's `cadcraft-dwg-lossless` branch and in review upstream as
  [storytold/cadcraft#2456](https://github.com/storytold/cadcraft/pull/2456): every object keeps its handle and
  type, and the objects you didn't change are saved as the file had them. They come to the main build once
  upstream takes them.

The code stays CADCraft's wherever it can, so upstream's improvements keep merging in. The installers put
`draftcraft` and `draftcraft-cli` on your system; in the source, the crates and binaries keep CADCraft's names
(`cargo run -p cadcraft`, `cadcraft-cli`).

## License and credits

DraftCraft, like CADCraft, is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Copyright (c) 2026 ArtCraft Team, the CADCraft contributors and WithShalom Studios. Required notices are
in [NOTICE](NOTICE).

Bundled fonts, icons, images and other assets keep their own open licenses; each one is listed with its author,
source and license in [ATTRIBUTION.md](ATTRIBUTION.md). DraftCraft's app icon and logo are original artwork
(`packaging/compass-icon.py`, `packaging/draftcraft-logo.py`); CADCraft's icons, its single-stroke drafting font,
its hatch patterns and its linetypes are original work, drawn or defined in code.

The ArtCraft name, wordmark and logos in [`docs/brand/`](docs/brand/) are trademarks of the ArtCraft Team. Under
[`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt), forks must remove them, so DraftCraft doesn't show
them; the files stay in `docs/brand/` only because they are upstream's.

Tool Sets, Layers, and Properties can be regrouped, split, floated within the app window, closed, and reopened. The drawing canvas stays visible and cannot be closed or turned into a tab. Drag a panel header to a target edge or another header. Its context menu offers Float panel, Move to group, and Close panel. Existing panel toggles reopen panels at their saved location; Reset Panel Layout restores the app layout. Floating panels remain inside the current app window.
