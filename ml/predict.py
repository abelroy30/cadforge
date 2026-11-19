#!/usr/bin/env python3
"""Answer the studio's picture-to-steps question with a trained model.

The studio runs this when CADFORGE_VISION names it, and falls back to its own
geometric baseline if it fails, so failing loudly is the right behaviour.

It is handed the picture on disk, as raw pixels:

    CADFORGE_IMAGE    path to width x height raw RGBA bytes
    CADFORGE_WIDTH    pixels across
    CADFORGE_HEIGHT   pixels down
    CADFORGE_ADAPTER  the LoRA adapter to load (optional)
    CADFORGE_MODEL    base model id (optional)

and must print, as its last line, a JSON object shaped like the studio's own
inference:

    {"source": str, "width": int, "height": int,
     "steps": [{"op": str, "detail": str}],
     "program": {"version": 1, "units": "mm", "ops": [...]},
     "confidence": float, "notes": [str]}

Anything else on stdout is shown in the studio's log, so progress prints are
fine - the studio takes the last JSON object it finds.
"""

import json
import os
import sys

INSTRUCTION = (
    "You are shown one or more views of a mechanical part. "
    "Write the CAD operations that build it, as a single JSON object with keys "
    "version, units and ops. Reply with the JSON only."
)


def log(message):
    print(message, flush=True)


def fail(message):
    """A non-zero exit is what tells the studio to use its own baseline."""
    log("error: " + message)
    sys.exit(1)


def describe(program):
    """The same prose shape the studio's build-history view uses."""
    steps = []
    for op in program.get("ops", []):
        kind = op.get("op", "op")
        if kind == "extrude":
            shape = op.get("profile", {}).get("shape", "profile")
            steps.append(
                {
                    "op": "Extrude",
                    "detail": f"{shape} profile, distance {op.get('distance')} mm",
                }
            )
        elif kind == "hole":
            steps.append(
                {
                    "op": "Hole",
                    "detail": f"d {op.get('diameter')} mm, through all",
                }
            )
        elif kind == "hole_pattern":
            steps.append(
                {
                    "op": "HolePattern",
                    "detail": (
                        f"d {op.get('diameter')} mm, through all, "
                        f"{len(op.get('positions', []))} positions"
                    ),
                }
            )
        elif kind == "fillet":
            steps.append(
                {"op": "Fillet", "detail": f"radius {op.get('radius')} mm"}
            )
        else:
            steps.append({"op": kind, "detail": ""})
    return steps


def parse_program(text):
    """Pull the JSON object out of whatever the model wrapped it in."""
    start = text.find("{")
    end = text.rfind("}")
    if start < 0 or end <= start:
        fail("the model did not return a JSON object: " + text.strip()[:400])
    try:
        program = json.loads(text[start : end + 1])
    except json.JSONDecodeError as exc:
        fail(f"the model's JSON did not parse ({exc})")
    if not program.get("ops"):
        fail("the model returned a program with no operations")
    program.setdefault("version", 1)
    program.setdefault("units", "mm")
    return program


def main():
    image_path = os.environ.get("CADFORGE_IMAGE")
    width = int(os.environ.get("CADFORGE_WIDTH", "0"))
    height = int(os.environ.get("CADFORGE_HEIGHT", "0"))
    if not image_path or not width or not height:
        fail("CADFORGE_IMAGE, CADFORGE_WIDTH and CADFORGE_HEIGHT are all needed")

    adapter = os.environ.get("CADFORGE_ADAPTER") or str(
        __import__("pathlib").Path("data") / "runs" / "adapter"
    )
    model_id = os.environ.get("CADFORGE_MODEL", "Qwen/Qwen2-VL-2B-Instruct")

    try:
        import torch
        from PIL import Image
        from transformers import AutoProcessor, AutoModelForImageTextToText
    except ImportError as exc:
        fail(f"the model stack is not installed ({exc})")

    if not os.path.isfile(image_path):
        fail("the picture the studio staged is missing")

    # The studio sends raw RGBA: the browser has already decoded the upload, so
    # no image decoder has to travel with the server.
    raw = open(image_path, "rb").read()
    if len(raw) < width * height * 4:
        fail("the staged picture is shorter than its stated size")
    image = Image.frombytes("RGBA", (width, height), raw[: width * height * 4])

    if os.path.isdir(adapter):
        log(f"loading adapter {adapter}")
        from peft import PeftModel

        processor = AutoProcessor.from_pretrained(adapter)
        base = AutoModelForImageTextToText.from_pretrained(
            model_id, torch_dtype=torch.bfloat16, device_map="auto"
        )
        model = PeftModel.from_pretrained(base, adapter)
        source = f"{adapter}"
    else:
        log(f"no adapter at {adapter}; using the base model")
        processor = AutoProcessor.from_pretrained(model_id)
        model = AutoModelForImageTextToText.from_pretrained(
            model_id, torch_dtype=torch.bfloat16, device_map="auto"
        )
        source = model_id
    model.eval()

    messages = [
        {
            "role": "user",
            "content": [{"type": "image"}, {"type": "text", "text": INSTRUCTION}],
        }
    ]
    text = processor.apply_chat_template(
        messages, tokenize=False, add_generation_prompt=True
    )
    inputs = processor(text=[text], images=[image], return_tensors="pt")
    inputs = {k: v.to(model.device) for k, v in inputs.items()}

    log("generating...")
    with torch.no_grad():
        out = model.generate(**inputs, max_new_tokens=256, do_sample=False)
    answer = processor.batch_decode(
        out[:, inputs["input_ids"].shape[1] :], skip_special_tokens=True
    )[0]

    program = parse_program(answer)
    ops = program.get("ops") if isinstance(program, dict) else []
    # Not a calibrated probability, and it does not pretend to be one. What it
    # reflects is how well the answer held together: a program that parsed into
    # real operations is worth more than one that had to be salvaged into an
    # empty one. The studio shows it beside the baseline's own number.
    confidence = 0.6 if ops else 0.1
    result = {
        "source": "model: " + source,
        "width": width,
        "height": height,
        "steps": describe(program),
        "program": program,
        "confidence": confidence,
        "notes": [
            "Produced by the trained model, from the picture alone. "
            "Compare it with the studio's geometric baseline to see what "
            "training actually bought.",
        ],
    }
    # The studio reads the last JSON object, so this must be the final line.
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
