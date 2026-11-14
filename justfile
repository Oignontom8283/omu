# Commands of the project. `just` lists them; every recipe is a thin alias over cargo so that the same
# command works for every contributor and in CI.

# `just` runs recipes with `sh`; on Windows without Git Bash in PATH, use PowerShell instead.
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

# Build every binary (daemon and command line)
build:
    cargo build --all-targets

# Build in release mode
release:
    cargo build --release

# Run every test
test:
    cargo test

# Fast type-check of everything
check:
    cargo check --all-targets

# Lints; warnings are errors
lint:
    cargo clippy --all-targets -- -D warnings

# Format the code
fmt:
    cargo fmt

# Check the formatting without changing files
fmt-check:
    cargo fmt --check

# Everything a change must pass before it is committed
verify: fmt-check lint test client-only

# The client-only build: what the command line and the Tauri window need (no audio engine, no Windows API)
client-only:
    cargo check --no-default-features --features cli

# Start the daemon (arguments after `--` go to it): just daemon -- --help
daemon *args:
    cargo run --bin omud -- {{args}}

# Run the command-line client: just cli -- status
cli *args:
    cargo run --bin omu -- {{args}}

# Install the web panel's dependencies (needs Node.js)
ui-install:
    npm --prefix ui install

# Build the web panel into ui/dist; the daemon serves it (rebuild the daemon in release mode to embed it)
ui-build:
    npm --prefix ui run build

# Develop the panel with hot reload against a running daemon: open http://localhost:5173
ui-dev:
    npm --prefix ui run dev

# Type-check and test the web panel
ui-test:
    npm --prefix ui run typecheck
    npm --prefix ui test

# Try everything without real audio: the daemon with a simulated audio system, serving the panel
demo:
    cargo run --bin omud -- --demo

# Everything, Rust and web panel
verify-all: verify ui-test

# Remove build output
clean:
    cargo clean
