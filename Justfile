set shell := ["bash", "-euo", "pipefail", "-c"]

exe := if os_family() == "windows" { ".exe" } else { "" }

# Lint and test build the dev profile: debug assertions on, and no release rebuild of the engine for
# every edit.
[doc("rustfmt, clippy, the repo's own lints, and `rift check` on the editor's templates.")]
lint:
    cargo fmt --check
    cargo run --quiet -p lint
    cargo clippy --all-targets -- -D warnings
    cargo run --quiet -p cli --bin rift -- check editor/templates/new_game

# Every test lives in a tests/ folder, and every example in the docs compiles and runs; the libs' and
# bins' own test harnesses would build for nothing.
[doc("The unit and integration tests, and the examples in the docs.")]
test:
    cargo test --workspace --test '*'
    cargo test --workspace --doc

[doc("Builds the editor and the two game binaries it plays with, then opens it, on FOLDER if given.")]
editor *folder:
    cargo build --release -p editor -p engine -p game-server --bin editor --bin game-server --bin desktop-client
    target/release/editor {{folder}}

# Frame pointers give perf whole call stacks without copying each sampled stack; the profiling
# build keeps the symbols the release binaries are stripped of.
[doc("The running platform's binaries as a release lays them out, into OUT: a binaries folder. EXTRAS may add profiling (game-server-profiling), which builds on Linux only.")]
dist out="target/dist" *extras="":
    #!/usr/bin/env bash
    set -euo pipefail
    out={{quote(out)}}
    case {{quote(extras)}} in
      "" | profiling) ;;
      *) echo "no extra {{extras}}: the only extra is profiling" >&2; exit 1 ;;
    esac
    rm -rf "$out" && mkdir -p "$out"
    # One build for every binary: each target's build script caches whether the tree was dirty
    # when it last ran, so left to themselves the server and the client can disagree.
    dirty=$(git status --porcelain --untracked-files=no)
    export RIFT_BUILD="${RIFT_BUILD:-$(git rev-parse --short=8 HEAD)${dirty:+-dirty}}"
    cargo build --release -p editor -p engine -p game-server -p cli -p web-server --bin editor --bin game-server --bin desktop-client --bin rift --bin web-server
    for binary in editor game-server desktop-client rift web-server; do
      cp "target/release/$binary{{exe}}" "$out/"
    done
    if [ {{quote(os())}} = linux ] && [ {{quote(extras)}} = profiling ]; then
      RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling --features profiling -p game-server --bin game-server
      cp target/profiling/game-server "$out/game-server-profiling"
    fi
    if [ {{quote(os_family())}} != windows ]; then
      for binary in editor game-server desktop-client rift web-server; do
        strip "$out/$binary"
      done
    fi

[doc("Each binary of BINARIES as a release asset of its own: <binary>-PLATFORM.tar.gz, or .zip on Windows, holding the bare binary.")]
archives platform binaries="target/dist" out="target/archives":
    #!/usr/bin/env bash
    set -euo pipefail
    rm -rf {{quote(out)}} && mkdir -p {{quote(out)}}
    out=$(cd {{quote(out)}} && pwd)
    cd {{quote(binaries)}}
    for file in *; do
      case "$file" in
        *.exe) 7z a -tzip -bso0 -bsp0 "$(cygpath -w "$out/${file%.exe}-{{platform}}.zip")" "$file" ;;
        *) tar -czf "$out/$file-{{platform}}.tar.gz" "$file" ;;
      esac
    done

# Players are congested (all on the spawn point, one shared view) by default; `--spread` spreads them.
[doc("The server's capacity on a generated game, or on GAME followed by the client's frame rate and memory.")]
bench *args: dist
    target/dist/rift bench {{args}}

[doc("Starts the desktop client from the rift.env the website hands out and checks it draws the game, on the binaries folder BINARIES, else a fresh `just dist`.")]
smoke binaries="":
    {{ if binaries == "" { "just dist" } else { "true" } }}
    cargo run --release --quiet -p smoke -- {{ if binaries == "" { "target/dist" } else { quote(binaries) } }}

[doc("game-server-profiling on a generated game under load, sampled by perf into target/profile.")]
profile *args: (dist "target/dist" "profiling")
    target/dist/rift profile --out target/profile {{args}}

[doc("The rift CLI, run from a binaries folder of this build.")]
rift *args: dist
    target/dist/rift {{args}}
