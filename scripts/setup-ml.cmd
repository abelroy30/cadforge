@echo off
REM Install the Python training stack: PyTorch with CUDA, transformers, peft.
REM Nothing here is needed to USE the studio - only to train a model with it.
setlocal
cd /d "%~dp0.."
title cadforge - machine learning setup

where python >nul 2>nul
if errorlevel 1 (
  echo.
  echo   Python was not found. Install Python 3.10 or newer from python.org
echo   and tick "Add python.exe to PATH" during the install.
  echo.
  pause
  exit /b 1
)

echo   Python:
python --version
echo.
echo   Graphics card:
nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv
if errorlevel 1 (
  echo   ^(nvidia-smi was not found, so PyTorch will only run on the CPU -
  echo    install the NVIDIA driver to train on the GPU^)
)

echo.
echo   Installing PyTorch with CUDA support. This is a large download...
python -m pip install --upgrade pip
python -m pip install torch torchvision --index-url https://download.pytorch.org/whl/cu124
if errorlevel 1 goto :failed

echo.
echo   Installing the rest of the stack...
python -m pip install -r ml\requirements.txt
if errorlevel 1 goto :failed

echo.
echo   Checking that PyTorch can see the GPU...
python -c "import torch;print('   torch', torch.__version__, '| cuda available:', torch.cuda.is_available());print('   device:', torch.cuda.get_device_name(0) if torch.cuda.is_available() else 'cpu')"

echo.
echo   Done. Start the studio with run.cmd and press "Refresh system" to check.
echo   Next: read docs\TRAINING.md - it explains what a 4B model needs.
echo.
pause
exit /b 0

:failed
echo.
echo   The install failed. Scroll up for the first error.
echo   If PyTorch is the problem, try without the CUDA index to at least get a
echo   working (CPU-only) install, then retry on a machine with the GPU driver.
 echo.
pause
exit /b 1
