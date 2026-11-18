#!/bin/sh
# cadforge - build and start the studio, then open it in your browser.
set -e
if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is not installed. Get it from https://rustup.rs and run this again."
  exit 1
fi
echo "Starting cadforge on http://localhost:8787 ..."
( sleep 3
  open http://localhost:8787 2>/dev/null || xdg-open http://localhost:8787 2>/dev/null || true ) &
exec cargo run --release -- "$@"
