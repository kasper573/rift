set shell := ["bash", "-euo", "pipefail", "-c"]

compose := "docker compose -f docker/docker-compose.yaml -f docker/docker-compose.test.yaml --profile test"
showcase_dir := "target/showcase"

lint:
    cargo fmt --check
    cargo run --release --quiet -p engine --bin lint
    cargo clippy --release --all-targets -- -D warnings
    cargo clippy --release -p engine --lib --bin client --target wasm32-unknown-unknown -- -D warnings

# Every test lives in a tests/ folder; the libs' and bins' own test harnesses would build for nothing.
test:
    cargo test --release -p engine -p editor --test '*'

# The website and game server binaries, and the roles the auth stack syncs from.
build:
    cargo build --release -p website -p engine --bin website --bin game-server

# The browser client bundle the website serves; content syncs from the game server at boot.
wasm:
    cargo build --release -p engine --bin client --target wasm32-unknown-unknown
    wasm-bindgen --target web --no-typescript --out-name rift --out-dir target/wasm target/wasm32-unknown-unknown/release/client.wasm

# Builds the editor and the two game binaries it starts, then opens the editor on FOLDER.
editor folder="example_game":
    cargo build --release -p editor -p engine --bin editor --bin game-server --bin desktop-client
    target/release/editor {{quote(folder)}}

# The archive a release ships: the three binaries and the example content, for the running platform.
archive out="target/archive":
    cargo build --release -p editor -p engine --bin editor --bin game-server --bin desktop-client
    rm -rf {{quote(out)}} && mkdir -p {{quote(out)}}
    cp target/release/{editor,game-server,desktop-client}{{ if os_family() == "windows" { ".exe" } else { "" } }} {{quote(out)}}
    cp -rL example_game {{quote(out / "content")}}

# The archive's pieces as separate downloads: the editor, game server and desktop client, stripped of
# the symbol tables a profiler reads, the example game's content folder as rift-example-game.zip, and
# the rift.env that sets the server and client up to play on DOMAIN.
release-files domain out="target/release-files": (archive "target/archive")
    #!/usr/bin/env bash
    set -euo pipefail
    [ -n {{quote(domain)}} ] || { echo "release-files: no domain" >&2; exit 1; }
    rm -rf {{quote(out)}} && mkdir -p {{quote(out)}}
    out=$(cd {{quote(out)}} && pwd)
    cd target/archive
    cp editor* game-server* desktop-client* "$out"
    {{ if os_family() == "windows" { "" } else { 'strip "$out/editor" "$out/game-server" "$out/desktop-client"' } }}
    {{ if os_family() == "windows" { '7z a -tzip -bso0 -bsp0 "$(cygpath -w "$out/rift-example-game.zip")" content' } else { 'zip -qr "$out/rift-example-game.zip" content' } }}
    cat > "$out/rift.env" <<'EOF'
    # Read by desktop-client and game-server in this folder, set up to play on {{domain}}.
    # Every line is explained at https://{{domain}}/downloads#rift-env

    RIFT_CLIENT_GAME_SERVER_URL=https://game-server.{{domain}}
    RIFT_CLIENT_GAME_SERVER_WS_URL=wss://game-server.{{domain}}
    RIFT_CLIENT_AUTH=oidc
    RIFT_CLIENT_PROFILE_DIR=profile
    RIFT_CLIENT_EXIT_ON_STDIN_EOF=false

    RIFT_AUTH_ISSUER=https://auth.{{domain}}/realms/rift
    RIFT_AUTH_AUDIENCE=rift
    RIFT_AUTH_JWKS_URI=https://auth.{{domain}}/realms/rift/protocol/openid-connect/certs
    RIFT_AUTH_TOKEN_URI=https://auth.{{domain}}/realms/rift/protocol/openid-connect/token

    RIFT_GAME_SERVER_BIND=0.0.0.0
    RIFT_GAME_SERVER_PORT=9998
    RIFT_GAME_SERVER_WS_PORT=9999
    RIFT_GAME_SERVER_MAX_CLIENTS=8192
    RIFT_GAME_SERVER_CONTENT_DIR=content
    RIFT_GAME_SERVER_PLAYER_SPAWN_TYPE=map
    RIFT_GAME_SERVER_PLAYER_IMMORTAL=false
    RIFT_GAME_SERVER_DAILY_RESET_UTC_HOUR=4
    RIFT_GAME_SERVER_AUTH=oidc
    RIFT_GAME_SERVER_TEST_CHARACTERS=off
    RIFT_GAME_SERVER_EXIT_ON_STDIN_EOF=false
    RIFT_GAME_SERVER_PYROSCOPE_ENABLED=false
    RIFT_GAME_SERVER_PYROSCOPE_SAMPLE_HZ=99
    EOF

# The local stack, with its observability services.
stack: build wasm (up "--profile dev")

# The stack, rebuilding the wasm and the website on every change: reload the page to pick one up.
dev: stack
    source docker/.env.test && echo "sign in and play at https://$RIFT_DOMAIN/play"
    cargo watch -w engine -w design_tokens -w bevy -s 'just wasm && {{compose}} up -d --build --wait rift-website'

# Tears the stack down and wipes its data, so keycloak imports its realm again on the next start.
reset:
    {{compose}} down -v

logs:
    {{compose}} logs --no-color

# No service names: compose's build sections are the single source of truth for what we publish.
[doc("Builds and pushes the production images.")]
push-images:
    docker compose -f docker/docker-compose.yaml --profile prod build
    docker compose -f docker/docker-compose.yaml --profile prod push

# Builds, brings up the stack and runs the Playwright suite (e2e/). FILTER limits which tests run.
e2e filter="": build wasm e2e-stack-up (e2e-run filter)

# The stack without its observability services, which the e2e doesn't exercise and which start slowest.
e2e-stack-up: (up "")

# The e2e stack but the website, which packages the wasm: all it waits on can start while that compiles.
e2e-stack-ahead: (up "" "rift-game-server reverse-proxy")

# The e2e builds two ways. deploy: what production ships, the server at the release default and the
# wasm shrunk with z + LTO + strip for mobile Safari's per-tab memory. fast: the server unoptimized
# and the wasm without LTO, which takes minutes even with every dependency built.
e2e-server flavor:
    {{ if flavor == "deploy" { "" } else if flavor == "fast" { "CARGO_PROFILE_RELEASE_OPT_LEVEL=0" } else { error("flavor: deploy or fast") } }} just build

e2e-client flavor:
    CARGO_PROFILE_RELEASE_OPT_LEVEL=z CARGO_PROFILE_RELEASE_STRIP=symbols {{ if flavor == "deploy" { "CARGO_PROFILE_RELEASE_LTO=true" } else if flavor == "fast" { "" } else { error("flavor: deploy or fast") } }} just wasm

# The e2e stack's images that don't package the build, so they can be made while it compiles.
e2e-stack-images:
    {{compose}} build keycloak reverse-proxy
    {{compose}} pull postgres

# LP_NUM_THREADS bounds each headed browser's Mesa threads so parallel workers don't thrash.
[doc("The Playwright suite against a running stack, on a virtual display.")]
e2e-run filter="":
    LP_NUM_THREADS=2 xvfb-run -a just playwright {{filter}}

# Records showcase chapters, all at once as far as the cores go, to target/showcase/NN-<chapter>.mp4:
# `just showcase combat travel`. Naming none records every chapter and stitches them into
# target/showcase/showcase.mp4.
showcase *chapters:
    cargo build --release -p showcase -p engine --bin showcase --bin game-server --bin desktop-client
    RIFT_SHOWCASE_CONTENT_DIR=$PWD/example_game RIFT_SHOWCASE_OUT_DIR=$PWD/{{showcase_dir}} RIFT_SHOWCASE_CORES_PER_CHAPTER=5 \
      target/release/showcase {{chapters}}

[doc("Records the editor on a copy of the example content at SCENE, a JSON array of steps, as editor-NAME.webm; HOOK runs with WORK naming the copy's parent.")]
editor-showcase name scene seconds="8" input="" hook="":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p editor
    export WORK=$(mktemp -d)
    trap 'rm -rf "$WORK"' EXIT
    cp -rL example_game "$WORK/game"
    just record {{quote(showcase_dir / "editor-" + kebabcase(name) + ".webm")}} "Rift editor" {{seconds}} {{quote(input)}} {{quote(hook)}} \
      target/release/editor "$WORK/game" --scene {{quote(scene)}} --user-folder "$WORK/user"

# Frames sampled evenly across VIDEO, tiled into one image beside it: a recording at a glance.
contact-sheet video:
    ffmpeg -v error -y -i {{quote(video)}} -frames:v 1 \
      -vf "fps=16/$(ffprobe -v error -show_entries format=duration -of csv=p=0 {{quote(video)}}),scale=640:-1,tile=4x4:padding=4" \
      {{quote(without_extension(video) + ".png")}}

# Empty NAMES or ANIMATIONS mean all.
[doc("Films actors in all eight directions through the game's sprite renderer: `just render-actor adventurer,dire-wolf walk,attack 2`.")]
render-actor names="" animations="" loops="4":
    RIFT_RENDER_CONTENT_DIR=example_game cargo run --quiet -p engine --bin render-actor -- {{quote(names)}} {{quote(animations)}} {{loops}} \
      target/render/actor-{{ if names == "" { "all" } else { replace(names, ",", "+") } }}-{{ if animations == "" { "all" } else { replace(animations, ",", "+") } }}-x{{loops}}.mp4

# Players are congested (all on the spawn tile, one shared view) by default; `dist` spreads them for distinct views.
[doc("The server's highest area count sustained within the tick budget, then the client's frame rate through a playthrough.")]
bench mode="":
    RIFT_BENCH_CONTENT_DIR=example_game cargo run --release -p engine --bin bench -- {{mode}}
    cargo bench -p engine --bench client

loadtest count="dyn":
    RIFT_LOADTEST_HTTP_URL=http://127.0.0.1:9998 RIFT_LOADTEST_WS_URL=ws://127.0.0.1:9999 \
      cargo run --release -p engine --bin loadtest -- {{count}}

loadtest-stack-up: (up "-f docker/docker-compose.loadtest.yaml")

# The content copy is synced in place, never replaced, so a running game server keeps the folder it mounted.
[private]
up flags services="":
    mkdir -p target/stack/content && rsync -aL --delete example_game/ target/stack/content/
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} {{flags}} up -d --build --wait {{services}}

# Films COMMAND's window titled TITLE on a private virtual display to OUT, playing INPUT (an xdotool
# command chain in window pixels) and running HOOK (a shell line) while it records.
[private]
[positional-arguments]
record out title seconds input hook +command:
    #!/usr/bin/env bash
    set -euo pipefail
    export out=$1 title=$2 seconds=$3 input=$4 hook=$5
    shift 5
    mkdir -p "$(dirname "$out")"
    xvfb-run -a -s "-screen 0 2560x1440x24" bash -euo pipefail -c '
      "$@" & app=$!
      window=$(xdotool search --sync --name "$title" | head -1)
      xdotool windowmove --sync "$window" 0 0
      xdotool windowfocus --sync "$window"
      eval "$(xdotool getwindowgeometry --shell "$window")"
      # The window maps before bevy has rendered into it.
      sleep 2
      ffmpeg -v error -y -f x11grab -framerate 30 -video_size "${WIDTH}x${HEIGHT}" -i "$DISPLAY+$X,$Y" \
        -t "$seconds" -c:v libvpx -b:v 4M -deadline realtime -cpu-used 8 "$out" & recording=$!
      [ -z "$hook" ] || bash -c "$hook" &
      [ -z "$input" ] || xdotool $input
      wait $recording
      kill $app' _ "$@"
    echo "$out"

[private]
playwright *args:
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; source docker/.env.test; set +a
    cd e2e
    [ -d node_modules ] || npm ci
    npx playwright test {{args}}
