# Commands of the project. `just` lists them; every recipe is a thin alias over cargo and npm so that the same
# command works for every contributor and in CI.
#
# Three executables come out of this repository, one source tree. A TARGET is one of:
#
#   demon (or server)   omu-daemon  the daemon: audio engine, control server, web panel
#   cli                 omu-cli     the command-line client
#   gui                 omu-gui     the desktop window
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
    {{ if target == "cli" { "cargo run --bin omu-cli --no-default-features --features cli -- " + args } else if target == "gui" { "cargo run --bin omu-gui --no-default-features --features gui" } else if replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon") == "demon" { "cargo run --bin omu-daemon --no-default-features --features daemon -- " + args } else { error("run: unknown target `" + target + "` (use gui, cli, demon or server)") } }}

# The cargo command of a target (private). `all` builds the targets one after the other, each with exactly its own
# features: cargo unifies the features of everything built by one command, so building them together would give the
# command-line client the daemon's code (and size).
[private]
_cargo verb target:
    {{ if target == "all" { just_executable() + " _each " + quote(verb) } else if target == "gui" { "cargo " + verb + " --bin omu-gui --no-default-features --features gui" } else if target == "cli" { "cargo " + verb + " --bin omu-cli --no-default-features --features cli" } else if target == "demon" { "cargo " + verb + " --bin omu-daemon --no-default-features --features daemon" } else { error("unknown target `" + target + "`: use all, gui, cli, demon or server") } }}

[private]
_each verb: (_cargo verb "cli") (_cargo verb "demon") (_cargo verb "gui")

# Run every test (Rust, every feature)
test:
    cargo test --all-features

# Fast type-check of everything
check:
    cargo check --all-features --all-targets

# Lints; warnings are errors
lint:
    cargo clippy --all-features --all-targets -- -D warnings

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
    cargo run --bin omu-daemon -- --demo

# Everything, Rust and web panel
verify-all: verify ui-test

# Remove build output
clean:
    cargo clean
