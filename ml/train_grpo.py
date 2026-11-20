#!/usr/bin/env python3
"""Reinforcement-learning training with a staged curriculum, scored by the studio.

What this is: the loop samples a group of candidate operation programs for a
part, asks the studio to score them, and nudges the policy towards the better
members of the group. The reward is *not* computed here - it is computed in
Rust, by the studio, and fetched over HTTP - so there is exactly one definition
of what a good program is, and the interface can show the same numbers.

Be precise about the update rule, because the difference matters:

  * what IS implemented is the group-relative part of GRPO, applied through a
    supervised loss (RAFT-style). A group of candidates is scored, the group's
    mean is subtracted as a baseline, and the loss on each candidate is weighted
    by how much better than its group it did. That is the mechanism that makes
    reinforcement learning work with a black-box reward, and the reward here is
    a real verified one, not a learned guess.
  * what is NOT implemented is the PPO-style probability ratio and the KL term
    against a frozen reference model. Until that is added, this is not full GRPO
    and the report says so rather than claiming otherwise.

The curriculum climbs each part's *own* history rather than an absolute count:
ask for the first two operations, then the first five, then all of them. A part
with three operations therefore climbs the whole ladder, which an absolute
ladder would never let it do. In the last stage a dimension is hidden from the
model and it has to work the value out; the studio scores that recovery, so
'guessing the missing size' is trained and checked rather than hoped for.

Convergence stops the loop when the moving average of the reward stops
improving over a patience window - with a floor underneath it, because a curve
that has flattened at zero has not converged, it has failed. A hard ceiling on
steps and wall-clock time is the backstop, and hitting it is reported as 'not
converged'.

Environment:
  CADFORGE_DATASET    the manifest the studio wrote (the studio sets this)
  CADFORGE_OUT        where the adapter and report go (the studio sets this)
  CADFORGE_STUDIO     base URL of the studio, default http://127.0.0.1:8787
  CADFORGE_MODEL      base model, default Qwen/Qwen3-VL-4B-Instruct
  CADFORGE_GROUP      candidates sampled per part, default 4
  CADFORGE_LR         learning rate, default 1e-5
  CADFORGE_MAX_STEPS  hard ceiling on steps, default 400
  CADFORGE_MAX_EPOCHS passes over the whole curriculum, default 50
  CADFORGE_PATIENCE   evaluations to watch before calling it converged, default 12
  CADFORGE_EPSILON    an improvement below this counts as settled, default 0.01
  CADFORGE_FLOOR      below this the run has not converged, whatever the curve
  CADFORGE_CEILING_S  wall-clock ceiling in seconds, 0 for none
  CADFORGE_STUB=1     run the loop with a stand-in policy and no model at all.
                      It exercises the plumbing - scoring, the curriculum, the
                      convergence test, the progress protocol - without a
                      graphics card. It does not train anything.
"""

import json
import os
import random
import sys
import time
import urllib.error
import urllib.request


# ------------------------------------------------------------------ talking

def env(name, default=None):
    value = os.environ.get(name)
    return value if value not in (None, "") else default


def env_int(name, default):
    try:
        return int(env(name, default))
    except (TypeError, ValueError):
        return default


def env_float(name, default):
    try:
        return float(env(name, default))
    except (TypeError, ValueError):
        return default


def say(message):
    """A line for the studio's run log."""
    print(message, flush=True)


def progress(step, reward, mean, best, stage):
    """One point on the curve. The studio draws these as they arrive."""
    print(
        "CADFORGE_STEP "
        + json.dumps(
            {
                "step": int(step),
                "reward": round(float(reward), 5),
                "mean": round(float(mean), 5),
                "best": round(float(best), 5),
                "stage": int(stage),
            }
        ),
        flush=True,
    )


def done(converged, reason):
    """The verdict. The studio believes this line rather than guessing."""
    print(
        "CADFORGE_DONE " + json.dumps({"converged": bool(converged), "reason": reason}),
        flush=True,
    )


class Studio:
    """The reward, fetched. Scoring lives in the studio so that the number being
    optimised is the same number the interface shows."""

    def __init__(self, base_url):
        self.base_url = base_url.rstrip("/")

    def score(self, items):
        body = json.dumps({"items": items}).encode("utf-8")
        request = urllib.request.Request(
            self.base_url + "/api/rewards/score",
            data=body,
            headers={"content-type": "application/json"},
        )
        with urllib.request.urlopen(request, timeout=180) as response:
            payload = json.loads(response.read())
        return [entry["verdict"] for entry in payload["verdicts"]]


# ---------------------------------------------------------------- curriculum

def stages_for(reference_ops):
    """The ladder for one part, counted against that part's own history.

    Ask for the first two operations, then the first five, then all of them -
    and drop a rung that repeats, so a two-operation part has a two rung ladder
    rather than three identical ones.
    """
    n = len(reference_ops)
    ladder = []
    for want in (min(2, n), min(5, n), n):
        if want and want not in ladder:
            ladder.append(want)
    return ladder


def mask_for(pair, op_index, label):
    """Hide the primary dimension of one operation and report the true value."""
    ops = (pair.get("program") or {}).get("ops") or []
    if not ops:
        return None
    op = ops[op_index % len(ops)]
    for key in ("distance", "diameter", "radius"):
        if isinstance(op.get(key), (int, float)):
            return {"op": op_index % len(ops), "value": float(op[key]), "label": key}
    return None


# ------------------------------------------------------------------ policies

class StubPolicy:
    """A stand-in that does not learn and is not a model.

    It samples near the reference answer with noise that shrinks as the loop
    runs, which is enough to exercise scoring, the curriculum, the convergence
    test and the progress protocol on a machine with no graphics card. The
    report records that this was used, so a curve produced this way can never be
    mistaken for training.
    """

    def __init__(self, group, seed=7):
        self.group = group
        self.random = random.Random(seed)
        self.noise = 0.35

    def sample(self, pair, want):
        reference = pair.get("program") or {}
        ops = (reference.get("ops") or [])[:want]
        out = []
        for _ in range(self.group):
            candidate = json.loads(json.dumps(ops))
            for op in candidate:
                for key in ("distance", "diameter", "radius"):
                    value = op.get(key)
                    if isinstance(value, (int, float)):
                        drift = self.random.uniform(-1, 1) * self.noise * value
                        op[key] = max(0.1, round(value + drift, 3))
            out.append(
                {
                    "version": reference.get("version", 1),
                    "units": reference.get("units", "mm"),
                    "ops": candidate,
                }
            )
        return out

    def learn(self, advantages):
        # Better groups mean less noise, which is the shape of learning even
        # though nothing here has weights.
        if advantages and max(advantages) > 0:
            self.noise = max(0.02, self.noise * 0.93)


def load_vlm(model_name, torch):
    """Load the model with the class that actually matches it.

    Qwen3-VL and Qwen2.5-VL are different architectures, so loading one with the
    other's class either fails outright or - far worse - builds a model that runs
    and is not the one that was asked for. The generic image-text auto-class
    covers both, and the name it resolved to is printed, so the run log says what
    was really loaded rather than what was hoped for.
    """
    import transformers

    attempts = []
    auto = getattr(transformers, "AutoModelForImageTextToText", None)
    if auto is not None:
        attempts.append(("AutoModelForImageTextToText", auto))
    # Named fallbacks, for a transformers old enough to lack the auto-class on
    # this architecture but new enough to have the model itself.
    for name in ("Qwen3VLForConditionalGeneration", "Qwen2_5_VLForConditionalGeneration"):
        cls = getattr(transformers, name, None)
        if cls is not None:
            attempts.append((name, cls))

    errors = []
    for name, cls in attempts:
        try:
            model = cls.from_pretrained(
                model_name, torch_dtype=torch.bfloat16, device_map="auto"
            )
            say(f"loaded {model.__class__.__name__} for {model_name}")
            return model
        except Exception as exc:  # noqa: BLE001 - reported with the class that failed
            errors.append(f"{name}: {exc}")

    raise SystemExit(
        f"could not load {model_name} with transformers {transformers.__version__}. "
        "Tried: " + "; ".join(errors) + ". Upgrade with: pip install -U transformers peft"
    )


class VlmPolicy:
    """The real thing: a small vision-language model with a LoRA adapter."""

    def __init__(self, model_name, group, lr):
        try:
            import torch
            from transformers import AutoProcessor
            from peft import LoraConfig, get_peft_model
        except ImportError as exc:  # pragma: no cover - depends on the machine
            raise SystemExit(
                "this machine has no torch/transformers stack, so there is nothing to train "
                "with: run scripts/setup-ml.cmd first, or set CADFORGE_STUB=1 to exercise the "
                "loop without a model. Underlying error: " + str(exc)
            )

        self.torch = torch
        self.group = group
        # The most recent sampling is kept so the update can compute the loss on
        # the very answers that were scored, conditioned on the same picture.
        self._last = None
        self.processor = AutoProcessor.from_pretrained(model_name)
        model = load_vlm(model_name, torch)
        lora = LoraConfig(
            r=16,
            lora_alpha=32,
            lora_dropout=0.05,
            target_modules=["q_proj", "k_proj", "v_proj", "o_proj"],
            task_type="CAUSAL_LM",
        )
        self.model = get_peft_model(model, lora)
        self.model.train()
        self.optimiser = torch.optim.AdamW(
            [p for p in self.model.parameters() if p.requires_grad], lr=lr
        )

    def _messages(self, pair, want, masked):
        text = (
            "You are reading a part and writing the operations that built it.\n"
            "Answer with JSON only, shaped {\"version\": 1, \"units\": \"mm\", \"ops\": [...]}.\n"
            f"Give the first {want} operation(s), in the order they were made.\n"
            "An extrude is {\"op\": \"extrude\", \"profile\": {...}, \"distance\": mm}."
        )
        if masked:
            text += (
                f"\nOne dimension has been removed: the {masked['label']} of operation "
                f"{masked['op'] + 1}. Work it out from the rest of the part."
            )
        content = [{"type": "text", "text": text}]
        image = pair.get("image_path")
        if image and image.lower().endswith((".png", ".jpg", ".jpeg", ".webp")):
            content.insert(0, {"type": "image", "image": image})
        return [{"role": "user", "content": content}]

    def _encode(self, chat):
        # return_dict=True is what carries the picture. Without it the processor
        # hands back token ids only, and a vision-language model silently becomes
        # a text one that never sees the part - which is exactly the kind of
        # quiet no-op this project keeps having to dig out.
        return self.processor.apply_chat_template(
            chat,
            tokenize=True,
            add_generation_prompt=True,
            return_tensors="pt",
            return_dict=True,
        )

    def sample(self, pair, want, masked=None):
        chat = self._messages(pair, want, masked)
        inputs = self._encode(chat).to(self.model.device)
        prompt_len = int(inputs["input_ids"].shape[1])
        with self.torch.no_grad():
            out = self.model.generate(
                **inputs,
                do_sample=True,
                temperature=0.9,
                top_p=0.95,
                max_new_tokens=512,
                num_return_sequences=self.group,
            )
        answers = out[:, prompt_len:]
        # Kept for the update step, so it learns from the answers that were
        # actually scored rather than from a fresh sample that was not.
        self._last = (inputs, answers.tolist(), prompt_len)
        text = self.processor.batch_decode(answers, skip_special_tokens=True)
        return [self._as_program(t) for t in text]

    @staticmethod
    def _as_program(text):
        start = text.find("{")
        end = text.rfind("}")
        if start < 0 or end <= start:
            return {"version": 1, "units": "mm", "ops": []}
        try:
            return json.loads(text[start:end + 1])
        except json.JSONDecodeError:
            return {"version": 1, "units": "mm", "ops": []}

    def learn(self, advantages):
        """One advantage-weighted step per sampled candidate.

        The loss is the model's own loss on the answer it produced, conditioned
        on the same picture and prompt - the prompt and the image tokens are
        masked out, so only the answer is learned. Each candidate's loss is
        weighted by how much better than its group it scored.

        This is deliberately the simple, inspectable version: the PPO ratio and
        the KL term that would make it full GRPO are not here yet."""
        if not self._last:
            return
        torch = self.torch
        inputs, answers, prompt_len = self._last
        for row, advantage in zip(answers, advantages):
            answer = torch.tensor([row], device=self.model.device)
            full = torch.cat([inputs["input_ids"], answer], dim=1)
            labels = full.clone()
            labels[:, :prompt_len] = -100
            batch = {
                key: value
                for key, value in inputs.items()
                if key != "input_ids"
            }
            batch["input_ids"] = full
            batch["labels"] = labels
            if "attention_mask" in inputs:
                batch["attention_mask"] = torch.cat(
                    [inputs["attention_mask"], torch.ones_like(answer)], dim=1
                )
            weight = 1.0 + max(-1.0, min(2.0, float(advantage)))
            out = self.model(**batch)
            loss = out.loss * weight
            self.optimiser.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(self.model.parameters(), 1.0)
            self.optimiser.step()

    def save(self, path):
        self.model.save_pretrained(path)
        self.processor.save_pretrained(path)


# -------------------------------------------------------------- convergence

def moving_average(history, window=10):
    tail = history[-window:]
    return sum(tail) / len(tail)


def has_converged(history, patience, epsilon, floor, best):
    """Settled *and* worth settling at.

    The floor is the important half: a reward curve that has flattened at zero
    has not converged, it has failed, and a run that says otherwise is the most
    expensive kind of wrong answer.
    """
    if len(history) < patience + 1:
        return False
    window = history[-(patience + 1):]
    half = max(1, len(window) // 2)
    first = sum(window[:half]) / half
    last = sum(window[-half:]) / half
    if last - first >= epsilon:
        return False
    return best >= floor


# ---------------------------------------------------------------------- main

def main():
    manifest_path = env("CADFORGE_DATASET")
    if not manifest_path:
        raise SystemExit("CADFORGE_DATASET is not set, so there is nothing to train on")
    if not os.path.isfile(manifest_path):
        raise SystemExit(f"no manifest at {manifest_path}")

    with open(manifest_path, encoding="utf-8") as handle:
        manifest = json.load(handle)

    pairs = [
        p
        for p in manifest.get("pairs", [])
        if p.get("program") and (p.get("program") or {}).get("ops") and p.get("split") == "train"
    ]
    held_out = [
        p
        for p in manifest.get("pairs", [])
        if p.get("program") and (p.get("program") or {}).get("ops") and p.get("split") == "val"
    ]
    if not pairs:
        raise SystemExit(
            "this dataset has no training pairs that carry an operation program. A program "
            "comes from a STEP part or a SolidWorks export; a picture of a part on its own "
            "has no operations to learn."
        )

    stub = env("CADFORGE_STUB") == "1"
    group = env_int("CADFORGE_GROUP", 4)
    lr = env_float("CADFORGE_LR", 1e-5)
    max_steps = env_int("CADFORGE_MAX_STEPS", 400)
    patience = env_int("CADFORGE_PATIENCE", 12)
    epsilon = env_float("CADFORGE_EPSILON", 0.01)
    floor = env_float("CADFORGE_FLOOR", 0.1)
    ceiling = env_int("CADFORGE_CEILING_S", 0)
    model_name = env("CADFORGE_MODEL", "Qwen/Qwen3-VL-4B-Instruct")
    out_dir = env("CADFORGE_OUT", ".")
    os.makedirs(out_dir, exist_ok=True)

    rewards = [r for r in manifest.get("rewards", []) if r]
    say(f"{len(pairs)} training part(s), {len(held_out)} held out")
    say("reward: " + ", ".join(f"{r['name']} ({r['kind']} x{r['weight']})" for r in rewards))
    if stub:
        say("CADFORGE_STUB=1: a stand-in policy, no model, nothing is trained")
        policy = StubPolicy(group)
    else:
        say(f"loading {model_name}")
        policy = VlmPolicy(model_name, group, lr)
    studio = Studio(env("CADFORGE_STUDIO", "http://127.0.0.1:8787"))

    # The ladder is per part, so group the parts by how many rungs they have.
    ladders = {p["image_id"]: stages_for(p.get("reference_ops") or []) for p in pairs}
    stages = sorted({len(v) for v in ladders.values()})
    if not stages:
        raise SystemExit("every part has an empty reference sequence, so there is no ladder to climb")
    top = max(stages)
    say(f"curriculum: {top} stage(s), counted against each part's own history")

    history = []
    best = 0.0
    step = 0
    started = time.time()
    reason = "the loop ended without the reward settling"
    converged = False

    # The ladder is climbed more than once. One pass over a small dataset is a
    # handful of steps, and a loop that stopped there could never reach the
    # patience window, let alone converge - which would make a small shop's data
    # untrainable. What ends training is the ceiling, not the ladder.
    epochs = env_int("CADFORGE_MAX_EPOCHS", 50)
    plan = [(epoch, stage) for epoch in range(epochs) for stage in range(top)]
    for epoch, stage in plan:
        rung = [p for p in pairs if len(ladders[p["image_id"]]) > stage]
        say(f"epoch {epoch + 1}/{epochs} stage {stage + 1}/{top}: {len(rung)} part(s), "
            + ("the whole history and a hidden dimension"
               if stage == top - 1 else "a longer prefix"))
        random.Random(stage * 977 + epoch).shuffle(rung)

        for pair in rung:
            want = ladders[pair["image_id"]][stage]
            masked = mask_for(pair, stage, "dimension") if stage == top - 1 else None
            if masked:
                say(f"  hiding one dimension of {pair.get('image_name')} for stage {stage + 1}")

            candidates = policy.sample(pair, want, masked) if not stub else policy.sample(pair, want)
            items = [
                {
                    "asset_id": pair.get("source_id"),
                    "program": candidate,
                    "masked": only_value(masked),
                }
                for candidate in candidates
            ]
            if not pair.get("source_id"):
                say(f"  skipping {pair.get('image_name')}: no source document to score against")
                continue

            verdicts = studio.score(items)
            group_rewards = [float(v["total"]) for v in verdicts]
            mean = sum(group_rewards) / len(group_rewards)
            spread = (sum((r - mean) ** 2 for r in group_rewards) / len(group_rewards)) ** 0.5
            advantages = [(r - mean) / (spread + 1e-6) for r in group_rewards]

            # The update happens for a real model too. This used to be skipped
            # unless the stub was running, so a real policy was loaded, sampled
            # and scored and then never changed: a training loop that evaluated
            # itself and never learned.
            policy.learn(advantages)
            step += 1
            best = max(best, max(group_rewards))
            history.append(mean)
            average = moving_average(history)
            progress(step, mean, average, best, stage + 1)

            if has_converged(history, patience, epsilon, floor, best):
                converged = True
                reason = f"the reward settled at {average:.3f} after {step} step(s)"
                break
            if step >= max_steps:
                reason = f"hit the ceiling of {max_steps} steps without settling"
                break
            if ceiling and time.time() - started > ceiling:
                reason = f"hit the wall-clock ceiling of {ceiling}s without settling"
                break
        if converged or step >= max_steps or (ceiling and time.time() - started > ceiling):
            break

    # A held-out score, so the report says something a training number cannot:
    # how the policy does on parts it was never paid to reproduce.
    held_out_mean = None
    if held_out:
        try:
            scores = []
            for pair in held_out[:20]:
                if not pair.get("source_id"):
                    continue
                candidate = policy.sample(pair, len(pair.get("reference_ops") or []) or 1)
                verdicts = studio.score(
                    [{"asset_id": pair["source_id"], "program": candidate[0]}]
                )
                scores.append(float(verdicts[0]["total"]))
            if scores:
                held_out_mean = sum(scores) / len(scores)
                say(f"held out: {held_out_mean:.3f} over {len(scores)} part(s)")
        except (urllib.error.URLError, KeyError) as exc:
            say(f"could not score the held-out parts: {exc}")

    adapter = os.path.join(out_dir, "adapter")
    if stub:
        say("nothing to save: the stand-in policy has no weights")
    else:
        policy.save(adapter)
        say(f"adapter written to {adapter}")

    report = {
        "model": None if stub else model_name,
        "stub": stub,
        "steps": step,
        "best_reward": round(best, 4),
        "final_mean": round(moving_average(history), 4) if history else None,
        "converged": converged,
        "reason": reason,
        "stages": top,
        "group_size": group,
        "training_parts": len(pairs),
        "held_out_parts": len(held_out),
        "held_out_reward": None if held_out_mean is None else round(held_out_mean, 4),
        "floor": floor,
        "patience": patience,
        "epsilon": epsilon,
        "update": "group-relative advantages, applied through a supervised loss",
        "not_implemented": "the PPO ratio and the KL term of full GRPO",
        "elapsed_s": round(time.time() - started, 1),
        "rewards": rewards,
    }
    with open(os.path.join(out_dir, "report.json"), "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
    say(f"report written to {os.path.join(out_dir, 'report.json')}")

    done(converged, reason)
    return 0


def only_value(masked):
    """The studio wants the hidden value and which operation it belonged to; the
    label is for the log."""
    if not masked:
        return None
    return {"op": masked["op"], "value": masked["value"]}


if __name__ == "__main__":
    sys.exit(main())
