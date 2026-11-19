# Training a model that turns pictures into CAD steps

This is the honest version of "I am going to train a 4-billion-parameter model
on my RTX 3070". It is doable, but it is not a button, and most of the work is
the data rather than the model.

## The short version

* **8 GB of VRAM cannot hold a 4B model in 16-bit.** It can hold a 4B model in
  4-bit with LoRA adapters, at short sequences and batch 1. That is what
  `ml/train_vision.py` does.
* **The studio gives you the data**, not the weights: a dataset manifest pairing
  each picture with the operations that make it.
* **A model trained on a few hundred parts learns the format and common shapes.**
  It will not generalise to arbitrary parts. That is a data problem, not a
  settings problem.
* **You always have a baseline.** The studio can read steps from a picture with
  no model at all (the "Picture to steps" panel). That baseline is what your
  trained model has to beat - and now you can see both, side by side.

## What the 8 GB actually buys you

Roughly, for a 4B VLM trained with QLoRA at sequence length 1024, batch 1:

| Part                                | Roughly   |
|-------------------------------------|-----------|
| Base weights, 4-bit NF4             | 2.2-2.6 GB |
| LoRA adapters + 8-bit optimizer     | 0.3-0.6 GB |
| Activations, gradient checkpointing | 1.5-2.5 GB |
| CUDA context and buffers            | 0.6-1.0 GB |
| **Total**                           | **5-7 GB** |

So it fits, with little room to spare. The settings that matter:

| Setting                | Use this            | Why |
|------------------------|---------------------|-----|
| `CADFORGE_BATCH`       | `1`                 | Anything larger runs out of memory |
| `CADFORGE_GRAD_ACCUM`  | `8` to `16`         | Rebuilds an effective batch without the memory |
| `CADFORGE_MAXLEN`      | `768` to `1024`     | Shorter sequences are much cheaper |
| `CADFORGE_LORA_R`      | `16`                | r=32 is rarely worth the memory |
| Image resolution       | 224 to 336 px       | A 448 px picture costs 4x the tokens |

If you see `CUDA out of memory`, lower `CADFORGE_MAXLEN` first, then
`CADFORGE_LORA_R`. Nothing else will help as much.

## Which base model

Set `CADFORGE_MODEL` to change it. Sensible choices:

* `Qwen/Qwen2-VL-2B-Instruct` - the default. Starts on a 3070 without drama, and
  is the right size for a first run: get the pipeline working here first.
* `OpenGVLab/InternVL2-4B` - an actual 4-billion-parameter vision-language model,
  which is what you asked for.
* `Qwen/Qwen2.5-VL-3B-Instruct` or `-7B-Instruct` - strong at structured output
  and reading diagrams. The 7B will not fit on 8 GB even in 4-bit at useful
  sequence lengths; it needs 12 GB or more.

Whatever you pick, run one epoch on 100 pairs first. If that works, scale up.

## The data is the job

A pair is *(one or more pictures of a part, the operations that build it)*.

The studio builds those pairs for you:

1. **Upload the CAD** into the **STEP / STP** lane (or the SolidWorks lane).
2. **Select a part and press "Capture views".** The studio renders front, top,
   right and isometric views on a transparent background, files them as
   `view` artifacts, and lays out a drawing sheet with a title block.
3. **Tick the part and its views, then "New dataset from selected".**
4. **Start a training run.** The studio writes the manifest, pairs each picture
   with its part by name, and hands it to the trainer.

What each source gives you:

| Source            | Target the trainer can use |
|-------------------|----------------------------|
| STEP / STP        | the operations reconstructed from the geometry - a full program |
| SolidWorks part   | its parameters only, unless you export the feature tree |
| Feature-tree JSON | the true recorded operations - the best target there is |

**For SolidWorks users:** run `tools/solidworks/ExportFeatureTree.bas` in
SolidWorks and upload the JSON. That is the only way to train on what the part
was *actually* built from rather than what the geometry suggests.

The manifest is written to `data/runs/<run-id>/dataset.json` and is also
available at `GET /api/datasets/<id>/export`. It states, per pair, how the
target was obtained (`reconstructed_from_geometry`, `recorded_steps`,
`parameters_only`), so you can filter by how much you trust it.

## Running it

Install the stack once (Windows):

```bat
scripts\setup-ml.cmd
```

Then either use the studio - register a base model, make a dataset, press
*Start training run* - or run the script by hand against a manifest:

```bat
set CADFORGE_DATASET=data\runs\<id>\dataset.json
set CADFORGE_OUT=checkpoints\run1
set CADFORGE_MODEL=OpenGVLab/InternVL2-4B
python ml\train_vision.py
```

The studio runs it for you by setting `CADFORGE_TRAINER`:

```
set CADFORGE_TRAINER=python ml/train_vision.py
```

and passing `CADFORGE_DATASET`, `CADFORGE_OUT` and `CADFORGE_DATA` to it. That
is the whole interface: the studio describes the dataset on disk, the trainer
reads it and prints progress lines, which appear in the run log live.

## Using the model in the studio

```
set CADFORGE_VISION=python ml/predict.py
set CADFORGE_ADAPTER=checkpoints\run1\adapter
```

Now "Picture to steps" runs your model first, and falls back to the built-in
baseline - loudly, with the reason - if it fails. The panel shows which one
answered.

## Installing it all offline

Nothing in the studio needs the network. Training does, once, unless you have
the wheels already:

* **Rust** - `rustup-init.exe` (needed once to build the studio).
* **Python** 3.10+ from python.org.
* **PyTorch** with CUDA - `scripts\setup-ml.cmd` fetches it from
  `download.pytorch.org`.
* **The base model** - download it from Hugging Face once (`pip install
  huggingface_hub` then `huggingface-cli download <model>`), or point
  `CADFORGE_MODEL` at a folder you already have. `from_pretrained` accepts a
  local path, and then nothing is fetched at all.

After that, pull the network cable: the studio, the readers, the rebuild engine
and the trainer all run locally.

## How to tell whether it worked

1. **Compare against the baseline.** Run the same picture through the studio with
   and without `CADFORGE_VISION` set. If the model is not better, the model is
   not better.
2. **Rebuild and measure.** The panel's *Rebuild this* button turns the predicted
   steps into real geometry; the studio scores the result against the original
   (chamfer distance and size delta). That number is the honest score.
3. **Watch the held-out split.** The trainer holds out 5% of pairs and reports
   the counts. If training loss falls and held-out quality does not, you are
   memorising.

## Honest limits

* A 4B model fine-tuned on a few thousand pairs learns the **output format** and
  the shapes it saw. It does not learn engineering intent.
* One view cannot tell you a part's depth or its feature tree. Multi-view
  capture helps; the serializer's ambiguity does not disappear.
* To go further than this, the useful direction is not a bigger model - it is
  more paired data. CAD datasets in B-rep or sequence form, or the parts you
  already own, will move the result far more than another few billion
  parameters will.
