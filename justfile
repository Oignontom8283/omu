# Commands of the project. `just` lists them; every recipe is a thin alias over cargo and npm so that the same
# command works for every contributor and in CI.
#
# Three executables come out of this repository, one source tree:
#
#   demon/server   omud      the daemon: audio engine, control server, web panel
#   cli            omu       the command-line client
#   gui            omu-gui   the desktop window
#
# `just build [target]` makes release builds, `just dev [target]` makes debug builds (fast to compile, with
# debug information). Without a target, everything is built.

# `just` runs recipes with `sh`; on Windows without Git Bash in PATH, use PowerShell instead.
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

# Release build: just build [all|gui|cli|demon/server]
build target="all": (_panel target) (_cargo "--release" replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon/server"))

# Same as `build`
release target="all": (build target)

# Debug build (not release): just dev [all|gui|cli|demon/server]
dev target="all": (_panel target) (_cargo "" replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon/server"))

# Run a debug build: just run demon/server -- --demo      just run cli -- status      just run gui
run target="demon/server" *args: (dev target)
    {{ if target == "cli" { "cargo run --bin omu --no-default-features --features cli -- " + trim_start_match(args, "-- ") } else if target == "gui" { "cargo run -p omu-gui" } else if replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon/server") == "demon/server" { "cargo run --bin omud -- " + trim_start_match(args, "-- ") } else { error("run: unknown target `" + target + "` (gui, cli or demon/server)") } }}

# The cargo command of a target (private)
[private]
_cargo mode target:
    cargo build {{mode}} {{ if target == "all" { "--workspace" } else if target == "gui" { "-p omu-gui" } else if target == "cli" { "--bin omu --no-default-features --features cli" } else if target == "demon/server" { "--bin omud" } else { error("unknown target `" + target + "`: use all, gui, cli or demon/server") } }}

# The web panel is embedded in the daemon and in the window: build it first (the command line does not need it)
[private]
_panel target:
    {{ if target == "cli" { "echo \"(the command line does not use the web panel)\"" } else { "npm --prefix ui install --no-audit --no-fund --loglevel=error" } }}
    {{ if target == "cli" { "echo \"\"" } else { "npm --prefix ui run build" } }}

# Run every test (Rust, workspace)
test:
    cargo test --workspace

# Fast type-check of everything
check:
    cargo check --workspace --all-targets

# Lints; warnings are errors
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format the code
fmt:
    cargo fmt --all

# Check the formatting without changing files
fmt-check:
    cargo fmt --all --check

# Everything a change must pass before it is committed
verify: fmt-check lint test client-only

# The client-only build: what the command line needs (no audio engine, no Windows API)
client-only:
    cargo check --no-default-features --features cli

# Type-check and test the web panel
ui-test:
    npm --prefix ui run typecheck
    npm --prefix ui test

# Develop the panel with hot reload against a running daemon: open http://localhost:5173
ui-hot:
    npm --prefix ui run dev

# Try everything without real audio: the daemon with a simulated audio system, serving the panel
demo: (dev "demon/server")
    cargo run --bin omud -- --demo

# Everything, Rust and web panel
verify-all: verify ui-test

# Remove build output
clean:
    cargo clean
