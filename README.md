# cadforge

A CAD-shaped web studio for training small models that **generate parametric
CAD**: you show it shapes and the operations that produced them, it learns to
produce the operations for a new shape from the shape alone.

The plan lives in `ARCHITECTURE.md`. This file is how to run it.

## Run it

**Windows 11: double-click `run.cmd`.**

The first time it builds the studio (a few minutes) and needs Rust - it tells you
exactly how to get it if it is missing. Every time after that it starts in a
second and **opens your browser by itself** on http://localhost:8787.

Everything else is already in this folder: the web interface, the CAD readers,
the rebuild engine, and sample documents. Nothing is downloaded at runtime and
nothing needs the network.

After the first build, `target\release\cadforge.exe` *is* the studio - pin it to
the taskbar, or make your own shortcut to it.

**macOS / Linux:** `./run.sh` does the same thing.

Or drive it by hand - the whole app is one Rust binary and a `static/` folder:

```sh
cargo run                       # http://localhost:8787
cargo run -- --port 9000        # another port
cargo run -- --data /tmp/cf     # another storage dir (default: ./data)
```

Then open http://localhost:8787. There is no build step for the web UI, no
CDN and no network access needed: `static/vendor/` holds three.js.

**Want to see the interface without running anything?**
`tools/make_preview.py` freezes a running instance into a single self-contained
HTML file you can open from disk - same CSS, same `app.js`, same data. It is a
photograph, not the live app.

```sh
tools/make_preview.py --out preview/cadforge-preview.html
```

## Use it

1. **Add files.** There are three doors, and they mean different things:
   * **SolidWorks / CAD** - `.sldprt`, `.sldasm`, `.slddrw`: the studio opens them,
     reads their parameters and references, and shows their embedded preview.
   * **STEP / STP** - neutral CAD: the studio reconstructs the build history from
     the geometry.
   * **Images** - pictures of finished parts, for the vision side.

   Drop a whole batch onto a tile (or onto the viewport, where the lane is read
   from each file's extension). The **Processing** panel shows every file going
   queued → uploading → done, and each one appears in **Artifacts** the moment it
   has been read.
2. **Click an artifact** to load it; orbit with the mouse. The inspector shows a
   **How this was processed** tree: every stage from the upload to what was read
   out of the file, and for SolidWorks documents the parameters and the documents
   they reference.
3. **Capture views** (in the viewport toolbar) renders four flat views of the
   loaded part on a transparent background and lays them out as a drawing sheet
   with a title block. Both go back into Artifacts, ready to train on.
4. **Picture to steps** (right panel): drop in a picture and the studio writes the
   build steps it implies, then *Rebuild this* turns them into real geometry you
   can look at. This works with no model installed - it is the baseline your
   training has to beat.
5. Press *Register* under **Base models** to point the studio at a model.
6. Tick artifacts, then *New dataset from selected*.
7. Pick a model and a dataset, press **Start training run**, and watch the stages,
   progress bar and live log on the right.

## See it work with no CAD files

If you have nothing to upload yet, the studio can write its own sample
assignment: two parts, an assembly and a drawing, each a real compound file
carrying parameters, a preview, and - for the assembly - the parts it is made
of.

```sh
cargo run -- --emit-samples samples
```

Then upload all four in the studio. They are read exactly as a genuine
SolidWorks file is, so the whole pipeline is visible without a licence.

## Train a model

The pipeline ships with a simulation so the whole loop is visible end to end. To
train for real, install the Python stack once:

```bat
scripts\setup-ml.cmd
```

then point the studio at the trainer:

```bat
set CADFORGE_TRAINER=python ml/train_vision.py
run.cmd
```

Register a base model, build a dataset, press *Start training run*, and the log
fills in live. The studio describes the dataset as picture/operation pairs and
hands the trainer that manifest plus an output directory; that is the entire
interface between the two.

**Read [docs/TRAINING.md](docs/TRAINING.md) before planning a 4-billion-parameter
run.** It has the memory budget for an 8 GB card, the settings that matter, which
base models fit, how to install everything offline - and an honest account of what
such a model can and cannot learn.

To use a trained model in the studio:

```bat
set CADFORGE_VISION=python ml/predict.py
set CADFORGE_ADAPTER=checkpoints\run1\adapter
```

## Reading STEP parts

The studio reads STEP/STP directly - no CAD kernel required - and reconstructs
what the part is made of: topology counts, units, and the build steps implied by
the boundary representation (an extrude, holes, a hole pattern, fillets).

```sh
tools/make_step.py plate > plate.step     # 60x40x8 plate with 4 x D6 holes
tools/make_step.py box   > box.step       # plain box
```

Upload one, click it in the Artifacts tree, and the reconstructed history
appears in the inspector while the part renders in the viewport.

## Reading SolidWorks documents

`.SLDPRT`, `.SLDASM` and `.SLDDRW` are OLE2 compound files - a small filesystem
inside one file. The studio opens them directly, with nothing installed, and
reports what can be read honestly:

* **the document's parameters**, from its OLE property-set streams: title,
  subject, author, company, revision, the dates, and any custom properties
  (Material, Mass, Part Number...). These are the values a shop actually fills
  in, and they are read from the file, never guessed.
* **the documents it references** - an assembly's component list, a drawing's
  model - found by reading the text inside its streams.
* **the embedded preview image**, shown in the inspector.
* **the stream catalogue**: every stream with its size and role, including the
  ones that are not yet recognised.

The feature tree itself (`ThirdPv`) is a proprietary, undocumented encoding, so
it is *listed and named but not decoded* - decoding it honestly needs a real
sample to work against, and a confident wrong history would be worse than none.
For the true tree, use the export below.

### The real feature tree

SolidWorks will hand over the real tree itself. `tools/solidworks/ExportFeatureTree.bas`
is a macro that walks the FeatureManager in SolidWorks and writes it to JSON -
every feature, its parameters, and whether it is suppressed. Upload that JSON and
the studio shows the modelling history *as recorded*, not inferred from
geometry; it is the ground truth for training.

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `CADFORGE_PORT` | `8787` | listen port |
| `CADFORGE_DATA` | `data` | uploads, db.json, run manifests |
| `CADFORGE_STATIC` | `static` | web GUI directory |
| `CADFORGE_NO_OPEN` | unset | set it to stop the browser opening by itself |
| `CADFORGE_TRAINER` | unset | external trainer command; unset = built-in simulation |
| `CADFORGE_VISION` | unset | external picture-to-steps command; unset = built-in baseline |
| `CADFORGE_ADAPTER` | unset | adapter a vision command should load |
| `CADFORGE_PYTHON` | auto | the Python to probe for the training stack |
| `RUST_LOG` | `info` | log level |

## Layout

```
src/main.rs       server, options, opens the browser, sample documents
src/api.rs        HTTP API + SSE
src/model.rs      types and persistence
src/system.rs     what the machine can do: GPU, Python, training stack
src/vision.rs     picture -> build steps (the geometric baseline)
src/dataset.rs    pairs pictures with the operations that make them
src/pipeline.rs   the training pipeline and its stages
src/ole.rs        OLE2 compound-file reader (+ writer, for tests and samples)
src/sldworks.rs   SolidWorks documents: streams, preview, parameters, references
src/propset.rs    OLE property-set reader (the document's real parameters)
src/swhist.rs     SolidWorks feature-tree export -> build steps
src/stl.rs        STL parser (ASCII + binary)
src/step.rs       STEP (ISO 10303-21) reader + build-history reconstruction
static/           the web GUI (plain JS + three.js, no build step)
ml/               training and inference scripts (optional, needs Python)
docs/TRAINING.md  what a 4B model on a 3070 really takes
scripts/          Windows helpers: ML setup
tools/            STEP test-part generator, preview freezer, SolidWorks macro
tests/ui_contract.rs  the interface contract: ids exist, routes match, no network
run.cmd, run.sh   start it and open the browser
```

## Tests

```sh
cargo test
```
