# cadforge — project description

## What it is

cadforge is a **CAD-shaped web studio for training small models that generate
parametric CAD**. You show it shapes *and* the operations that produced them; it
learns to produce the operations for a new shape you only show the result of.

It runs entirely on your own machine. It is a Rust server with a browser
interface, a set of CAD readers with no CAD kernel behind them, a rebuild engine,
and a thin seam where a training script plugs in.

The plan and its rationale live in `ARCHITECTURE.md`. This document is the
description: what the thing is, who it is for, what is in scope, and which
decisions shaped it. For where it actually stands, see `SUMMARY.md`.

## The problem

A machine shop or a designer accumulates thousands of parts, each with a real
modelling history: this plate, extruded 8 mm, then four 6 mm holes drilled in a
pattern. That history is the valuable thing—it is what a person would need to
reproduce the part—and it is locked inside whatever tool made it.

Meanwhile the interesting question is the inverse one: given only a *shape* (a
picture of a part, a drawing, geometry without history), can something work out
how it was made?

Answering that needs three things that are usually missing from a CAD workflow:

1. a **machine-readable history** — not prose, not a screenshot;
2. a **way to check** whether a produced history is right;
3. **data**, in a form a model can learn from.

cadforge is built around those three.

## The loop

```
  upload parts            build dataset           run pipeline           ask for a new shape
  (STEP/SW/STL/img)  ->  (artifacts + history) -> (fine-tune a model) -> (model emits ops)
         ^                                                                       |
         |                                                                       v
         +------------------- correct / score <------------------ rebuild in an engine
```

Each arrow is a real component, not a placeholder:

* **upload** — readers for STEP (ISO 10303-21, written from scratch), SolidWorks
  compound documents (also from scratch), and STL.
* **history** — recovered from the STEP B-rep by fingerprint, or recorded by a
  SolidWorks macro that writes the real FeatureManager tree to JSON.
* **dataset** — picture/operation pairs, written as one manifest that is the
  entire interface to a trainer.
* **fine-tune** — a QLoRA script for a small vision-language model, or anything
  else that honours the same manifest contract.
* **rebuild** — the operations are executed back into geometry, natively or via
  FreeCAD.
* **score** — a symmetric Chamfer distance between the rebuild and the target, so
  "did it learn anything" is a number.

## Who it is for

* **A shop with a SolidWorks archive** that wants its own parts searchable and
  reproducible, and wants the history to become training data.
* **Someone training a model for CAD** who needs paired data, a rebuild driver and
  a scoring loop rather than a dataset they have to invent.
* **Someone who wants to see the loop work** before investing in a GPU: the
  built-in simulation walks all eight stages in seconds, and the baseline answers
  picture-to-steps with no model at all.

## In scope

* Reading CAD without needing the CAD program installed, where that is possible
  and honest.
* Turning recorded or inferred history into a versioned, executable operation
  program.
* Executing that program back into geometry and measuring the difference.
* Producing training data with a stated provenance for every pair.
* The training loop, the live UI for it, and the seams for a real trainer.
* Working offline, on Windows, from a double-click.

## Explicitly out of scope

* **Being a CAD program.** It does not edit models, and it is not a kernel.
* **Decoding the SolidWorks feature tree from the file.** The encoding is
  proprietary and undocumented. It is listed and named, never guessed at, and the
  real tree comes from an export through SolidWorks' own API.
* **A general B-rep kernel.** The native engine builds the surface model a part
  already has; it does not do boolean subtraction. FreeCAD is the intended
  production backend for real cutting and filleting.
* **Cloud services, telemetry, accounts, multi-tenancy.** Nothing leaves the
  machine; there is no CDN and no runtime download.
* **Being a finished product.** It has no packaging, no installer and no licence
  file; it has a launcher.

## Design decisions and why

**Rust server, plain-JavaScript interface.** Rust for the parsers, the geometry
and the orchestration; the browser needs JavaScript anyway. There is **no build
step for the UI** — plain files plus a vendored three.js — so the first run has
zero toolchain friction. A bundler can come when the UI needs one.

**No CAD kernel dependency, deliberately.** Reading STEP, reading a compound
file and rebuilding a surface are all written here rather than pulled from
OpenCASCADE. That keeps the build a single binary and lets the whole round trip
be tested on a machine with nothing installed. The cost is stated in the code:
planar faces are fan-triangulated, an inferred profile is a convex hull, and the
native engine cannot fillet. When fidelity matters more, `opencascade-rs` is the
intended route.

**One program, four operations, versioned.** A model needs a small, exact target.
The operation program is deliberately the subset that the readers can recover and
the engines can execute — `extrude`, `hole`, `hole_pattern`, `fillet` — with a
`version` field so it can grow.

**Honesty is a feature, not a disclaimer.** The project refuses to invent what it
cannot read, and it carries its uncertainty in its own output: the feature tree is
listed but not decoded; baseline predictions say their size is proportional and
their depth assumed; a rebuild carries warnings; a dataset carries the counts of
what failed to pair. A confidently wrong build history is treated as worse than
none, because it would be trained on.

**The simulated pipeline came first, on purpose.** The stages, the progress and
the live log are the contract. A real trainer drops in behind them without the
app changing — which is also why the external trainer is a shell command with
four environment variables rather than an embedded Python runtime.

**Local and portable.** Offline, Windows-first, one binary, no external services.
The UI contract test fails the build if an `http(s)://` URL ever appears in the
page, the script or the CSS.

## How it is organised

| Layer | Where |
|---|---|
| server, options, sample documents | `src/main.rs` |
| HTTP API and both event streams | `src/api.rs` |
| domain types and persistence | `src/model.rs` |
| CAD readers | `src/step.rs`, `src/ole.rs`, `src/propset.rs`, `src/sldworks.rs`, `src/swhist.rs`, `src/stl.rs` |
| geometry and the operation program | `src/ops.rs`, `src/solid.rs`, `src/freecad.rs`, `src/score.rs`, `src/vec3.rs` |
| learning glue | `src/dataset.rs`, `src/pipeline.rs`, `src/vision.rs`, `src/system.rs` |
| interface | `static/` |
| training and inference | `ml/` |
| SolidWorks macros and generators | `tools/` |

`GLOSSARY.md` has the file-by-file version and the dependency map.

## The honest position on the model

This is the part most easily oversold, so it is stated plainly here and
repeatedly in the code.

What is built is the **pipeline, not the weights**. A fine-tuned small model on a
few hundred pairs learns the output *format* and the shapes it saw; it does not
learn engineering intent, and it will not generalise to arbitrary parts. The
limiting factor is paired data, not parameter count (`TRAINING.md` puts a 4B model
on an 8 GB card at 5–7 GB of VRAM, and says so).

That is why the studio answers picture-to-steps *without* any model, with a
geometric baseline, and is explicit that this is the baseline to beat. A model is
an upgrade you can measure, not a prerequisite for the tool working.
