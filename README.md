# cadforge

A CAD-shaped web studio for training small models that **generate parametric
CAD**: you show it shapes and the operations that produced them, it learns to
produce the operations for a new shape from the shape alone.

The plan lives in `ARCHITECTURE.md`. This file is how to run it.

## Run it

**Windows:** double-click `run.cmd`. It checks for Rust, starts the studio and
opens the browser. Nothing else to install.

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

1. **Upload** - drag files onto the viewport (or press *Upload*, or drop a whole
   folder): many at once is fine. The **Processing** panel shows each file as it
   goes - queued, uploading, done - and every artifact lands in the Artifacts
   list the moment the server has read it, without a refresh.
2. Click an artifact to load it; orbit with the mouse. The inspector shows a
   **How this was processed** tree: every stage from the upload to what was read
   out of the file.
3. Press *Register* under **Base models** to point the studio at a model.
4. Tick artifacts, then *New dataset from selected*.
5. Pick a model and a dataset, press **Start training run**, and watch the
   stages, progress bar and live log on the right.

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

## Train for real

The pipeline ships with a simulation so the whole loop is visible. To run your
own trainer instead, set `CADFORGE_TRAINER` - each run executes that command and
streams its output into the run log. The run is passed as JSON in
`CADFORGE_RUN`:

```sh
CADFORGE_TRAINER="python3 trainer/train.py" cargo run
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
| `CADFORGE_DATA` | `data` | uploads and db.json |
| `CADFORGE_STATIC` | `static` | web GUI directory |
| `CADFORGE_TRAINER` | unset | external trainer command |
| `RUST_LOG` | `info` | log level |

## Layout

```
src/main.rs       server, options, static files
src/api.rs        HTTP API + SSE
src/model.rs      types and persistence
src/pipeline.rs   the training pipeline and its stages
src/ole.rs        OLE2 compound-file reader (+ writer, for tests and samples)
src/sldworks.rs   SolidWorks documents: streams, preview, parameters, references
src/propset.rs    OLE property-set reader (the document's real parameters)
src/swhist.rs     SolidWorks feature-tree export -> build steps
src/stl.rs        STL parser (ASCII + binary)
src/step.rs       STEP (ISO 10303-21) reader + build-history reconstruction
static/           the web GUI (plain JS + three.js, no build step)
tools/make_step.py  generate STEP test parts
tools/make_preview.py  freeze a running instance into one HTML file
tools/solidworks/ExportFeatureTree.bas  export the real tree from SolidWorks
run.cmd, run.sh   start it and open the browser
tests/fixtures/   generated parts used by the tests
```

## Tests

```sh
cargo test
```
