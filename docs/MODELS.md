# Loading a model, and training it on your own parts

Which model to pick, how to get it onto the machine, how to train it here, and
how to tell whether it learned anything. `TRAINING.md` is the background; this is
the one to follow at the keyboard.

## Three different "models", and only one of them is weights

This trips people up, so state it plainly:

| Where | What it is | Set by |
|---|---|---|
| **A registered base model** | a *record*: name, kind, parameter count, notes. It is what a run is started against, and what the *Storage* panel estimates a model's size from. **It downloads nothing** | the studio's **Base models** panel, or `POST /api/models` |
| **The weights the trainer loads** | the actual model, fetched by `from_pretrained` when a run starts | `CADFORGE_MODEL` on the trainer |
| **The weights the inferencer loads** | the same base plus the LoRA adapter that was trained | `CADFORGE_VISION`, and `CADFORGE_ADAPTER` or a deploy |

So registering `Qwen3-VL-4B-Instruct` in the studio does not fetch it, and
setting `CADFORGE_MODEL` does not tell the studio what it is training. They are
two different things that happen to share a name; keep them the same name.

## Which one to load

The task is *picture(s) of a part -> a program of CAD operations*. That needs a
model that reads images, and a model that emits strict JSON.

| Model | 4-bit weights, roughly | Why |
|---|---|---|
| **`Qwen/Qwen3-VL-4B-Instruct`** | 2.5-3 GB | **Start here.** Trains on an 8 GB card, reads a drawing's dimension text, and is the current default of `ml/train_grpo.py` |
| `Qwen/Qwen2.5-VL-3B-Instruct` | ~2 GB | The safe fallback: more tooling exists, and it fits with more headroom |
| `Qwen/Qwen3-VL-8B-Instruct` | 5-6 GB | Better grounding, but wants 16 GB to train, not 8 |
| `Qwen/Qwen2.5-Coder-7B-Instruct` | ~4.5 GB | Only if you drop the vision path entirely: STEP files in, programs out. No pictures, so no drawing dimensions either |

Those sizes are **estimates from parameter counts, not measurements** - the same
distinction the *Storage* panel makes, for the same reason.

Two things worth being straight about:

* **Video is not a route to 3D here.** Qwen3-VL can take a video, but watching a
  part turn shows *appearance*, not geometry. Four stills are cheaper and no
  worse for this task. The honest use of video would be more conditioning views
  for the same reward, not reconstruction.
* **The strongest argument for a VLM in this project is dimension text.** The
  geometric baseline assumes the longest side is 100 mm, because a picture alone
  does not carry a scale. A VLM that reads `40` off a title block fixes that at
  the source - that is what `Qwen/Qwen3-VL-4B-Instruct`'s OCR is for.

## Getting it onto the machine, once

Install the stack:

```bat
scripts\setup-ml.cmd
```

Then fetch the weights once, so nothing needs the network mid-run:

```bat
pip install -U huggingface_hub
huggingface-cli download Qwen/Qwen3-VL-4B-Instruct --local-dir models\qwen3-vl-4b
```

A local folder is a perfectly good `CADFORGE_MODEL`. After that the trainer and
the inferencer both load from disk and nothing is fetched at all:

```bat
set CADFORGE_MODEL=models\qwen3-vl-4b
```

Check what the machine is actually willing to do before a long run:

```sh
curl -s localhost:8787/api/system
```

## Train it here

Register the model in **Base models** (a record - see the table above), make a
dataset from your parts and their views, then either:

**Supervised** - one picture to one recorded program:

```bat
set CADFORGE_TRAINER=python ml/train_vision.py
set CADFORGE_MODEL=models\qwen3-vl-4b
run.cmd
```

**The loop** - sample candidates, score them with the studio's reward, learn:

```bat
set CADFORGE_TRAINER=python ml/train_grpo.py
set CADFORGE_MODEL=models\qwen3-vl-4b
run.cmd
```

Both write `adapter/` into the run's output folder, and both appear in the run's
**outputs** in the studio. The loop also draws the reward curve and ends with the
trainer's own verdict in words.

### Settings that matter on a small card

| Setting | 8 GB | 12 GB+ | Why |
|---|---|---|---|
| `CADFORGE_BATCH` | `1` | `1`-`2` | Anything larger runs out of memory |
| `CADFORGE_MAXLEN` | `768` | `1024` | Sequence length is the big cost |
| `CADFORGE_GRAD_ACCUM` | `16` | `8` | Rebuilds an effective batch without the memory |
| `CADFORGE_GROUP` | `2` | `4` | Candidates sampled per part per step |
| `CADFORGE_MAX_STEPS` | `200` | `400` | A ceiling, not a target |

`CUDA out of memory` is answered by lowering `CADFORGE_MAXLEN` first, then
`CADFORGE_LORA_R`, then `CADFORGE_GROUP`. Nothing else helps as much.

## Did the picture actually reach the model?

Worth checking deliberately, because the failure is silent. If the processor is
called without `return_dict=True`, a vision-language model is handed token ids
only and quietly becomes a text model that never sees the part: it still runs,
still generates, still scores, and has learned nothing about geometry.

So the run log should say, near the top:

```text
loading models\qwen3-vl-4b
loaded Qwen3VLForConditionalGeneration for models\qwen3-vl-4b
```

The class name is printed by the loader, so the log states what was loaded rather
than what was hoped for. If it names a different class, or the load fails, the
error lists every class it tried, and the fix is `pip install -U transformers peft`.

## Using it

Point the studio at the inferencer:

```bat
set CADFORGE_VISION=python ml/predict.py
set CADFORGE_MODEL=models\qwen3-vl-4b
run.cmd
```

Or press **Deploy** on the finished run in the **Runs** tab, which is the same
thing without the environment: the studio serves that run's adapter and passes
the base model it was trained against, so the adapter is loaded onto the weights
it belongs to rather than onto a default that would silently mismatch.

## Honest limits

* **None of this has been run with real weights in this repository.** The trainer
  is written, its protocol is tested, and the plumbing is exercised with
  `CADFORGE_STUB=1` - but no GPU run appears in this project's history.
* A 4B model fine-tuned on a few hundred pairs learns the output format and the
  shapes it saw; it does not learn engineering intent.
* One view cannot give a part's depth. More views help; the ambiguity does not
  vanish.

## Troubleshooting

| What you see | What it means |
|---|---|
| `could not load ... with transformers <version>` | The installed transformers predates that architecture. `pip install -U transformers peft` |
| `PyTorch cannot see a GPU` | The CUDA build is wrong. Nothing will finish in reasonable time on the CPU |
| `no usable training pairs` | Every picture lacks a program. A picture pairs with a part; a STEP part or a feature-tree export is what carries the operations |
| `held-out loss` not improving while training loss falls | Memorisation. More parts, not more epochs |
| The panel says `baseline` after deploying | The vision command failed and the studio fell back loudly - read the notes in the result, they say why |
