# Commands of the project. `just` lists them; every recipe is a thin alias over cargo and npm so that the same
# command works for every contributor and in CI.
#
# Three executables come out of this repository, one source tree. A TARGET is one of:
#
#   demon (or server)   omud      the daemon: audio engine, control server, web panel
#   cli                 omu       the command-line client
#   gui                 omu-gui   the desktop window
#   all                           the three (the default of build and dev)
#
# `just build [target]` makes release builds, `just dev [target]` makes debug builds (fast to compile, with debug
# information). Cargo itself builds the web panel (npm) for the targets that embed it, so `cargo run` of the daemon or
# of the window is always up to date; the command line needs no Node.js.

# `just` runs recipes with `sh`; on Windows without Git Bash in PATH, use PowerShell instead.
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

# Release build: just build [all|gui|cli|demon|server]
build target="all": (_cargo "build --release" replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon"))

# Same as `build`
release target="all": (build target)

# Debug build (not release): just dev [all|gui|cli|demon|server]
dev target="all": (_cargo "build" replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon"))

# Run a debug build: just run demon --demo     just run cli status     just run gui
run target="demon" *args:
    {{ if target == "cli" { "cargo run --bin omu --no-default-features --features cli -- " + args } else if target == "gui" { "cargo run -p omu-gui" } else if replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon") == "demon" { "cargo run --bin omud -- " + args } else { error("run: unknown target `" + target + "` (use gui, cli, demon or server)") } }}

# The cargo command of a target (private)
[private]
_cargo verb target:
    cargo {{verb}} {{ if target == "all" { "--workspace" } else if target == "gui" { "-p omu-gui" } else if target == "cli" { "--bin omu --no-default-features --features cli" } else if target == "demon" { "--bin omud" } else { error("unknown target `" + target + "`: use all, gui, cli, demon or server") } }}

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
demo:
    cargo run --bin omud -- --demo

# Everything, Rust and web panel
verify-all: verify ui-test

# Remove build output
clean:
    cargo clean
