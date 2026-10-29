set shell := ["bash", "-euo", "pipefail", "-c"]

exe := if os_family() == "windows" { ".exe" } else { "" }

lint:
    cargo fmt --check
    cargo run --release --quiet -p engine --bin lint
    cargo clippy --release --all-targets -- -D warnings
    cargo clippy --release -p engine --lib --bin wasm-client --target wasm32-unknown-unknown -- -D warnings
    cargo run --release --quiet -p cli --bin rift -- check editor/templates/new_game

# Every test lives in a tests/ folder; the libs' and bins' own test harnesses would build for nothing.
[doc("The unit and integration tests.")]
test:
    cargo test --release --workspace --test '*'

[doc("Builds the editor and the two game binaries it plays with, then opens it, on FOLDER if given.")]
editor *folder:
    cargo build --release -p editor -p engine --bin editor --bin game-server --bin desktop-client
    target/release/editor {{folder}}

# Shrunk with z + LTO + strip for mobile Safari's per-tab memory. wasm-bindgen's version must match
# the `wasm-bindgen` crate's.
[doc("The browser client as a release ships it, into target/wasm.")]
wasm:
    CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_STRIP=symbols CARGO_PROFILE_RELEASE_LTO=true \
      cargo build --release -p engine --bin wasm-client --target wasm32-unknown-unknown
    rm -rf target/wasm
    wasm-bindgen --target web --no-typescript --out-name rift --out-dir target/wasm target/wasm32-unknown-unknown/release/wasm-client.wasm

# Frame pointers give perf whole call stacks without copying each sampled stack; the profiling
# build keeps the symbols the release binaries are stripped of.
[doc("The running platform's binaries as a release lays them out, into OUT: a binaries folder.")]
dist out="target/dist":
    #!/usr/bin/env bash
    set -euo pipefail
    out={{quote(out)}}
    cargo build --release -p editor -p engine -p cli --bin editor --bin game-server --bin desktop-client --bin rift
    rm -rf "$out" && mkdir -p "$out"
    for binary in editor game-server desktop-client rift; do
      cp "target/release/$binary{{exe}}" "$out/"
    done
    if [ {{quote(os())}} = linux ]; then
      cargo build --release -p web-server
      cp target/release/web-server "$out/"
      RUSTFLAGS="-C force-frame-pointers=yes" cargo build --profile profiling --features profiling -p engine --bin game-server
      cp target/profiling/game-server "$out/game-server-profiling"
      just wasm
      tar -czf "$out/wasm-client.tar.gz" -C target/wasm .
    fi
    if [ {{quote(os_family())}} != windows ]; then
      for binary in editor game-server desktop-client rift web-server; do
        [ ! -f "$out/$binary" ] || strip "$out/$binary"
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
        wasm-client.tar.gz) cp "$file" "$out/" ;;
        *.exe) 7z a -tzip -bso0 -bsp0 "$(cygpath -w "$out/${file%.exe}-{{platform}}.zip")" "$file" ;;
        *) tar -czf "$out/$file-{{platform}}.tar.gz" "$file" ;;
      esac
    done

# Players are congested (all on the spawn tile, one shared view) by default; `--spread` spreads them.
[doc("The server's capacity on a generated game, or on GAME followed by the client's frame rate and memory.")]
bench *args: dist
    target/dist/rift bench {{args}}

[doc("game-server-profiling on a generated game under load, sampled by perf into target/profile.")]
profile *args: dist
    target/dist/rift profile --out target/profile {{args}}

[doc("The rift CLI, run from a binaries folder of this build.")]
rift *args: dist
    target/dist/rift {{args}}
