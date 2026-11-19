//! What this machine can actually do.
//!
//! Training a four-billion-parameter vision model is not something the studio can
//! promise: it depends on the graphics card, the driver, and a Python stack with
//! a CUDA build of PyTorch. Rather than pretend, the studio looks, and says what
//! it found - and what to install if something is missing. Honest numbers beat
//! an optimistic progress bar.
//!
//! Everything here is a read-only probe of the local machine. Nothing is
//! installed, nothing is downloaded, and no probe leaves the computer.

use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Gpu {
    pub present: bool,
    pub name: String,
    pub vram_mb: u64,
    pub driver: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Python {
    pub present: bool,
    /// The command that worked, e.g. `python` or `py -3`.
    pub command: String,
    pub version: String,
    /// `torch.__version__` when it imports, otherwise `None`.
    pub torch: Option<String>,
    /// Does that PyTorch see a CUDA device?
    pub cuda: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,
    pub gpu: Gpu,
    pub python: Python,
    pub trainer: String,
    pub external_trainer: Option<String>,
    /// A command the studio will run to turn a picture into steps, if one is
    /// configured; otherwise the built-in geometric baseline is used.
    pub vision_command: Option<String>,
    pub advice: Vec<String>,
}

/// Run a program and return its trimmed stdout, or `None` if it is missing or
/// fails. Probes are best-effort by design: a missing tool is information, not
/// an error.
fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// The Python interpreters worth trying, most specific first. `CADFORGE_PYTHON`
/// wins, so a user with a virtualenv can point at it.
fn python_candidates() -> Vec<Vec<String>> {
    let mut out = Vec::new();
    if let Ok(custom) = std::env::var("CADFORGE_PYTHON") {
        if !custom.trim().is_empty() {
            out.push(custom.split_whitespace().map(String::from).collect());
        }
    }
    for c in ["python", "python3", "py"] {
        out.push(vec![c.to_string()]);
    }
    out
}

fn probe_python() -> Python {
    for cand in python_candidates() {
        let program = &cand[0];
        let mut args: Vec<&str> = cand[1..].iter().map(String::as_str).collect();
        let is_py_launcher = program == "py";
        if is_py_launcher && args.is_empty() {
            args.push("-3");
        }
        let mut version_args = args.clone();
        version_args.push("--version");
        let Some(version) = run(program, &version_args) else {
            continue;
        };

        // Is this interpreter one that can run the training stack?
        let mut probe_args = args.clone();
        probe_args.push("-c");
        probe_args.push("import torch;print(torch.__version__, int(torch.cuda.is_available()))");
        let (torch, cuda) = match run(program, &probe_args) {
            Some(text) => {
                let mut parts = text.split_whitespace();
                let v = parts.next().map(String::from);
                let c = parts.next().map(|s| s == "1").unwrap_or(false);
                (v, c)
            }
            None => (None, false),
        };

        let command = cand.join(" ");
        return Python {
            present: true,
            command,
            version,
            torch,
            cuda,
        };
    }
    Python::default()
}

fn probe_gpu() -> Gpu {
    let text = run(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total,driver_version",
            "--format=csv,noheader",
        ],
    );
    let Some(line) = text.and_then(|t| t.lines().next().map(String::from)) else {
        return Gpu::default();
    };
    let mut parts = line.split(',').map(|s| s.trim().to_string());
    let name = parts.next().unwrap_or_default();
    let vram = parts
        .next()
        .and_then(|s| s.split_whitespace().next().map(String::from))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let driver = parts.next().unwrap_or_default();
    Gpu {
        present: !name.is_empty(),
        name,
        vram_mb: vram,
        driver,
    }
}

/// Look at the machine and work out what is ready and what is not.
pub fn detect() -> SystemInfo {
    let gpu = probe_gpu();
    let python = probe_python();
    let external_trainer = std::env::var("CADFORGE_TRAINER")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let vision_command = std::env::var("CADFORGE_VISION")
        .ok()
        .filter(|s| !s.trim().is_empty());

    let mut advice: Vec<String> = Vec::new();

    if !gpu.present {
        advice.push(
            "No NVIDIA GPU was found. Training falls back to the CPU, which cannot hold a \
             4B model - install the NVIDIA driver so nvidia-smi works."
                .into(),
        );
    } else {
        let gb = gpu.vram_mb as f64 / 1024.0;
        if gpu.vram_mb < 12_000 {
            advice.push(format!(
                "{} has {:.0} GB of VRAM. A 4B model will not fit at 16-bit: train it with \
                 4-bit QLoRA and gradient checkpointing, at short sequences and batch 1. \
                 See docs/TRAINING.md for the exact settings.",
                gpu.name, gb
            ));
        } else {
            advice.push(format!(
                "{} has {:.0} GB of VRAM - enough for 4-bit QLoRA of a 4B model.",
                gpu.name, gb
            ));
        }
    }

    if !python.present {
        advice.push(
            "Python was not found. The training scripts need Python 3.10 or newer - install \
             it from python.org and tick \"Add python.exe to PATH\"."
                .into(),
        );
    } else if python.torch.is_none() {
        advice.push(format!(
            "{} is installed but PyTorch is not. Run scripts/setup-ml.cmd to install the \
             CUDA build, or pip install the packages in ml/requirements.txt.",
            python.version
        ));
    } else if !python.cuda {
        advice.push(
            "PyTorch cannot see the GPU, so it is a CPU-only build. Reinstall it with the \
             CUDA index, as scripts/setup-ml.cmd does."
                .into(),
        );
    }

    if vision_command.is_none() {
        advice.push(
            "No vision model is configured, so the picture-to-steps test runs the built-in \
             geometric baseline: it measures the silhouette and writes the operations that \
             would produce it. Set CADFORGE_VISION to use your trained model instead."
                .into(),
        );
    }

    SystemInfo {
        os: std::env::consts::OS.to_string(),
        gpu,
        python,
        trainer: crate::pipeline::trainer_name(&external_trainer),
        external_trainer,
        vision_command,
        advice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_reports_something_on_any_machine() {
        let info = detect();
        assert!(!info.os.is_empty());
        // There is always advice to give: at minimum, how the vision test will
        // behave when no model is configured.
        assert!(!info.advice.is_empty());
        // The GPU probe never lies about being present without a name.
        assert!(info.gpu.present == !info.gpu.name.is_empty());
    }
}
