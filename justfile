# Commands of the project. `just` lists them; every recipe is a thin alias over cargo and npm so that the same
# command works for every contributor and in CI.
#
# Three executables come out of this repository, one source tree. A TARGET is one of:
#
#   demon (or server)   omud  the daemon: audio engine, control server, web panel
#   cli                 omu-cli     the command-line client
#   gui                 omu-gui     the desktop window
#   all                           the three (the default of build and dev)
#
# `just build [target]` (alias `just release`) makes release builds, `just build [target] native` also tunes them for
# this machine's processor, `just dev [target]` makes debug builds (fast to compile, with debug information) and
# `just run [target]` builds in release mode and runs. Cargo itself builds the web panel (npm) for the targets that embed it, so `cargo run` of the daemon or
# of the window is always up to date; the command line needs no Node.js.

# `just` runs recipes with `sh`; on Windows without Git Bash in PATH, use PowerShell instead.
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

# With `native` the code is also tuned for THIS machine's processor (-C target-cpu=native): a little faster, but the
# executables may crash with an illegal instruction on another, older or different processor. Meant for building from
# source to run on the same computer, not for binaries that are handed to others.

# Release build: just build [all|gui|cli|demon|server] [native]
build target="all" native="": (_cargo "build --release" replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon") native)

# Same as `build`
release target="all" native="": (build target native)

# Debug build (not release): just dev [all|gui|cli|demon|server]
dev target="all": (_cargo "build" replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon") "")

# Build (release) and run: just run demon --demo     just run cli status     just run gui
run target="demon" *args:
    {{ if target == "cli" { "cargo run --release -p app-omu-cli -- " + args } else if target == "gui" { "cargo run --release -p app-omu-gui" } else if replace_regex(target, "^(daemon|demon|server)(/server)?$", "demon") == "demon" { "cargo run --release -p app-omud -- " + args } else { error("run: unknown target `" + target + "` (use gui, cli, demon or server)") } }}

# The cargo command of a target (private). Each executable is a package of its own (src/bin/) that lists the components it
# needs, so building one compiles only those: the command line never compiles the audio engine. `-p` matters: in a workspace
# `--bin` alone would still resolve the features of every member together. The daemon's options (the
# icon, the web panel, the DeepFilterNet code) are default features of its package. `all` builds the three one after the other.
[private]
_cargo verb target native:
    {{ if target == "all" { just_executable() + " _each " + quote(verb) + " " + (if native == "" { "-" } else { quote(native) }) } else { "cargo " + verb + " -p " + (if target == "gui" { "app-omu-gui" } else if target == "cli" { "app-omu-cli" } else if target == "demon" { "app-omud" } else { error("unknown target `" + target + "`: use all, gui, cli, demon or server") }) + (if native == "native" { " --config \"build.rustflags=['-C','target-cpu=native']\"" } else if native == "" { "" } else if native == "-" { "" } else { error("unknown option `" + native + "`: the only one is `native`") }) } }}

[private]
_each verb native: (_cargo verb "cli" native) (_cargo verb "demon" native) (_cargo verb "gui" native)

# Run every test (Rust)
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
    cargo check -p app-omu-cli

# Type-check and test the web panel
ui-test:
    npm --prefix ui run typecheck
    npm --prefix ui test

# Develop the panel with hot reload against a running daemon: open http://localhost:5173
ui-hot:
    npm --prefix ui run dev

# Try everything without real audio: the daemon with a simulated audio system, serving the panel
demo:
    cargo run -p app-omud -- --demo

# Everything, Rust and web panel
verify-all: verify ui-test

# Remove build output
clean:
    cargo clean
