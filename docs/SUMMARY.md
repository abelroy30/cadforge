# cadforge — project summary

Where the project stands: what works, what has been verified and how, what has
*not* been verified, and what is known to be wrong or missing.

Last reviewed: November 2026.

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

## The bug that mattered most, and what it turned out to be

The score reported a `max_gap` of about 7.8 mm on a plate rebuilt from its own
geometry, while the mean distance was 0.02 mm. `ARCHITECTURE.md` guessed the cause
was triangle *centroids* belonging to equivalent triangulations of the same quads,
i.e. a harmless artefact of sampling. **That guess was wrong.**

The real cause was two fans that did not close their loops:

```rust
for w in points.windows(2) {        // src/solid.rs, and again in src/step.rs
    push_tri(&mut out, c, w[0], w[1]);
}
```

`windows(2)` stops one pair short, so the wedge between the **last** vertex and
the **first** was never swept, in *either* the native engine or the STEP reader.
Every planar face had a hole in it. On the plate that was exactly 6 missing
triangles — 274 instead of 280 — and the 7.8 mm was the distance across the gap,
not an artefact. The old metric hid it: its unweighted mean over sample *points*
saw 5 bad samples out of 544 and reported 0.02.

Two fixes:

1. Both fans now close the loop, skipping only a genuine zero-width edge.
2. The metric measures the distance from a sample to the other *surface* — exact
   point-to-triangle distance — instead of to the nearest sample point, and weights
   the mean by triangle area. A different triangulation of the same face now scores
   exactly zero; it used to be punished for being cut differently.

The plate round trip used to report `chamfer 0.40, max gap 7.20`. The same test now
asserts, and passes, that **no point of the rebuild is more than 0.01 mm from the
original**. `max_gap` is a usable training signal for the first time.

This is the kind of failure that would have quietly poisoned training: the signal
said a correct rebuild was wrong.

---

## What works

| Piece | State |
|---|---|
| Rust server (axum) + JSON API + two SSE streams | working, exercised over HTTP |
| Web interface, 3D viewport, orbit camera | working, **not clicked in a browser** here |
| Upload into three lanes, live per-file status | working |
| Removing artifacts, **and now models and datasets** | working, live-checked |
| STL reader (ASCII + binary) | working |
| STEP reader: entity graph, topology, units | working |
| STEP history: extrude, holes, hole pattern, fillet inferred from the B-rep | working, tested against generated parts |
| STEP preview: planar faces triangulated, holes as strips | working, faces now fully covered |
| Operation program (versioned, executable) | working, schema pinned by test |
| Native rebuild engine | working; no boolean subtraction, cannot fillet, says so |
| FreeCAD engine: emits a macro, runs it where FreeCAD exists | macro generation tested; **execution untested** |
| Scoring: symmetric Chamfer distance + true max deviation | working, tested |
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
| Narrow-window layout | media queries added; **not seen in a browser** |

---

## How it is verified

**112 automated tests** (`cargo test`): 100 unit tests beside the code, and 12
interface-contract tests in `tests/ui_contract.rs`. `cargo fmt --check` is clean
and the Python scripts compile.

They cover:

* the STEP round trip — a part is parsed, turned into a program, rebuilt
  natively, triangulated and scored, with `chamfer` **and** `max_gap` now both
  required to be under 0.01 mm, so a gap in a face fails the test;
* that a face is *fully* triangulated, checked by **surface area** — a missing
  wedge loses area and a doubled one gains it, where counting triangles would
  notice neither;
* the metric's properties: identical shapes score zero, a different triangulation
  of the same face scores zero, a uniform offset is measured exactly, a distant
  point still gets a real distance, empty input has no score;
* the operation program: the exact JSON discriminants a model is trained on, the
  defaults for omitted fields, and that validation refuses each thing a driver
  could not build (and accepts a through hole with no depth);
* the native engine: thickness as asked, a fillet reported rather than dropped,
  drilling before anything exists reported, one wall per pattern position, and the
  convex hull dropping points inside or on an edge;
* the vector helpers, including that `perpendicular` is genuinely perpendicular
  and unit length for every axis it is handed;
* the manifest: pairing, counts, provenance, and that a picture's train/val side
  cannot move when the dataset is re-exported in a different order;
* that a 10% split actually holds pictures back, over 2000 ids (a hash that always
  answered `train` would otherwise pass every other check while meaning nothing is
  ever evaluated);
* a full simulated run reaching the end rather than deadlocking, and reporting
  what the dataset is made of and how the pairs split;
* that an external trainer's output file is reported as a run output;
* the interface contract: every element id the script reaches for exists, every
  URL it calls is a real route, ids are unique, every lane the studio can use has
  a filter chip, the page is completely offline, and the scripts load in order.

The work in this pass was also exercised **live over HTTP**: a run was started and
both deletions were refused with a reason while it was going, then accepted after
it finished, with a repeat delete returning a clean 404 and the run's own record
surviving.

---

## What is NOT verified

Stated plainly, because these are the things most likely to break first:

1. **The interface has never been clicked in a real browser.** There is no browser
   in the development environment. It is covered by a contract test and a syntax
   check — every id, route, lane and string is right — but no one has seen it
   render, dragged a file onto a tile, or pressed *Capture views*. **This is the
   first thing to exercise on a Windows machine.**
2. **The narrow-window media queries have never been seen.** They were written
   from the stylesheet, not observed on a screen.
3. **`tools/solidworks/CaptureViews.bas` has never been executed.** No SolidWorks
   here. It is written against documented APIs, it verifies each file it claims to
   write is non-empty, and it reports failure rather than leaving a broken
   capture — but the `swStandardViews_e` ids are unconfirmed.
4. **`src/freecad.rs`'s `run()` has never run.** There is no `freecadcmd` here.
   Macro generation is tested; the invocation, and every real cut and fillet, is
   not.
5. **No model has been trained on real data.** The trainer is written and its
   manifest contract is tested; a GPU run on a shop's own parts has not happened.
6. **No real `.SLDPRT` has been read.** The reader is tested against files built
   by the project's own writer, which is the honest limit of testing a proprietary
   format without the program.

---

## Known issues and gaps

1. **`static/app.js` has no tests beyond the contract test.** It is a plain script
   with globals, no modules and no type checking. The contract test catches wrong
   ids and routes, not wrong behaviour.
2. **`ml/predict.py`'s confidence is structural, not calibrated** — 0.6 when the
   answer parsed into operations, 0.1 when it had to be salvaged into an empty
   program. Nothing in that script can produce a real probability, and the value
   says so in the result notes.
3. **A live run cannot be deleted, only cancelled.** Deleting a model or a dataset
   a queued or running run is using is refused (with the reason), which is
   deliberate: the run would have nothing to record against. There is no endpoint
   to delete a *run* at all, so history accumulates.
4. **The release workflow is not active.** `.github/workflows/release.yml` is
   written and committed, but the push is refused: the token here lacks the
   `workflow` scope, which GitHub requires for that path specifically. It sits in
   an unpushed local commit, so `origin/master` does not have it. It goes live
   with one `git push` from a machine that has the scope, or by pasting the same
   file (`docs/release-workflow.yml`) into the web editor.
5. **Deleting an artifact does not clean up datasets that reference it.** A
   dataset is a list of ids and keeps the dead id; the manifest skips an asset it
   cannot find rather than failing.
6. **No `panic = "abort"` in the release profile** even though it would shrink the
   binary further: it is incompatible with `cargo test --release`, which the
   release workflow runs.

---

## Data folders and rewards

Added after the sections above were written, and verified live over HTTP:

* **Folders instead of a bulk upload** (`src/ingest.rs`, the *Data folders*
  panel). A source is an index, never a copy: files are read where they live, so
  pointing at a large library costs nothing and removing a source never touches
  the user's files. Up to ten, each re-scannable. Verified: adding, scanning,
  the ten-folder cap, and that removing a source leaves the files on disk.
* **The work, watched.** Every folder walk and every file read is streamed as it
  happens, so the *Activity* panel shows work rather than a spinner.
* **Sizes, labelled by how they are known.** Data is measured; a base model's
  size is estimated from its parameter count, because the studio never downloads
  weights; adapter sizes are measured from what runs wrote.
* **Rewards as data** (`src/reward.rs`, the *Rewards* panel). Four kinds, each
  weighted and switchable, with a preview that scores one part and shows the
  working. Validity is a gate rather than a weighted term; a reward with nothing
  to compare against abstains and says so; and the set travels with the
  manifest, so a finished run can be explained after the rewards are edited.

All three of those are now built: the loop and its convergence test are in
*The training loop* below, and Deploy and the tabbed panels are in *Deploy, and
the panels*.

One caveat worth stating plainly: the reward preview scores the studio's own
reading of a part against itself, so a perfect score there is expected. The
number becomes informative when it is scoring a different program, which is what
training does - and until a real run has happened, no reward has been tested
against a model.

## The training loop

`ml/train_grpo.py` is the loop, and the studio runs it as an external trainer.
Verified end to end against a stand-in policy (`CADFORGE_STUB=1`), which exercises
scoring over HTTP, the curriculum, the hidden dimension, the convergence test,
the progress protocol and the curve in the interface - and trains nothing:

```text
status: succeeded | stage: Converged
note: converged: the reward settled at 0.460 after 4 step(s)
curve points: 4
```

The complementary case was seen as well: a run that reaches its ceiling without
improving is not recorded as converged, and its note says why.

**What is not verified, and cannot be from here:** no model has ever been
trained. There is no GPU and no torch stack in this environment, so every part of
the model path - loading a Qwen VL model, generating candidates, the update step -
is untested against real weights. The loop's plumbing is tested; its learning is
not.

**Also not implemented:** full GRPO's PPO ratio and KL term. Group-relative
advantages are real, so the verified reward does drive the update, but the update
is advantage-weighted supervised training rather than ratio-based policy
optimisation. The report a run writes states this.

## Deploy, and the panels

**Deploy** makes a finished run the thing that answers *Picture to steps*. A run
row grows a **Deploy** button only when the run wrote something named like an
adapter, and the Training card shows what is being served, with **Undeploy**
beside it. Verified live in both directions:

```text
run that wrote adapter/weights.bin:  deploy 200, GET {"deployed":true, ...}
                                     undeploy 204, then 400 "nothing is deployed"
run that wrote only report.json:     400 "this run produced no adapter to deploy -
                                     it wrote report.json. Nothing was trained to serve."
```

**The panels are tabbed.** Six cards down each side became *Files / Activity /
Runs* and *Inspect / Rewards / Train*, so one job is on screen at a time. A card
belongs to a tab by an attribute, so the tabbing is one function rather than
per-card wiring, and the contract test checks that every tab reaches a card and
that every button explains itself on hover.

Still not verified: no adapter has ever answered a picture, because none has
been trained here.

## The model path, and three bugs in it (part 2)

Writing `docs/MODELS.md` meant reading the trainer's model path closely, and it
had three defects that would have made the guide a lie:

1. **The RL loop never updated a real model.** `policy.learn` was called only
   inside `if stub:`, so a run with actual weights loaded the model, sampled
   candidates, scored them, and then saved an untouched adapter. It was a loop
   that evaluated its policy and never changed it. Both policies now take the
   same `learn(advantages)`, and the call is unconditional.
2. **`Qwen2_5_VLForConditionalGeneration` was hardcoded** while the default model
   was `Qwen/Qwen3-VL-4B-Instruct` - a different architecture. Loading one with
   the other's class fails, or worse, builds something that runs and is not what
   was asked for. `load_vlm` now tries the generic image-text auto-class, then
   named classes, prints the class it resolved to, and on failure lists every
   class it tried.
3. **The image never reached the model.** `_encode` called
   `apply_chat_template(...)` without `return_dict=True`, which returns token ids
   and no `pixel_values`: a vision-language model silently became a text one that
   never saw the part. And `learn` trained on the answer tokens as *input*,
   unconditioned on the prompt; it now builds prompt+answer with the picture and
   masks the prompt, so the loss is the model's own loss on the answers that were
   actually scored.

Two of these - the missing image and the missing update - are the same failure
mode this project keeps meeting: something that runs, reports success, and does
nothing. Neither is caught by any test here, because both need weights; what is
recorded instead is that the stub loop still converges after the change.

**Deploying an adapter now finds its base.** `Deployment` carries the base
model's id as well as its name, and the studio passes it to the vision command as
`CADFORGE_MODEL`. Before this, deploying a Qwen3-VL-4B adapter left `predict.py`
on its own default base - an adapter loaded onto different weights is not the
trained model, and nothing would have said so.

## Where to go next

In the order the project is blocked, not by attractiveness:

1. **Open it in a browser on Windows.** Upload the samples
   (`cargo run -- --emit-samples samples`), walk every panel, resize the window to
   check the new layout, and note anything that throws. This is the largest
   untested surface.
2. **Run `CaptureViews.bas` on a real part** and confirm the four PNGs appear and
   pair with it (`counts.pictures_without_a_document` should be 0).
3. **Train once on real data** — one epoch on ~100 pairs, then compare the model's
   answers against the geometric baseline to see what training actually bought.
   With `max_gap` now meaningful, the score is finally worth trusting as the
   measure.
4. **Decide the real-engine question**: FreeCAD first (free, scriptable) or
   SolidWorks first (where the history comes from). Still open in
   `ARCHITECTURE.md`.

---

## A note on corrections

While writing the documentation, three claims in it turned out to be wrong or
stale, and were corrected rather than carried forward:

* `ARCHITECTURE.md` described seven pipeline stages (there are eight), a route
table missing eight endpoints, and SolidWorks-native history as "not yet".
* `scripts/train-local.cmd` set `CADFORGE_VAL_PCT`, which the trainer never reads.
* This file claimed `selectAsset`'s `video` branch was unreachable. It is not: the
  branch keys on an artifact's *kind*, and a dropped `.mp4` really is classified
  `kind: video`. Videos land on the `other` *lane*, which has a filter chip.

The first summary of the `max_gap` bug in `ARCHITECTURE.md` was also a
misdiagnosis, as described at the top of this file.

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
| `docs/MODELS.md` | which model to load, fetching the weights, training and deploying it |
| `docs/SUMMARY.md` | this file |
