# Commands of the project: `just` lists them. Each one is a short alias over cargo and npm, so that it is the same for every
# contributor and in CI.
#
#   just build     compile everything (debug)       just test      run all the tests
#   just gui       run the desktop window           just ci        what a change must pass before it is committed
#   just demo      run the daemon on simulated audio
#
# Three programs come out of this repository: the daemon (omud), the command line (omu-cli) and the window (omu-gui).
#
# Why `gui`, `daemon` and `cli` start the built program instead of calling `cargo run`: cargo chooses the options of every
# dependency for the command as a whole, so `cargo run --bin omu-gui` compiles its own copy of dependencies that `just build`
# (the whole workspace) already compiled. Every recipe below that compiles uses the whole workspace, so that they all share one copy.

# `just` runs recipes with `sh`; on Windows without Git Bash in PATH, use PowerShell instead.
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# Where cargo puts what it builds, and the ending of a program's file name.
debug := if env('CARGO_TARGET_DIR', '') == '' { './target/debug' } else { env('CARGO_TARGET_DIR') / 'debug' }
optimised := if env('CARGO_TARGET_DIR', '') == '' { './target/release' } else { env('CARGO_TARGET_DIR') / 'release' }
ext := if os() == "windows" { ".exe" } else { "" }

default:
    @just --list --unsorted

# Compile everything (debug)
build:
    cargo build --workspace --all-targets

# Run the desktop window
gui: build
    {{ debug }}/omu-gui{{ ext }}

# Run the daemon: just daemon --help
daemon *args: build
    {{ debug }}/omud{{ ext }} {{ args }}

# Run the command line: just cli status
cli *args: build
    {{ debug }}/omu-cli{{ ext }} {{ args }}

# Run the daemon on a simulated audio system, to try everything without touching real audio
demo: build
    {{ debug }}/omud{{ ext }} --demo

# Run the tests (Rust, then the web panel)
test:
    cargo test --workspace
    npm --prefix ui run typecheck
    npm --prefix ui test

# Fast type-check of everything
check:
    cargo check --workspace --all-targets

# Lints; warnings are errors
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Format the code
fmt:
    cargo fmt --all

# Everything a change must pass before it is committed: formatting, lints, the command line alone, then the tests
ci: && test
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo check --bin omu-cli

# Optimised builds, in their own folder
release:
    cargo build --release --workspace

# Run the desktop window, optimised (what to use every day: the debug build is much slower)
gui-release: release
    {{ optimised }}/omu-gui{{ ext }}

# Run the daemon, optimised: just daemon-release --help
daemon-release *args: release
    {{ optimised }}/omud{{ ext }} {{ args }}

# Run the command line, optimised: just cli-release status
cli-release *args: release
    {{ optimised }}/omu-cli{{ ext }} {{ args }}

# Run the daemon on simulated audio, optimised
demo-release: release
    {{ optimised }}/omud{{ ext }} --demo

# Tuned for THIS machine's processor: a little faster, but the programs may crash with an illegal instruction on another, older
# or different one. Only for building from source to run on the same computer, never for programs that are handed to others.

# Optimised builds tuned for this machine's processor
release-native:
    cargo build --release --workspace --config "build.rustflags=['-C','target-cpu=native']"

# Everything that is handed to people, in dist/ (what a release or the CI runs): the optimised programs of each target, then the installer
# or package made of them. One recipe per target, to build just that one (a tester gets package-win-x64 and nobody compiles ARM for it).
# The Linux packages (deb, rpm) and the macOS one will join the list here when they exist; each lives in its own folder of scripts/.
# The ARM64 one needs a C compiler for ARM64 (the build server has clang; on a PC: the ARM64 build tools of Visual Studio): to build only what
# works on a given computer, run the recipe of that target.

# Build everything that is handed to people, in dist/
package: package-win-x64 package-win-arm64

# The Windows installer for Intel and AMD processors (dist/omu-<version>-setup-x64.exe)
package-win-x64 *args:
    rustup target add x86_64-pc-windows-msvc
    cargo build --release --target x86_64-pc-windows-msvc --bin omud --bin omu-cli --bin omu-gui
    cargo run -p omu-win-installer -- --arch x64 {{ args }}

# The Windows installer for ARM processors (dist/omu-<version>-setup-arm64.exe). Compiling for ARM needs the ARM64 build tools of Visual Studio
# (the component "MSVC ... ARM64 build tools"); the Rust target is added by the recipe itself.
package-win-arm64 *args:
    rustup target add aarch64-pc-windows-msvc
    cargo build --release --target aarch64-pc-windows-msvc --bin omud --bin omu-cli --bin omu-gui
    cargo run -p omu-win-installer -- --arch arm64 {{ args }}

# Both Windows installers
package-win *args: (package-win-x64 args) (package-win-arm64 args)

# Develop the web panel with hot reload against a running daemon: open http://localhost:5173
ui:
    npm --prefix ui run dev

# Install the web panel's dependencies (once, and when they change)
setup:
    npm --prefix ui ci

# Remove what was built
clean:
    cargo clean
