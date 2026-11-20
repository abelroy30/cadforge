# cadforge — how to use it

Everything you can do with the studio, in order, with the exact commands. For
what the words mean see `CONCEPTS.md`; for what each file is see `GLOSSARY.md`.

---

## 1. What you need

To **use** the studio:

* Windows 10/11, or macOS/Linux.
* Nothing else. The web interface, the CAD readers, the rebuild engine and the
  sample documents are all in the folder. Nothing is downloaded at runtime.

To **build** it (once):

* Rust, from <https://rustup.rs>. On Windows the installer offers the Visual
  Studio C++ build tools — accept them, they are required to link.

To **train a model** (optional):

* Python 3.10+ and an NVIDIA GPU with the driver installed. `scripts\setup-ml.cmd`
  installs the rest.

To **read the real feature tree** (optional):

* A machine with SolidWorks, for the export macro. Everything else works without
  a licence.

---

## 2. Start it

| Platform | Command |
|---|---|
| Windows | double-click `run.cmd` |
| macOS / Linux | `./run.sh` |
| any, by hand | `cargo run` |

The first run builds the studio (a few minutes); after that it starts in a
second and **opens your browser itself** on <http://localhost:8787>.

After the first build, `target\release\cadforge.exe` *is* the studio — pin it to
the taskbar if you like.

### Options

```sh
cargo run -- --port 9000            # -p  another port
cargo run -- --data /tmp/cf         # -d  another storage directory (default ./data)
cargo run -- --static ./static      # -s  where the web files live
cargo run -- --emit-samples samples # write sample documents and exit
cargo run -- --help                 # -h
```

### See the interface without running anything

`tools/make_preview.py` freezes a running instance into one self-contained HTML
file — the real interface, real data:

```sh
tools/make_preview.py --out preview/cadforge-preview.html
```

It is a photograph, not the live app: buttons that change things do not reach the
backend. It is useful for showing someone what it looks like.

---

## 3. The interface

Three columns: what you uploaded on the left, the 3D viewport in the middle, the
inspector and the training console on the right. Each side column is split into
tabs, so one job is on screen at a time - **Files / Activity / Runs** on the
left, **Inspect / Rewards / Train** on the right - and every control says what it
does on hover.

**Left — Files & artifacts.**

* **Add files** — three tiles, and they mean different things:
  * **SolidWorks / CAD** (`.sldprt`, `.sldasm`, `.slddrw`)
  * **STEP / STP** — neutral CAD
  * **Images** — pictures of finished parts
* **Processing** — every file going `queued → uploading → done`, or an error.
  `Clear` empties the feed.
* **Artifacts** — everything you have uploaded. Filter chips (`All`, `SolidWorks`,
  `STEP`, `Images`, `Views`, `Drawings`, `Other`), a checkbox per row, `×` to
  delete, and *New dataset from selected*.
* **Datasets** and **Runs** — what you have built and what has been run.

**Middle — viewport.**

* Orbit with the mouse, and the toolbar has `Shaded` / `Wire` / `Both`, **Fit**,
  a grid toggle, **Capture views**, and **Export drawing**.
* Drop files anywhere here and each one's lane is read from its extension —
  this is the way to add a mixed batch in one go.

**Right — inspector and training** — *Inspect* (the inspector and Picture to
steps), *Rewards*, and *Train* (the models, the run, and Deploy).

* The inspector shows *How this was processed* for whatever artifact you clicked:
  every stage from upload to what was read out of the file, the reconstructed
  build steps, and for SolidWorks documents the parameters, the references and
  the stream table.
* **Picture to steps** — drop a picture, press *Run*, get the build steps.
* **Base models** — `Register` to point the studio at a model, and `×` on a row
  to remove one (refused while a run is still using it, and it says so).
* **Training** — machine capabilities, model and dataset pickers, *Start training
  run*, a progress bar, the stage checklist, the live log and *Cancel*.

---

## 4. Workflows

### 4.1 See it work with no CAD files at all

The studio can write its own samples — two parts, an assembly and a drawing, each
a real compound file carrying parameters, a preview, and for the assembly the
parts it is made of:

```sh
cargo run -- --emit-samples samples
```

Upload all four in the studio. They are read exactly as a genuine SolidWorks file
is, so the whole pipeline is visible without a licence.

### 4.2 Upload a real batch

1. Drop everything on the viewport, or drop file types onto the matching tile.
2. Watch the **Processing** feed: each file flips to `done` as the server finishes
   reading it, and appears in **Artifacts** at the same moment (that is the asset
   event stream, not a poll).
3. Click an artifact to load it and read what was found.

### 4.3 Get the *real* feature tree out of SolidWorks

This is the ground-truth path, and it is the highest-value thing to do if you
have SolidWorks.

1. In SolidWorks: **Tools > Macro > New**, save as `ExportFeatureTree.swp`, then
   paste `tools/solidworks/ExportFeatureTree.bas` into the editor.
2. Open the part and run the macro. It writes `<DocumentName>.features.json`
   beside the model.
3. Upload that JSON. The inspector shows the modelling history **as recorded**,
   not inferred from geometry.

There is an optional bulk mode — put a folder in `SWBATCH_ROOT` at the top of the
macro and it walks every part and assembly underneath.

### 4.4 Capture the SolidWorks views

1. Install `tools/solidworks/CaptureViews.bas` the same way.
2. Run it on an open part. It writes `-front.png`, `-top.png`, `-right.png` and
   `-iso.png` beside the model.
3. Upload them to the **Images** tile.

**Why the names matter.** The studio pairs a picture with its part by name:
strip the extension, then strip a trailing view word. `plate-front.png` becomes
`plate`, which pairs with `plate.SLDPRT`. The names are exactly the ones the
studio knows, which is why the isometric view is written as `-iso.png` and never
`-isometric.png`. Rename a capture and it silently stops pairing.

### 4.5 Capture views from inside the studio

Load any part and press **Capture views**. The studio renders front, top, right
and isometric offscreen on a transparent background, adds them to Artifacts, then
composes them into a **drawing sheet with a title block** and adds that too.
**Export drawing** lays out the sheet without the individual views.

Note what this is and is not: it renders the geometry *you loaded*, in the
browser. It is not a screenshot of a SolidWorks window — for that, use the macro
above.

### 4.6 Read a STEP part

Upload a `.step`/`.stp` and click it. No CAD kernel is required. The inspector
shows the topology counts, the unit, and the build steps recovered from the
B-rep: an extrude, holes, a hole pattern, fillets.

Need test parts?

```sh
tools/make_step.py plate > plate.step   # 60x40x8 plate with 4 x D6 holes
tools/make_step.py box   > box.step     # plain box
```

### 4.7 Picture to steps

1. Drop a picture into **Picture to steps** and press *Run*.
2. You get the build steps, a confidence, and the honest notes the baseline
   attaches: size is proportional (longest side = 100 mm) and depth is assumed.
3. If the picture is an artifact of your own, select it first and the result is
   tied to it.
4. Press *Rebuild this* to turn those steps into geometry and look at it.

This works with **no model installed**. That is the point: it is the baseline your
training has to beat.

### 4.8 Build a dataset and see exactly what it is worth

1. Tick artifacts in the tree, press **New dataset from selected**.
2. Look at the manifest — this is the honest answer to "is my dataset any good":

```sh
curl -s http://localhost:8787/api/datasets/<dataset-id>/export
```

The `counts` block tells you how many pictures you have, how many pairs actually
carry an operation program (`with_program`), how many are train vs held out, and
how many pictures and documents failed to pair. Its `notes` explain the pairing
rule and the split. Read `CONCEPTS.md` for what each `target` label means.

### 4.9 Run the pipeline

Pick a model and a dataset, press **Start training run**. Watch the eight stages,
the progress bar and the live log. `Cancel` stops it.

With `CADFORGE_TRAINER` unset this is the **built-in simulation** — real stages,
real timings, no GPU. It is there so the loop is visible end to end in seconds.

### 4.10 Train a model for real

Install the Python stack once:

```bat
scripts\setup-ml.cmd
```

Then either run it in the studio:

```bat
set CADFORGE_TRAINER=python ml/train_vision.py
run.cmd
```

or in a window you control, without the browser:

```bat
scripts\train-local.cmd
```

`train-local.cmd` picks the newest manifest under `runs\`, shows you every path
and default before it starts, and can be re-pointed per run:

```bat
set CADFORGE_EPOCHS=8 && scripts\train-local.cmd
set CADFORGE_MODEL=Qwen/Qwen2.5-VL-3B-Instruct && scripts\train-local.cmd
```

This does a QLoRA fine-tune (4-bit NF4, LoRA r=16) of a vision-language model,
training on the manifest's `train` pairs and scoring the held-out ones. It writes
`adapter/` and `training-report.json`, and those files appear as the run's
**outputs** in the studio.

**Read `TRAINING.md` before planning a 4-billion-parameter run.** The short
version: 8 GB of VRAM holds a 4B model in 4-bit with LoRA, at batch 1 and short
sequences, with little room to spare—and the model learns the output format and
the shapes it saw, not engineering intent.

### 4.11 Use your trained model

```bat
set CADFORGE_VISION=python ml/predict.py
set CADFORGE_ADAPTER=checkpoints\local\adapter
run.cmd
```

Now *Picture to steps* asks your model first. If it fails, the studio falls back
to the geometric baseline and says so in the result notes — so the panel never
stops working.

---

### 4.12 Point at a folder instead of uploading

Uploading is for one or two files. A library belongs in a folder, and the studio
indexes it **where it is**: nothing is copied, so pointing at 50 GB costs
nothing and your files are never touched.

1. In **Data folders**, paste the **absolute** path of a folder of parts,
   drawings and pictures. A relative path is refused, with the reason: it would
   mean a different folder every time it was scanned.
2. Press **Add folder**. The scan starts at once and reports as it goes - the
   walk, then every file it reads - in the **Activity** feed.
3. Up to **ten** folders. Each row shows what the last scan found, with buttons
   to re-scan it and to remove it.

Removing a folder forgets its artifacts; it does not delete anything on disk,
which is the whole point of indexing rather than importing. Files nothing here
can read (notes, archives) are counted as *skipped* rather than failing the
scan, and a file that disappears stops being an artifact on the next scan.
`CADFORGE_SCAN_LIMIT` caps how many files one scan will look at.

### 4.13 Watch one file being read

**Open one part or drawing...** takes a single file, reads its lane from the
extension, and shows the extraction in the **Inspector** as a tree: the header,
the B-rep summary, and the operations reconstructed from it, each with where its
value came from.

### 4.14 Tune the reward before spending hours on it

**Rewards** is what a training run is paid for. Every term has a weight and a
switch, and switching one off keeps its settings but stops it voting.

**Try it on a part** scores the studio's own reading of one artifact and shows
each term's raw measure, its score and its contribution. What you see is what
the trainer will be optimised against, because the set travels with the
manifest.

> A score of 100% here is expected and means little on its own: the reading is
> being scored against itself. The number becomes informative when it is scoring
> a *different* program, which is what training does.

### 4.15 Run the loop, and watch it converge

`Train` with `CADFORGE_TRAINER="python ml/train_grpo.py"` runs the reinforcement
loop instead of the supervised fine-tune in 4.10. Its panel shows the reward
curve as points arrive, and under it the trainer's own verdict in words -
`converged: the reward settled at ...`, or the reason it stopped without doing so.
The weights in that number come from the **Rewards** tab, and a weight changed
there travels into the run's manifest, so the trainer is paid exactly what you
set.

| Variable | Meaning | Default |
|---|---|---|
| `CADFORGE_TRAINER` | command that runs a training job | unset (learning is idle) |
| `CADFORGE_STUB` | `1` runs the loop with a stand-in policy and no model | `0` |
| `CADFORGE_STUDIO` | base URL the trainer calls back to | the studio's own |
| `CADFORGE_MAX_STEPS` | ceiling on optimiser steps | `400` |
| `CADFORGE_MAX_EPOCHS` | ceiling on passes over the pairs | `40` |
| `CADFORGE_PATIENCE` | steps the plateau test waits for an improvement | `8` |
| `CADFORGE_EPSILON` | improvement the plateau test requires | `0.01` |

To exercise the whole chain with no graphics card - it scores, climbs the
curriculum, hides a dimension and converges, and trains nothing:

```bat
set CADFORGE_TRAINER=python ml/train_grpo.py
set CADFORGE_STUB=1
run.cmd
```

A run that reaches a ceiling without improving is recorded as **not converged**
and says why. That distinction is the point: a finished run is not automatically
an achievement.

### 4.16 Deploy a finished run

Deploying makes a finished run's adapter the thing that answers **Picture to
steps**. Without it a trained model sits in a folder while the panel keeps using
the geometric baseline, which is a quiet way of training something nobody uses.

1. Finish a run that wrote an adapter. Its row in **Runs** grows a **Deploy**
   button - the studio offers the button only when there is something to serve.
2. Press it. The Training card then reads `serving <run> · <base model>`.
3. **Undeploy** stops serving it and the studio goes back to its own reading.

The studio refuses to deploy a run that has not finished, and one that trained
nothing - naming what the run actually wrote, so `report.json` reads as *a report
and no weights* rather than as a mysterious failure. `scripts/train-local.cmd`
writes `adapter/`; `ml/train_grpo.py` with `CADFORGE_STUB=1` writes none, which is
why a stub run cannot be deployed.

## 5. Driving the API directly

Every route is JSON. Uploads are the raw body with the name in the query string,
which is what lets the browser stream a file straight through.

```sh
# everything the UI needs
curl -s localhost:8787/api/state

# upload a file
curl -X POST --data-binary @plate.step \
  'http://localhost:8787/api/assets?name=plate.step&lane=step'

# what was read out of it (use the id from the response above)
curl -s localhost:8787/api/assets/<id>/history

# the SolidWorks stream catalogue
curl -s localhost:8787/api/assets/<id>/streams

# the machine: GPU, Python, training stack
curl -s localhost:8787/api/system

# the pipeline and its stages
curl -s localhost:8787/api/trainer

# watch runs live (Ctrl-C to stop)
curl -N localhost:8787/api/events
```

Creating things takes a JSON body:

```sh
curl -X POST localhost:8787/api/models \
  -H 'content-type: application/json' \
  -d '{"name":"Qwen2-VL-2B","kind":"base","params":2000000000,"notes":""}'

curl -X POST localhost:8787/api/datasets \
  -H 'content-type: application/json' \
  -d '{"name":"shop parts","asset_ids":["<id>","<id>"]}'

curl -X POST localhost:8787/api/runs \
  -H 'content-type: application/json' \
  -d '{"base_model_id":"<id>","dataset_id":"<id>"}'

curl -X POST localhost:8787/api/runs/<id>/cancel

# removing things again. A model or a dataset a run is still using is refused,
# with the reason in the error body, so cancel the run first.
curl -X DELETE localhost:8787/api/models/<id>
curl -X DELETE localhost:8787/api/datasets/<id>
curl -X DELETE localhost:8787/api/assets/<id>
```

`/api/predict` takes **raw RGBA pixels** as the body with `?w=` and `?h=` — the
browser has already decoded the image, so the studio carries no image decoder.
That makes it awkward to call by hand and easy from a browser or a script that
has the pixels; use the *Picture to steps* panel unless you are writing one.

Errors are always `{"error": "..."}` with a real HTTP status.

Folders, the work stream, storage and rewards:

```sh
# folders: add, re-scan, remove. Indexed in place - nothing is copied.
curl -s -X POST localhost:8787/api/sources \
  -H 'content-type: application/json' -d '{"path":"/home/me/parts"}'
curl -s -X POST localhost:8787/api/sources/<source-id>/scan
curl -s -X DELETE localhost:8787/api/sources/<source-id>

# watch folders and files being read, one event per file (Ctrl-C to stop)
curl -N localhost:8787/api/work/events

# what the data and the models weigh
curl -s localhost:8787/api/storage

# the reward a run is paid with, and what one program would score. This is the
# endpoint the trainer itself calls, so it is the honest way to check what a
# reward pays before spending hours on it.
curl -s -X POST localhost:8787/api/rewards/preview \
  -H 'content-type: application/json' -d '{"asset_id":"<id>"}'
curl -s -X POST localhost:8787/api/rewards/score \
  -H 'content-type: application/json' \
  -d '{"asset_id":"<id>","ops":[{"kind":"extrude","depth":100.0}]}'
curl -s -X PUT localhost:8787/api/rewards/<id> \
  -H 'content-type: application/json' -d '{"weight":2.5}'
curl -s -X PUT localhost:8787/api/rewards/<id> \
  -H 'content-type: application/json' -d '{"enabled":false}'

# serving: deploy a finished run, ask what is serving, stop serving
curl -s -X POST localhost:8787/api/runs/<id>/deploy
curl -s localhost:8787/api/deployment
curl -s -X DELETE localhost:8787/api/deployment
```

Scoring takes a program as well as an artifact, which is what makes it useful:
score the studio's own reading and you are comparing it with itself; score a
*different* program and the per-term numbers start to mean something.

## 6. Configuration

### Studio

| Variable | Default | Meaning |
|---|---|---|
| `CADFORGE_PORT` | `8787` | listen port |
| `CADFORGE_DATA` | `data` | uploads, `db.json`, run manifests |
| `CADFORGE_STATIC` | `static` | web files directory |
| `CADFORGE_NO_OPEN` | unset | set it and the browser does not open by itself |
| `CADFORGE_TRAINER` | unset | external trainer command; unset = built-in simulation |
| `CADFORGE_VISION` | unset | external picture-to-steps command; unset = geometric baseline |
| `CADFORGE_ADAPTER` | unset | adapter the vision command should load |
| `CADFORGE_PYTHON` | auto | the Python to probe for the training stack |
| `CADFORGE_VAL_PCT` | `10` | percent of pictures held out, clamped to 5–50. Read **where the manifest is built**, i.e. by the studio, not by the trainer |
| `RUST_LOG` | `info` | log level |

### Trainer (`ml/train_vision.py`)

| Variable | Default | Meaning |
|---|---|---|
| `CADFORGE_DATASET` | — | path to `dataset.json` (required) |
| `CADFORGE_OUT` | `checkpoints` | where the adapter and report are written |
| `CADFORGE_DATA` | `""` | data root, to resolve the pair image paths |
| `CADFORGE_MODEL` | `Qwen/Qwen2-VL-2B-Instruct` | base model |
| `CADFORGE_EPOCHS` | `3` | epochs |
| `CADFORGE_LR` | `2e-4` | learning rate |
| `CADFORGE_MAXLEN` | `1024` | max sequence length |
| `CADFORGE_BATCH` | `1` | batch size |
| `CADFORGE_GRAD_ACCUM` | `16` | gradient accumulation |
| `CADFORGE_LORA_R` | `16` | LoRA rank |

### Set by the studio on the trainer

You do not set these; they are how the studio hands work over.

| Variable | Value |
|---|---|
| `CADFORGE_RUN` | the run, as JSON |
| `CADFORGE_DATASET` | path to the manifest it just wrote |
| `CADFORGE_OUT` | the run's output directory |
| `CADFORGE_DATA` | the studio's data root |

### Set by the studio on a vision command

| Variable | Value |
|---|---|
| `CADFORGE_IMAGE` | path to the staged raw RGBA pixels |
| `CADFORGE_WIDTH`, `CADFORGE_HEIGHT` | the picture's size |

---

## 7. Where your files go

Everything lives under `CADFORGE_DATA` (default `./data`):

```
data/
  db.json                     the whole database, written atomically
  assets/<id>.<ext>           every uploaded file, under its UUID
  assets/<id>.preview         a preview image extracted from a document
  rebuilds/                   a FreeCAD rebuild's macro and its exported part
  runs/<run-id>/dataset.json  the manifest handed to the trainer
  runs/<run-id>/...           whatever the trainer wrote there
```

To start over, point `--data` at a new folder. To back up, copy the folder — it
is all of it.

---

## 8. Troubleshooting

| What you see | What it means |
|---|---|
| *"Rust is not installed, and it is needed once to build the studio."* | Install from <https://rustup.rs> and run `run.cmd` again |
| Build fails with a linker error | Accept the Visual Studio C++ build tools: re-run `rustup-init.exe` and take them |
| *"static dir … does not exist"* | Run from the repository root, or pass `--static` |
| The page is blank or unstyled | `static/` is missing or wrong; the vendored `three.js` is part of it |
| A SolidWorks upload says the feature tree is *listed but not decoded* | Correct and intentional. `ThirdPv`'s encoding is proprietary. Use the export macro for the real tree |
| A `.features.json` yields nothing | The macro wrote an empty tree; the studio says *this export was empty* rather than pretending it is corrupt |
| A run is refused with *"nothing in this dataset carries an operation program"* | The dataset holds no STEP part and no SolidWorks export, only pictures. A picture pairs with a part; on its own it has nothing to learn from |
| A run is refused with *"all of them fell on the held-out side of the split"* | Too few parts for the split to leave anything to train on. Add parts, or lower `CADFORGE_VAL_PCT` |
| A run finishes with *"not converged"* | It hit a ceiling - steps, epochs or time - without the reward settling. Usually the reward does not yet tell good programs from bad; check it on a real part in **Rewards** before spending another run |
| The reward curve stops improving but does not converge | That is the honest outcome, not a fault. Edit a weight in **Rewards** so the terms disagree more usefully, then run again |
| Pictures never pair with their part | The name rule. `plate-front.png` pairs with `plate.*`; `plate-isometric.png` does not |
| *"no freecadcmd on PATH"* / the macro was generated but not run | FreeCAD is not installed. This is reported, never silently ignored; the native engine still rebuilds |
| The training card says Python or torch is missing | Run `scripts\setup-ml.cmd` |
| *"no held-out pairs, so this run has no honest score"* | Your dataset is small enough that the 10% split held nothing back. Add parts, or lower `CADFORGE_VAL_PCT` |
| *"warning: under 8 GB of VRAM…"* | Lower `CADFORGE_MAXLEN` (768 or less) and keep `CADFORGE_BATCH=1`; see `TRAINING.md` |
| *"only a run that finished successfully can be deployed"* | The run is still going, or it failed. Wait, or cancel it |
| *"this run produced no adapter to deploy - it wrote report.json"* | The run trained nothing to serve. That is the honest answer rather than a fault: a stub loop and a real run that failed late both end here |
| A run finishes with no outputs | The trainer wrote nothing into `CADFORGE_OUT`; the studio warns about this rather than passing it as success |
| *"…is still running against that model - cancel it first"* | A live run is using it. `Cancel` the run, then delete |
| A run finishes, then its model or dataset vanishes from the pickers | It was deleted after the run; the run's own record is kept, but nothing can be started against it again |

If a run hangs, the log and the run's `stage` tell you where. The pipeline is
covered by a test that fails if a run stops dead rather than merely erroring.

---

## 9. Checking your install

```sh
cargo test
```

112 tests: the unit tests beside the code, plus `tests/ui_contract.rs`, which
checks the interface contract without a browser — every element id the script
reaches for exists, every URL it calls is a route the server serves, and the page
is completely offline. It also checks that every tab on the page reaches a card
and every button explains itself on hover. If you change the UI or the API, that test is what tells
you what you broke.

`cargo fmt --check` should be clean.
