# cadforge — project summary

Where the project stands today: what works, what has been verified and how, what
has *not* been verified, and what is known to be wrong or missing.

Last reviewed: November 2026, at commit `77eae31`.

---

## In one paragraph

The loop is closed end to end and runs locally: upload CAD (STEP, SolidWorks
compound documents, STL, images), have the studio read what it honestly can out of
each file, build picture/operation pairs into one trainer manifest, run an
eight-stage pipeline with live progress, and — with a real trainer plugged in —
fine-tune a small vision-language model whose output is executable geometry that
can be rebuilt and scored. The CAD readers, the operation program, both rebuild
engines and the scoring metric are written from scratch with no CAD kernel
dependency. What is *not* done: verifying the interface in a real browser, running
the SolidWorks capture macro on a real part, and training on real customer data —
all three of which need a Windows machine with the tools installed.

---

## What works

| Piece | State |
|---|---|
| Rust server (axum) + JSON API + two SSE streams | working, exercised over HTTP |
| Web interface, 3D viewport, orbit camera | working, **not clicked in a browser** here |
| Upload into three lanes, live per-file status | working |
| STL reader (ASCII + binary) | working |
| STEP reader: entity graph, topology, units | working |
| STEP history: extrude, holes, hole pattern, fillet inferred from the B-rep | working, tested against generated parts |
| STEP preview: planar faces triangulated, holes as strips | working |
| Operation program (versioned, executable) | working |
| Native rebuild engine | working; no boolean subtraction, cannot fillet |
| FreeCAD engine: emits a macro, runs it where FreeCAD exists | macro generation tested; **execution untested** |
| Scoring: symmetric Chamfer distance | working, tested |
| SolidWorks compound-file reader: streams, parameters, references, preview | working |
| SolidWorks feature tree | **listed, never decoded** — by design; use the export macro |
| Macro export → recorded history → program | working |
| Dataset manifest with pairing, provenance and split | working, tested |
| Eight-stage pipeline, live progress, cancel | working |
| External trainer seam (`CADFORGE_TRAINER`) | working, tested |
| Run outputs recorded from what the trainer wrote | working, tested |
| Geometric baseline: picture → steps (+ rebuild) | working |
| Machine probe: GPU, Python, torch, CUDA | working |
| QLoRA fine-tune script + held-out scoring | written; **not run on a GPU here** |
| Windows launcher that needs nothing but Rust to *use* | working |

---

## How it is verified

**66 automated tests** (`cargo test`): 58 unit tests beside the code, and 8
interface-contract tests in `tests/ui_contract.rs`. `cargo fmt --check` is clean
and the Python scripts compile.

They cover:

* the STEP round trip — a part is parsed, turned into a program, rebuilt
  natively, triangulated and scored, asserting `chamfer < 0.2` against its own
  geometry (`rebuilds_the_plate_from_its_own_history`), with
  `a_rebuild_that_loses_the_holes_scores_worse` guarding the metric itself;
* the metric's properties: identical shapes score zero, a known displacement
  scores roughly that, empty input has no score;
* the manifest: pairing, counts, provenance, and that a picture's train/val side
  cannot move when the dataset is re-exported in a different order;
* that a 10% split actually holds pictures back, over 2000 ids (a hash that
always answered `train` would otherwise pass every other check while meaning
nothing is ever evaluated);
* a full simulated run reaching the end rather than deadlocking, and that it
  reports what the dataset is made of and how the pairs split;
* that an external trainer's output file is reported as a run output;
* the interface contract: every element id the script reaches for exists, every
  URL it calls is a real route, ids are unique, the upload tiles and filter chips
  use real lane names, scripts load in order, and the page is completely offline.

The API has also been exercised **live over HTTP** during development: upload →
parse → predict → rebuild → dataset manifest → a full simulated run → a run with
an external trainer writing outputs.

---

## What is NOT verified

Stated plainly, because these are the things most likely to break first:

1. **The interface has never been clicked in a real browser.** There is no browser
   in the development environment. The UI is covered by a contract test and a
   syntax check — that every id, route and string is right — but no one has seen
   it render, dragged a file onto a tile, or pressed *Capture views*. **This is
   the first thing to exercise on a Windows machine.**
2. **`tools/solidworks/CaptureViews.bas` has never been executed.** No SolidWorks
   here. It is written against documented APIs, it verifies each file it claims to
   write is non-empty, and it reports failure rather than leaving a broken
   capture — but the `swStandardViews_e` ids are unconfirmed. A wrong id shows up
   immediately as a wrong angle.
3. **`src/freecad.rs`'s `run()` has never run.** There is no `freecadcmd` here.
   Macro generation is tested; the invocation, and every real cut and fillet, is
   not.
4. **No model has been trained on real data.** The trainer is written and its
   manifest contract is tested; a GPU run on a real shop's parts has not happened.
5. **No real `.SLDPRT` has been read.** The reader is tested against files built
   by the project's own writer, which is the honest limit of testing a proprietary
   format without the program.

---

## Known issues and gaps

1. **Model and dataset rows in the UI are not interactive.** `renderModels` and
   `renderDatasets` draw rows with no click, rename or delete handler, so a
   registered model or a built dataset cannot be inspected or removed from the
   interface. Artifacts and runs *are* interactive. Workaround: edit `db.json`,
   or use `DELETE /api/assets/{id}` for artifacts.
2. **`ARCHITECTURE.md` had drifted** from the code: it described seven stages
   (there are eight), a route table missing about eight endpoints, and
   SolidWorks-native history as "not yet" when the export path exists. Corrected
   in this documentation pass.
3. **A no-op setting was removed from `scripts/train-local.cmd`.** It set
   `CADFORGE_VAL_PCT`, which the trainer never reads — the split is decided when
   the studio writes the manifest. It looked like it did something and did not.
4. **`score::compare` reports a large `max_gap` on the generated plate** (7.77 mm)
   while the Chamfer distance is 0.02 mm and every triangle vertex matches 
   exactly. The geometry is right; the suspicion is the sample set or the
   nearest-neighbour search. **Unresolved**, and it matters: this distance is the
   training signal, and a phantom outlier would teach a model to chase nothing.
   Carried over from `ARCHITECTURE.md`.
5. **No unit tests in `src/solid.rs`, `src/vec3.rs` or `src/ops.rs`.** The round
   trip covers the engine indirectly, but the engine's own behaviour and the
   program schema's validation are not directly tested.
6. **`static/app.js` has no tests** beyond the contract test; it is a plain
   script with globals, no modules and no type checking.
7. **The interface does not adapt to narrow screens** — a fixed `300px 1fr 380px`
   grid with no media queries.
8. **`ml/predict.py` reports a hardcoded `confidence` of 0.5.** It is a
   placeholder, not a calibrated number.
9. **No `[profile.release]` tuning** in `Cargo.toml`; only the dev profile is
   optimised.
10. **Unused details**: the sprite symbol `#i-upload` is defined but never used,
    `#vp-toolbar` carries an id nothing reads, and `selectAsset` has a `video`
    branch that no lane can currently produce.
11. **The release workflow is not active.** `.github/workflows/release.yml` cannot
    be pushed by the current token (it lacks the `workflow` scope), so the
    Windows CI that would build a downloadable zip is kept as
    `docs/release-workflow.yml` until it can be added.

---

## Where to go next

In the order the project is blocked, not by attractiveness:

1. **Open it in a browser on Windows.** Upload the samples
   (`cargo run -- --emit-samples samples`), walk every panel, and note anything
   that throws. This is the largest untested surface.
2. **Run `CaptureViews.bas` on a real part** and confirm the four PNGs appear and
   pair with it (`counts.pictures_without_a_document` should be 0).
3. **Fix `score::compare`'s `max_gap`** (issue 4) before trusting the metric as a
   training signal.
4. **Train once on real data** — one epoch on ~100 pairs, then compare the model's
   answers against the geometric baseline to see what training actually bought.
5. **Make models and datasets interactive** in the UI (issue 1).
6. **Decide the real-engine question**: FreeCAD first (free, scriptable) or
   SolidWorks first (where the history comes from). This is still an open
   question in `ARCHITECTURE.md`.

---

## Documents

| File | Covers |
|---|---|
| `README.md` | how to run it |
| `ARCHITECTURE.md` | the plan, components, roadmap, open questions |
| `docs/USAGE.md` | using it, in detail, with commands |
| `docs/CONCEPTS.md` | the vocabulary and the ideas |
| `docs/GLOSSARY.md` | every file, and the dependency map |
| `docs/PROJECT.md` | what it is, for whom, and why it is built this way |
| `docs/TRAINING.md` | what fine-tuning on a small GPU really takes |
| `docs/SUMMARY.md` | this file |
