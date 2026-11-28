set shell := ["bash", "-euo", "pipefail", "-c"]

exe := if os_family() == "windows" { ".exe" } else { "" }

# The profile every recipe builds on: dev, made for iterating, unless `just profile=release <recipe>`
# asks for the optimized build that ships. `archives` and `profile` build release whatever this says.
profile := "dev"
out_dir := if profile == "dev" { "target/debug" } else if profile == "release" { "target/release" } else { error("the profiles are dev and release, not " + profile) }
# What every cargo build here passes, picking its targets after it with `--bin` or `--test`: the
# whole workspace, so every recipe builds the dependencies with the same features and none rebuilds
# bevy and the engine for a feature set of its own, on the one profile.
shared := "--workspace --profile " + profile

[doc("rustfmt, clippy, the repo's own lints, and `rift check` on the editor's templates.")]
lint:
    cargo fmt --check
    cargo build {{shared}} --quiet --bin lint --bin rift
    {{out_dir}}/lint
    cargo clippy {{shared}} --all-targets -- -D warnings
    {{out_dir}}/rift check editor/templates/new_game

[doc("Bakes the maps of the editor's templates, which `just lint` holds to being baked. Needs a GPU.")]
bake-templates:
    cargo build {{shared}} --quiet --bin rift
    {{out_dir}}/rift bake maps editor/templates/new_game

# Every test lives in a tests/ folder; the libs' and bins' own test harnesses would build for nothing.
[doc("The unit and integration tests.")]
test:
    cargo test {{shared}} --test '*'

[doc("Builds the editor and the two game binaries it plays with, then opens it, on FOLDER if given.")]
editor *folder:
    cargo build {{shared}} --bin editor --bin game-server --bin game-client
    {{out_dir}}/editor {{folder}}

# The profiling build keeps the symbols the release binaries are stripped of.
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
    cargo build {{shared}} --bin editor --bin game-server --bin game-client --bin rift --bin web-server
    for binary in editor game-server game-client rift web-server; do
      cp "{{out_dir}}/$binary{{exe}}" "$out/"
    done
    if [ {{quote(os())}} = linux ] && [ {{quote(extras)}} = profiling ]; then
      cargo build {{shared}} --features game-server/profiling --bin game-server
      cp {{out_dir}}/game-server "$out/game-server-profiling"
    fi
    if [ {{profile}} = release ] && [ {{quote(os_family())}} != windows ]; then
      for binary in editor game-server game-client rift web-server; do
        strip "$out/$binary"
      done
    fi

# The one way to a release asset, so what is published is always built on release.
[doc("This platform's binaries built on release, each as a release asset of its own in OUT: <binary>-PLATFORM.tar.gz, or .zip on Windows, holding the bare binary.")]
archives platform out="target/archives":
    #!/usr/bin/env bash
    set -euo pipefail
    just profile=release dist target/dist profiling
    rm -rf {{quote(out)}} && mkdir -p {{quote(out)}}
    out=$(cd {{quote(out)}} && pwd)
    cd target/dist
    for file in *; do
      case "$file" in
        *.exe) 7z a -tzip -bso0 -bsp0 "$(cygpath -w "$out/${file%.exe}-{{platform}}.zip")" "$file" ;;
        *) tar -czf "$out/$file-{{platform}}.tar.gz" "$file" ;;
      esac
    done

# Players are congested (all on the spawn point, one shared view) by default; `--spread` spreads them.
[doc("The server's capacity on a generated game, or on GAME followed by the client's and the editor's frame rates and memory.")]
bench *args: dist
    target/dist/rift bench {{args}}

[doc("Starts the game client from the rift.env the website hands out and checks it draws the game, on the binaries folder BINARIES, else a fresh `just dist`.")]
smoke binaries="":
    {{ if binaries == "" { "just profile=" + profile + " dist" } else { "true" } }}
    cargo build {{shared}} --quiet --bin smoke
    {{out_dir}}/smoke {{ if binaries == "" { "target/dist" } else { quote(binaries) } }}

# It samples the binary that ships, so it builds on release.
[doc("game-server-profiling on a generated game under load, sampled by perf into target/profile.")]
profile *args:
    just profile=release dist target/dist profiling
    target/dist/rift profile --out target/profile {{args}}

[doc("Draws a piece of a game off screen with this build's `rift render`: the dialogue in every style it sets, or an area.")]
render *args: dist
    target/dist/rift render {{args}}

[doc("The rift CLI, run from a binaries folder of this build.")]
rift *args: dist
    target/dist/rift {{args}}

# An edit appends a function the build has never seen: a touched file costs cargo a fraction of what
# an edit does.
[doc("Measuring only, and slow (many minutes; it edits the engine's and the editor's crate roots and puts them back): how long `just lint`, `just test` and `just dist` each take on their own from an up-to-date build, with nothing changed and after an edit to the engine or to the editor. Also writes cargo's timing report of the binaries' build after an engine edit.")]
build-times:
    #!/usr/bin/env bash
    set -euo pipefail
    crates=(engine editor)
    kept=$(mktemp -d)
    for crate in "${crates[@]}"; do cp "$crate/src/lib.rs" "$kept/$crate.rs"; done
    trap 'for crate in "${crates[@]}"; do cp "$kept/$crate.rs" "$crate/src/lib.rs"; done; rm -rf "$kept"' EXIT
    edit() { printf '\npub fn build_times_%s() -> u64 {\n    %s\n}\n' "$(date +%s%N)" "$(date +%s%N)" >> "$1/src/lib.rs"; }
    recipes=(lint test dist)
    each() { local recipe; for recipe in "${recipes[@]}"; do just profile={{profile}} "$recipe" > /dev/null 2>&1; done; }
    printf '%-18s' "after"; printf '%10s' "${recipes[@]}"; echo
    for change in nothing "${crates[@]}"; do
      printf '%-18s' "$change edited"
      for recipe in "${recipes[@]}"; do
        each
        [ "$change" = nothing ] || edit "$change"
        start=$(date +%s.%N)
        just profile={{profile}} "$recipe" > /dev/null 2>&1 || { echo; echo "just $recipe failed" >&2; exit 1; }
        printf '%9.1fs' "$(echo "$(date +%s.%N) - $start" | bc)"
      done
      echo
    done
    each
    edit engine
    cargo build {{shared}} --timings --bin editor --bin game-server --bin game-client --bin rift --bin web-server > /dev/null 2>&1
    echo "cargo's report of the binaries' build after an engine edit: target/cargo-timings/cargo-timing.html"
