# cadforge

A CAD-shaped web studio for training small models that **generate parametric
CAD**: you show it shapes and the operations that produced them, it learns to
produce the operations for a new shape from the shape alone.

The plan lives in `ARCHITECTURE.md`. This file is how to run it.

## Run it

```sh
cargo run                      # http://localhost:8787
cargo run -- --port 9000       # another port
cargo run -- --data /tmp/cf    # another storage dir (default: ./data)
```

Then open http://localhost:8787.

## Use it

1. **Upload** a part - drag files onto the viewport, or press *Upload* in the
   Artifacts panel. STL renders straight away; STEP/STP is stored with its
   build history.
2. Click an artifact to load it into the viewport; orbit with the mouse.
3. Press *Register* under **Base models** to point the studio at a model.
4. Tick artifacts, then *New dataset from selected*.
5. Pick a model and a dataset, press **Start training run**, and watch the
   stages, progress bar and live log on the right.

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
src/stl.rs        STL parser (ASCII + binary)
src/step.rs       STEP (ISO 10303-21) reader + build-history reconstruction
static/           the web GUI (plain JS + three.js, no build step)
tools/make_step.py  generate STEP test parts
tests/fixtures/   generated parts used by the tests
```

## Tests

```sh
cargo test
```
