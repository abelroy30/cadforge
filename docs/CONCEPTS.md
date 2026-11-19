# cadforge — core concepts

The vocabulary this project uses, and the ideas the code is built around. Read
this before the glossary: the glossary says what each file does, this says what
the words mean.

---

## The one-sentence idea

You show it shapes *and* the operations that produced them; it learns to produce
the operations for a new shape from the shape alone. Everything else is plumbing
around that loop.

```
  upload parts            build dataset           run pipeline           ask for a new shape
  (STEP/SW/STL/img)  ->  (artifacts + history) -> (fine-tune a model) -> (model emits ops)
         ^                                                                       |
         |                                                                       v
         +------------------- correct / score <------------------ rebuild in an engine
```

## Artifact

A file you uploaded, plus everything the studio managed to read out of it. An
artifact is not the file — it is the file *and* the result of reading it:

| Field | Holds |
|---|---|
| `kind` | what the file is: `mesh`, `step`, `sw_part`, `sw_assembly`, `sw_drawing`, `history`, `image`, `video`, `other` |
| `lane` | which door it came through (see below) |
| `step` | for STEP files: topology counts, units, and the inferred build steps |
| `catalogue` | for SolidWorks files: the stream list, parameters, references, preview |
| `actions` | the "how this was processed" tree the inspector draws |
| `triangles`, `bbox` | for meshes, what the viewport needs |

An artifact is kept even when it could not be read. The studio says plainly that
it could not read it rather than dropping it — see the comment in `src/api.rs`:
*"Keep the file - the user uploaded it - but say plainly that it could not be
read."*

### Lane

A lane is the intent behind an upload, and it is not the same thing as the file
type. There are three doors in the UI:

| Lane | Means | Files |
|---|---|---|
| `sldworks` | a native CAD document | `.sldprt`, `.sldasm`, `.slddrw` |
| `step` | neutral exchange CAD | `.step`, `.stp` |
| `image` | a picture of a finished part | `.png`, `.jpg`, `.jpeg`, `.webp`, `.gif` |

The studio also produces two more lanes itself: `view` (a captured render) and
`drawing` (a captured sheet). Anything unrecognised lands in `other`. When you
drop a mixed batch on a tile the lane is taken from the tile; drop it on the
viewport instead and `laneFor()` infers the lane per file from its extension.

## The operation program — the central contract

This is the single most important idea in the project. A build history as prose
is not trainable and not executable, so it is expressed as a short, versioned
list of operations: a **program**. `src/ops.rs` defines it.

```json
{
  "version": 1,
  "units": "mm",
  "ops": [
    { "op": "extrude", "profile": { "shape": "circle", "center": [0,0,0], "normal": [0,0,1], "diameter": 40 },
      "distance": 8, "direction": [0,0,1] },
    { "op": "hole_pattern", "diameter": 6, "through": true, "depth": 0,
      "positions": [[12,10,0],[48,10,0],[12,30,0],[48,30,0]], "axis": [0,0,1] }
  ]
}
```

Four operations exist:

| `op` | Fields |
|---|---|
| `extrude` | `profile`, `distance`, `direction` |
| `hole` | `diameter`, `through`, `depth`, `at`, `axis` |
| `hole_pattern` | `diameter`, `through`, `depth`, `positions`, `axis` |
| `fillet` | `radius` |

A `profile` is either a `polygon` (a list of points) or a `circle` (centre,
normal, diameter).

Why it matters:

* **a model emits it** — the fine-tune's target is this JSON, nothing looser;
* **an engine executes it** — so a prediction can be turned into geometry and
  looked at;
* **a score compares it** — so "did it learn anything" is a number, not a feeling;
* **it is versioned** — `version: 1`, checked by `Program::validate`.

A program is deliberately small. It is the subset of CAD that the readers can
recover and the engines can execute, not a general CAD kernel's worth of
operations.

## Two ways a program is born

| Source | Path | Target label |
|---|---|---|
| a STEP file | geometry fingerprints → inferred operations | `reconstructed_from_geometry` |
| a SolidWorks feature-tree export | the macro's JSON → recorded operations | `recorded_steps` |
| a picture | the geometric baseline (or a model) | — |

Both STEP and the macro's JSON end up as the *same* `Summary` shape, so an
inferred history and a recorded one travel the same road: build steps → program →
rebuild → score.

**Inference is not ground truth.** `src/step.rs` reads fingerprints — two parallel
planes are an extrude, a cylindrical face is a hole, repeated holes on one axis
are a pattern, a toroidal face is a fillet. That is honest inference from
geometry, and the project labels it as such. `src/swhist.rs` is the ground truth
path, because SolidWorks hands over the tree it already knows.

## The rebuild loop, and two engines

```
Program ──┬── src/solid.rs   (native)  ──► triangles ──┐
          │                                            ├──► src/score.rs
          └── src/freecad.rs (FreeCAD)  ──► triangles ──┘
```

* **Native** (`src/solid.rs`) — no CAD kernel needed, so the round trip is
testable anywhere. It builds the same *surface model* a STEP file carries (outer
planar faces plus cylindrical hole walls). It does **not** do boolean
subtraction, and it reports fillets as unsupported rather than dropping them
silently.
* **FreeCAD** (`src/freecad.rs`) — emits a real Python macro and runs it where
  `freecadcmd` exists. This is the intended production backend; it can genuinely
  cut, fillet and export.

**Scoring** (`src/score.rs`) samples points on both surfaces and takes the mean
nearest-neighbour distance in both directions: `chamfer`, `forward`, `backward`,
`max_gap`, `samples`, `size_delta`. Symmetric on purpose — leaving a hole out is
penalised, and so is inventing material.

## Dataset, pair, manifest

A **dataset** is a named set of artifact ids. Nothing more is stored: the
manifest is derived when it is asked for.

A **pair** is one picture joined to one document, and this is how learning
happens — the picture is the input, the document's program is the target.

Pairing is by name, and the rule is explicit rather than fuzzy: strip the
extension, then strip a trailing view word, and the two names must agree.

```
plate-front.png   ->  plate   ->  pairs with plate.SLDPRT / plate.step
plate-iso.png     ->  plate
plate-isometric.png -> plate-isometric   (not a known view word: does NOT pair)
```
The known view words are `front back left right top bottom iso sheet view`.
Nothing fuzzy is guessed here, because a wrong pair teaches a model the wrong
thing.

A document with no picture, or a picture with no document, is reported in the
manifest rather than silently dropped: `documents_without_a_picture` and
`pictures_without_a_document`.

### The manifest

One JSON document, written to `runs/<run-id>/dataset.json`, is the entire
interface between the studio and a trainer. It carries the dataset name and id,
`version: 1`, the array of `pairs`, a `counts` block, the unpaired documents, and
some `notes` explaining the rules.

Each pair carries the image path (`assets/<id>.<ext>`), the image name and id,
the source document, its `steps`, its `program`, a `target` label, and its
`split`. Documents without a program still appear — with `program: null` — so the
counts can tell you how much of the dataset is actually usable.

The `target` label is how the studio stays honest about what a pair teaches:

| `target` | Means |
|---|---|
| `reconstructed_from_geometry` | a STEP document: operations inferred from the B-rep |
| `recorded_steps` | a tree exported by the SolidWorks macro: recorded, not inferred |
| `parameters_only` | a document we can describe but not turn into operations |
| `unpaired` | no document matched the picture |
| `unreadable` | a STEP file that could not be read |

## The split

Every picture is assigned `train` or `val`. The side is a **hash of the picture's
id**, not its position in a list:

* the same picture lands on the same side every time;
* re-exporting the dataset cannot leak a part from train into eval;
* reordering the dataset changes nothing.

The rate is `CADFORGE_VAL_PCT`, default 10, clamped to 5–50. Only pairs that carry
a program are counted in the train/val totals, so `counts.train + counts.val` is
the number of pairs that can actually be trained on.

## A run

A run is one attempt at training, with eight stages:

| # | Stage | Weight |
|---|---|---|
| 1 | Resolve base model | 0.05 |
| 2 | Register dataset | 0.10 |
| 3 | Pair pictures with operations | 0.10 |
| 4 | Extract feature history | 0.20 |
| 5 | Tokenise operation sequences | 0.15 |
| 6 | Fine-tune | 0.30 |
| 7 | Evaluate reconstruction | 0.15 |
| 8 | Save checkpoint | 0.05 |

The weights are the stage's share of the progress bar, so "Fine-tune" really
does dominate. Progress and log lines are pushed to the browser over SSE as they
happen.

A run has a `status` (`queued`, `running`, `succeeded`, `failed`, `cancelled`), a
`stage`, a `progress`, a `log` of timestamped lines, and `outputs` — the files it
produced.

### The two trainer seams

A run either simulates or delegates, and switching is one variable:

1. **Built in** (`DefaultTrainer`) — walks the eight stages with visible ticks so
the whole loop is observable in seconds. Used when `CADFORGE_TRAINER` is unset.
2. **External** — `CADFORGE_TRAINER="python ml/train_vision.py"` runs that
   command instead. The studio writes the manifest, then hands the trainer four
   variables and streams its output into the run log:

| Variable | Value |
|---|---|
| `CADFORGE_RUN` | the run, as JSON |
| `CADFORGE_DATASET` | path to `dataset.json` |
| `CADFORGE_OUT` | the run's output directory |
| `CADFORGE_DATA` | the studio's data root |

That is the whole contract. A script, a Python process or a compiled binary all
plug in the same way. A trainer reports its result by *writing files* into
`CADFORGE_OUT`; the studio compares that directory before and after and records
whatever appeared, under whatever names the trainer chose.

In Rust the equivalent seam is the `Trainer` trait (`name()` + `stages()`),
which is what `GET /api/trainer` reports and what the UI's checklist is drawn
from.

## Vision: a baseline, not a model

"Picture to steps" always answers, with or without a trained model:

* **Baseline** (`src/vision.rs`) — measures the silhouette, the bounding box, the
  rectangularity and the enclosed holes, then writes the operations that would
  produce exactly that shape. It states its own limits in the reply: size is
  proportional (longest side taken as 100 mm), depth is *assumed*, and it is not
  a trained model.
* **A trained model** — set `CADFORGE_VISION` and it is asked first; if it fails,
  the studio falls back to the baseline and says so in the notes.

This is deliberate: the panel always works, and a trained model has a baseline
to beat.

## Local-only, and other standing rules

The project has a few invariants that are worth knowing because they explain
apparently odd decisions:

* **Nothing leaves the machine.** No CDN, no telemetry, no runtime downloads. The
  UI vendors three.js; `tests/ui_contract.rs` fails the build if an `http(s)://`
  URL appears in the page, the script or the CSS.
* **Never invent what you cannot read.** The SolidWorks feature tree is *listed
  and named but not decoded*, because guessing would produce confidently wrong
  build histories. The same principle shows up in the STEP reader (`ref_dir`
  "unused until we reconstruct sketches") and in the property reader ("nothing is
  guessed").
* **Carry your uncertainty in the output.** Baseline predictions carry notes
  saying they are proportional and assumed; rebuilt shapes carry `warnings`;
  datasets carry counts of what could not be paired.
* **One JSON document, written atomically.** `data/db.json` is written to
  `db.json.tmp` and renamed. Fine for a single node; Postgres when this is a
  service.
* **Tolerant readers.** Real STLs have wrong normals and truncated data; the
  reader keeps what is usable instead of refusing the file.
