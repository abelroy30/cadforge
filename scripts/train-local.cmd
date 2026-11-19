@echo off
REM One command to fine-tune the vision model on this machine.
REM
REM The studio does this for you from the Training card; this script is the same
REM thing without the browser, for when you want to run a long job in a window you
REM control. Everything it needs is set here, so it is safe to copy onto a fresh
REM machine that already ran scripts\setup-ml.cmd.
REM
REM   scripts\train-local.cmd                  train with the defaults below
REM   set CADFORGE_EPOCHS=8 && scripts\train-local.cmd
REM   set CADFORGE_MODEL=Qwen/Qwen2.5-VL-3B-Instruct && scripts\train-local.cmd

setlocal
cd /d "%~dp0.."

set PYTHON=%CADFORGE_PYTHON%
if "%PYTHON%"=="" set PYTHON=python

REM The dataset manifest the studio wrote for a run lives under runs\<id>. Point
REM CADFORGE_DATASET at one of them, or let this pick the newest.
if "%CADFORGE_DATASET%"=="" (
  for /f "delims=" %%f in ('dir /b /o-d "runs\*.json" 2^>nul') do (
    if not defined CADFORGE_DATASET set CADFORGE_DATASET=runs\%%f
  )
)
if "%CADFORGE_DATASET%"=="" (
  echo No dataset manifest found under runs\\.
  echo Start a training run in the studio once, or set CADFORGE_DATASET to a
  echo dataset.json and try again.
  exit /b 1
)

if "%CADFORGE_DATA%"=="" set CADFORGE_DATA=%CD%
if "%CADFORGE_OUT%"=="" set CADFORGE_OUT=%CD%\checkpoints\local
if "%CADFORGE_EPOCHS%"=="" set CADFORGE_EPOCHS=3
if "%CADFORGE_BATCH%"=="" set CADFORGE_BATCH=1
if "%CADFORGE_MAXLEN%"=="" set CADFORGE_MAXLEN=1024
if "%CADFORGE_GRAD_ACCUM%"=="" set CADFORGE_GRAD_ACCUM=16

REM No split setting here on purpose. The train/val side is decided when the
REM studio writes the manifest - from CADFORGE_VAL_PCT in the studio's own
REM environment - and the trainer reads that per-pair `split` field. Setting it
REM here would look like it did something and would not.

echo dataset : %CADFORGE_DATASET%
echo out     : %CADFORGE_OUT%
echo model   : %CADFORGE_MODEL%
echo.

"%PYTHON%" ml\train_vision.py
if errorlevel 1 (
  echo.
  echo Training failed. If it stopped on a missing package, run scripts\setup-ml.cmd.
  exit /b 1
)

echo.
echo Adapter written to %CADFORGE_OUT%\adapter
echo To use it in the studio, set CADFORGE_VISION=python ml\predict.py and restart.
endlocal
