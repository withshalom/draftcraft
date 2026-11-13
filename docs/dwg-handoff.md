# DWG and DXF in 3.0.0: handoff

For whoever reviews, tests and patches this work next. The design and the full record of
findings are in [`dwg.md`](dwg.md); this page is the state today, how to build and test it, and
what is left.

## State

- **Reading DWG**: DraftCraft's own reader (`draftcraft_io::dwg`), every version from R13 to
  AutoCAD 2018 and later, 2007 included. It is the default. acadrust is not used to open a file
  unless `DRAFTCRAFT_DWG_READER=acadrust` is set.
- **Writing DWG**: still through acadrust (`crates/dwg/src/acadrust`), from DraftCraft's DXF.
  DraftCraft's own writer (`crates/dwg/src/write`, AutoCAD 2018 only) exists but is not used for
  saving. **acadrust can't be switched off for saving until that writer is finished**: with it
  off there is no DWG save at all.
- **DXF**: linetype element rotations are degrees, as AutoCAD writes them (older DraftCraft files
  are still read as radians); typed table cells show AutoCAD's formatted text.

## Build and run

```bash
cargo build --release -p draftcraft
```

```bash
cargo run --release -p draftcraft -- --sample
```

```bash
cargo run --release -p draftcraft-cli -- info some.dwg
```

`info` prints `readNotes`: what the reader left out of the drawing, by type. An empty list means
the drawing holds everything the file does.

The web build: `trunk build --release` in `apps/draftcraft-web`. It always reads directly.

## Test

The gate CI runs:

```bash
cargo xtask ci
```

It stops at the first failing test. To see every result:

```bash
cargo test --workspace --no-fail-fast
```

The reader's own tests, and the round trips through DWG:

```bash
cargo test -p draftcraft-dwg -p draftcraft-io
```

Against AutoCAD's own files, with AutoCAD's DXF of the same drawing as the reference. The MI-201
drawings are in the repository (`crates/io/tests/data/mi201`). The ACadSharp samples (MIT) are
not; point the tests at a folder of them:

```bash
DRAFTCRAFT_DWG_SAMPLES=/path/to/ACadSharp/samples cargo test -p draftcraft-io --test it dwg_direct
```

Every difference between a DWG reading and AutoCAD's DXF, one per line:

```bash
cargo run -p draftcraft-io --example direct_sweep -- --deep sample_AC1032.dwg sample_AC1032_ascii.dxf
```

To compare readers on one file, set `DRAFTCRAFT_DWG_READER` to `acadrust` (through DXF) or
`native` (the first mapping), and open the file again.

## What fails today

Four tests of `cargo test -p draftcraft-io --test it`; everything else in the workspace passes.

| Test | Needs |
|---|---|
| `acis_data_store::dwg_keeps_the_solids_geometry` | The data store section of 2013-and-later files (`AcDb:AcDsPrototype_1b`, DXF's ACDSDATA), where solids keep their geometry |
| `acis_data_store::sat_text_solids_stay_in_dwg_2018_and_xref_contents_are_not_counted` | 3DSOLID, REGION and BODY as 2013-and-later files store them (in AutoCAD's 2018 region the DXF's identifier, group 2, is the 16 bytes at bit 157 of its data; what is before it is not worked out) |
| `dynblock_read::a_saved_drawing_keeps_the_dynamic_block_as_read` | The dynamic-block objects: ACAD_EVALUATION_GRAPH and the BLOCK…PARAMETER, …ACTION and …GRIP types, as their DXF groups |
| `dynblock_read::a_scale_action_survives_a_dwg_save` | Same |

## What a file can still lose on opening

Nothing is left out silently: each of these is named in the drawing's read note, with a count,
and the note says saving over the file would lose it.

**Geometry that is not drawn**

- 3DSOLID, REGION and BODY whose description is binary (AutoCAD 2004 and later files) or in the
  data store (2013 and later). Text ones (AutoCAD 2000 files, DraftCraft's own) are kept.
- MESH (subdivision meshes) and SHAPE (a shape from an SHX file: its name needs the file).
- ARC_DIMENSION and LARGE_RADIAL_DIMENSION are read as far as the drawing has a place for them
  (see "Decisions open" in `dwg.md`).

**Behaviour that is not kept**

- Dynamic blocks open as the plain blocks they were last shown as; their parameters, actions
  and grips are not kept, so they can't be changed and a save drops them.
- Draw order (SORTENTSTABLE): entities draw in the file's order, not the order set with
  DRAWORDER.
- An entity's own plot style (in a drawing that uses named plot styles).

**Data kept by DXF but not yet by DWG**

- MATERIAL, VISUALSTYLE and the section and detail view styles before 2013; CELLSTYLEMAP,
  TABLECONTENT, TABLEGEOMETRY.
- A material set ByBlock or on a layer (DXF group 347).

## Known differences from AutoCAD's DXF

On the ACadSharp sample, the DWG reading against AutoCAD's DXF of the same drawing
(lost / added / changed): 2000: 242 / 17 / 200. 2004: 249 / 11 / 137. 2007: 252 / 10 / 82.
2010: 252 / 14 / 69. 2013: 177 / 14 / 94. 2018: 162 / 14 / 53. Most are the objects above. Open
items among the rest:

- A true colour an AutoCAD 2000 file keeps in round-trip data is read as the true colour from
  DWG and as its nearest index colour from DXF. The DXF reader should take it too.
- A wipeout's boundary has one vertex fewer from DWG (the closing vertex).
- A fit-point spline in 2013 and later: the DWG reading keeps the fit points, the DXF has only
  control points.
- DXF: `dxf_table::CELL_TYPES` is `[DATA, HEADER, TITLE]`; AutoCAD's order is data, title,
  header. Fixing it needs a marker so older DraftCraft DXFs still read right.

## Not confirmed against a file AutoCAD saved

Listed in `dwg.md` ("Not yet confirmed"): a LEADER's annotation size from 2010, a multiline
style element's linetype before 2018, a hatch's elliptic edge, a diameter dimension's point
order, an OLE object's wrapper. A drawing from AutoCAD with each would settle it.

## What is left, in order

1. The four failing tests: the data store, then the dynamic-block objects (step 12c).
2. The other types in the read note (step 12c).
3. DraftCraft's own DWG writer: objects as 2018, sections and container, a read-back check on
   every save (steps 15 and 16); then the other save versions (17).
4. acadrust off for saving, then removed (18).

## Where things are

| Path | What |
|---|---|
| `crates/dwg/src/read/` | Containers (`r2000.rs`, `r2004.rs`, `r2007.rs`), sections, classes, the object map |
| `crates/dwg/src/file.rs` | `DwgFile`, `ObjectReader`: an object's fields in the order the format stores them |
| `crates/io/src/dwg/` | The reader: `header`, `tables`, `blocks`, `entities`, `table`, `objects`, `carry` (objects kept or rebuilt) |
| `crates/io/src/dxf_raw.rs` | The DXF reader's rule for what is kept without modelling; the DWG reader follows it |
| `crates/io/tests/it/dwg_direct.rs` | The reader against AutoCAD's files |
| `crates/io/examples/direct_sweep.rs` | Sweeps, stress drawings, the deep comparison |

## Clean-room

The format comes from the Open Design Alliance's public DWG specification, from DWG and DXF
files themselves, and from our own tests. acadrust (MPL-2.0) was consulted for layout facts,
each confirmed on a file; no code was copied. No GPL, LGPL or AGPL source was read.
