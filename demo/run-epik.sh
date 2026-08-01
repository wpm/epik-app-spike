#!/bin/sh
# The Epik demo, 0.2.0: everything the M3 shell script assembled by hand is now
# the built-in Epik profile, so this only has to say where the Epik checkout is.
#
# What the profile does — persona, EpikMCP, settings isolation, read-only tools
# pre-allowed and mutating ones gated — is in README.md and in
# crates/epik-core/src/profile.rs. The old flags are gone because they were
# describing a policy that now lives in Rust, where it can be tested.

set -e
DIR=$(dirname "$0")
REPO=$(cd "$DIR/.." && pwd)

: "${EPIK_CHECKOUT:=$HOME/Projects/Epik/Epik}"
export EPIK_CHECKOUT

if [ ! -d "$EPIK_CHECKOUT" ]; then
  echo "No Epik checkout at $EPIK_CHECKOUT." >&2
  echo "Set EPIK_CHECKOUT to one. The app still runs without it, but with no" >&2
  echo "persona and no EpikMCP — and it will say so in the window." >&2
fi

# Tauri embeds the frontend at compile time, so it has to exist first.
trunk build --config "$REPO/crates/epik-ui/Trunk.toml"
exec cargo run --manifest-path "$REPO/Cargo.toml" -p epik-app
