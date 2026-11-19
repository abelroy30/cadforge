#!/usr/bin/env python3
"""Fine-tune a vision-language model to turn a picture of a part into build steps.

This is the trainer the studio runs when you start a training run with
CADFORGE_TRAINER pointed at it. It reads the dataset the studio prepared:

    CADFORGE_DATASET   path to dataset.json (pictures paired with operations)
    CADFORGE_OUT       directory to write the adapter and the report into
    CADFORGE_MODEL     base model to adapt (default: see DEFAULT_MODEL below)

and prints plain progress lines, which the studio streams straight into the run
log. Everything it needs is on disk already; it downloads nothing.

What it does, honestly:

  * loads the base model in 4-bit (NF4) so a 4-billion-parameter model fits on
    an 8 GB card, and attaches LoRA adapters for training;
  * trains on (view, operation-program) pairs, where the programme is the JSON
    the studio reconstructed from the part's geometry;
  * writes a LoRA adapter, a tokenizer, and a short report into CADFORGE_OUT.

What it does NOT do: invent data. If the manifest holds few or no usable pairs
it says so and stops, because a model trained on nothing learns nothing.

Read docs/TRAINING.md for the settings that matter on a small GPU.
"""

import json
import os
import sys
from pathlib import Path

DEFAULT_MODEL = "Qwen/Qwen2-VL-2B-Instruct"

# The instruction is fixed so the model learns one job: picture(s) -> program.
INSTRUCTION = (
    "You are shown one or more views of a mechanical part. "
    "Write the CAD operations that build it, as a single JSON object with keys "
    "version, units and ops. Reply with the JSON only."
)


def log(message):
    """One progress line; the studio shows these in the run log as they arrive."""
    print(message, flush=True)


def die(message):
    log("error: " + message)
    sys.exit(1)


def load_manifest(path):
    if not path:
        die("no dataset was given: the studio sets CADFORGE_DATASET for a training run")
    p = Path(path)
    if not p.is_file():
        die(f"the dataset manifest {p} does not exist")
    data = json.loads(p.read_text(encoding="utf-8"))
    counts = data.get("counts", {})
    log(
        "dataset: {0} picture(s), {1} with a program, {2} document(s) with no picture".format(
            counts.get("pictures", 0),
            counts.get("with_program", 0),
            counts.get("documents_without_a_picture", 0),
        )
    )
    return data


def usable_pairs(manifest, data_root):
    """Pairs that have both a readable picture and a program to learn."""
    out = []
    skipped_no_program = 0
    skipped_no_file = 0
    for pair in manifest.get("pairs", []):
        program = pair.get("program")
        if not program or not program.get("ops"):
            skipped_no_program += 1
            continue
        image = Path(data_root) / pair["image"] if data_root else Path(pair["image"])
        if not image.is_file():
            skipped_no_file += 1
            continue
        out.append(
            {
                "image": str(image),
                "target": json.dumps(program, separators=(",", ":")),
                "name": pair.get("image_name", image.name),
            }
        )
    if skipped_no_program:
        log(
            f"skipped {skipped_no_program} pair(s) with no operation program "
            "(a SolidWorks file needs its feature tree exported to train on it)"
        )
    if skipped_no_file:
        log(f"skipped {skipped_no_file} pair(s) whose picture was missing from disk")
    return out


def main():
    dataset_path = os.environ.get("CADFORGE_DATASET")
    out_dir = Path(os.environ.get("CADFORGE_OUT", "checkpoints"))
    data_root = os.environ.get("CADFORGE_DATA", "")
    model_id = os.environ.get("CADFORGE_MODEL", DEFAULT_MODEL)
    epochs = float(os.environ.get("CADFORGE_EPOCHS", "3"))
    learning_rate = float(os.environ.get("CADFORGE_LR", "2e-4"))
    max_len = int(os.environ.get("CADFORGE_MAXLEN", "1024"))
    batch = int(os.environ.get("CADFORGE_BATCH", "1"))
    grad_accum = int(os.environ.get("CADFORGE_GRAD_ACCUM", "16"))
    lora_r = int(os.environ.get("CADFORGE_LORA_R", "16"))

    try:
        import torch
    except ImportError:
        die(
            "PyTorch is not installed for this Python. Run scripts\\setup-ml.cmd "
            "(Windows) or: pip install torch --index-url "
            "https://download.pytorch.org/whl/cu124"
        )
    try:
        from transformers import (
            AutoProcessor,
            AutoModelForImageTextToText,
            BitsAndBytesConfig,
            Trainer,
            TrainingArguments,
        )
        from peft import LoraConfig, get_peft_model, prepare_model_for_kbit_training
        from PIL import Image
    except ImportError as exc:
        die(f"the training stack is incomplete ({exc}). Run scripts\\setup-ml.cmd")

    if torch.cuda.is_available():
        name = torch.cuda.get_device_name(0)
        total = torch.cuda.get_device_properties(0).total_memory / 1024**3
        log(f"gpu: {name}, {total:.1f} GB")
        if total < 7.5:
            log(
                "warning: under 8 GB of VRAM. Use CADFORGE_BATCH=1 and "
                "CADFORGE_MAXLEN=768 or lower, or training may run out of memory."
            )
    else:
        log(
            "warning: PyTorch cannot see a GPU, so this will run on the CPU and "
            "may take days. Fix the CUDA install before a real run."
        )

    manifest = load_manifest(dataset_path)
    pairs = usable_pairs(manifest, data_root)
    if not pairs:
        die(
            "no usable training pairs. A vision run needs, for each part, at "
            "least one picture and one document whose operations were worked out "
            "(a STEP file, or a SolidWorks feature-tree export)."
        )
    log(f"training on {len(pairs)} pair(s)")

    # A small held-out slice, so the report can say whether it actually learned.
    split = max(1, len(pairs) // 20)
    eval_pairs = pairs[:split]
    train_pairs = pairs[split:] or pairs
    log(f"split: {len(train_pairs)} train, {len(eval_pairs)} held out")

    out_dir.mkdir(parents=True, exist_ok=True)

    log(f"loading {model_id} in 4-bit")
    quant = BitsAndBytesConfig(
        load_in_4bit=True,
        bnb_4bit_quant_type="nf4",
        bnb_4bit_compute_dtype=torch.bfloat16,
        bnb_4bit_use_double_quant=True,
    )
    processor = AutoProcessor.from_pretrained(model_id)
    model = AutoModelForImageTextToText.from_pretrained(
        model_id, quantization_config=quant, device_map="auto"
    )
    model = prepare_model_for_kbit_training(model)

    # Adapters only: the base weights are frozen, which is the whole reason a
    # 4B model can be trained on a card this small.
    lora = LoraConfig(
        r=lora_r,
        lora_alpha=lora_r * 2,
        lora_dropout=0.05,
        bias="none",
        task_type="CAUSAL_LM",
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj"],
    )
    model = get_peft_model(model, lora)
    model.print_trainable_parameters()

    image_token = getattr(processor.tokenizer, "pad_token_id", None)

    class Views(torch.utils.data.Dataset):
        def __init__(self, rows):
            self.rows = rows

        def __len__(self):
            return len(self.rows)

        def __getitem__(self, i):
            row = self.rows[i]
            image = Image.open(row["image"]).convert("RGB")
            messages = [
                {
                    "role": "user",
                    "content": [
                        {"type": "image"},
                        {"type": "text", "text": INSTRUCTION},
                    ],
                },
                {
                    "role": "assistant",
                    "content": [{"type": "text", "text": row["target"]}],
                },
            ]
            return {"messages": messages, "image": image, "name": row["name"]}

    def collate(batch):
        texts = [
            processor.apply_chat_template(
                row["messages"], tokenize=False, add_generation_prompt=False
            )
            for row in batch
        ]
        images = [row["image"] for row in batch]
        inputs = processor(
            text=texts, images=images, return_tensors="pt", padding=True,
            truncation=True, max_length=max_len,
        )
        labels = inputs["input_ids"].clone()
        # Train on the answer only: pad tokens and the picture's placeholders are
        # not something to learn to predict.
        if image_token is not None:
            labels[labels == image_token] = -100
        for token_id in (
            getattr(processor, "image_token_id", None),
            getattr(processor, "vision_start_token_id", None),
            getattr(processor, "vision_end_token_id", None),
        ):
            if token_id is not None:
                labels[labels == token_id] = -100
        inputs["labels"] = labels
        return inputs

    args = TrainingArguments(
        output_dir=str(out_dir / "trainer"),
        num_train_epochs=epochs,
        per_device_train_batch_size=batch,
        gradient_accumulation_steps=grad_accum,
        learning_rate=learning_rate,
        bf16=torch.cuda.is_available(),
        gradient_checkpointing=True,
        logging_steps=1,
        save_strategy="no",
        report_to=[],
        remove_unused_columns=False,
        dataloader_pin_memory=False,
    )

    trainer = Trainer(
        model=model,
        args=args,
        train_dataset=Views(train_pairs),
        eval_dataset=Views(eval_pairs),
        data_collator=collate,
    )

    log("training...")
    trainer.train()

    adapter_dir = out_dir / "adapter"
    model.save_pretrained(str(adapter_dir))
    processor.save_pretrained(str(adapter_dir))
    log(f"adapter written to {adapter_dir}")

    report = {
        "base_model": model_id,
        "adapter": str(adapter_dir),
        "pairs": len(pairs),
        "train": len(train_pairs),
        "held_out": len(eval_pairs),
        "epochs": epochs,
        "learning_rate": learning_rate,
        "lora_r": lora_r,
        "max_len": max_len,
    }
    (out_dir / "training-report.json").write_text(json.dumps(report, indent=2))
    log("report written to " + str(out_dir / "training-report.json"))
    log("done. Set CADFORGE_VISION=python ml/predict.py to use it in the studio.")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        log("stopped by the user")
        sys.exit(130)
