set shell := ["bash", "-c"]

compose_file := "docker/docker-compose.yaml"
compose := "docker compose -f " + compose_file + " --profile test"

lint:
    cargo fmt --check
    cargo run --release --quiet -p game --bin lint
    cargo clippy --release --all-targets -- -D warnings
    cargo clippy --release -p game --lib --bin client --target wasm32-unknown-unknown -- -D warnings

test:
    cargo test --release -p game -p ui

# Find the highest area count sustained within the tick budget. Players are congested (all on the
# spawn tile, one shared view) by default; `just bench dist` spreads them for distinct views.
bench mode="":
    RIFT_BENCH_ASSETS_DIR=assets cargo run --release -p game --bin bench -- {{mode}}

loadtest count="dyn":
    RIFT_LOADTEST_HTTP_URL=http://127.0.0.1:9998 RIFT_LOADTEST_WS_URL=ws://127.0.0.1:9999 \
      cargo run --release -p game --bin loadtest -- {{count}}

loadtest-stack-up:
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} -f docker/docker-compose.loadtest.yaml up -d --build --wait

# Rasterize a whole map to an image to preview it (e.g. `just render island island.png`). Defaults
# the output to `<map>.png`.
render map out="":
    cargo run -p bevy_tiled --bin render -- {{map}} {{out}}

build:
    cargo build --release -p website -p game --bin website --bin server
    cargo run --release -p game --bin kc-roles > docker/keycloak/roles.conf

# The browser client: a wasm binary post-processed by wasm-bindgen into the bundle the website serves
# (and bakes into its image). Assets are embedded in the binary, so there's nothing else to ship. The
# dev loop builds it plain and fast; the deploy alone re-runs this with LTO + size opt-level + strip
# (env, see ci.yml) to roughly halve the bundle for mobile Safari's per-tab memory budget.
wasm:
    cargo build --release -p game --bin client --target wasm32-unknown-unknown
    wasm-bindgen --target web --no-typescript --out-name rift --out-dir target/wasm \
      target/wasm32-unknown-unknown/release/client.wasm

stack: build wasm stack-up

# --profile dev adds the grafana/loki/tempo/... stack (opt-in; see the e2e-stack-up note).
stack-up:
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} --profile dev up -d --build --wait

prewarm:
    {{compose}} pull --ignore-buildable || true
    {{compose}} build keycloak reverse-proxy

# No service names: compose's build sections are the single source of truth for what we publish.
push-images:
    docker compose -f {{compose_file}} --profile prod build
    docker compose -f {{compose_file}} --profile prod push

logs:
    {{compose}} logs --no-color

kc-provision:
    cargo run --release -p game --bin kc-roles > docker/keycloak/roles.conf
    {{compose}} up -d --build keycloak

# Bring the stack up (website serving the baked wasm), then rebuild the wasm and refresh just the
# website image on every change. There's no wasm hot reload: sign in at the printed URL and reload
# the page to pick up a rebuild.
dev: stack
    #!/usr/bin/env bash
    set -euo pipefail
    command -v cargo-watch >/dev/null || { echo "just dev needs cargo-watch: cargo install cargo-watch --locked"; exit 1; }
    set -a; source docker/.env.test; set +a
    echo "sign in and play at https://${RIFT_DOMAIN}/play"
    cargo watch -w game -w ui -w bevy \
      -s 'just wasm && {{compose}} up -d --build --wait rift-website'

# `down -v` wipes volumes: keycloak only imports its realm on first boot, so realm changes
# (e.g. redirect URIs) don't apply until the DB is gone.
reset:
    {{compose}} down -v

# Build, bring up a fresh stack, and run the Playwright suite (e2e/). FILTER limits which tests run.
e2e filter="": build wasm e2e-stack-up (e2e-run filter)

# The "test" profile leaves out the observability stack (it's opt-in via the "dev" profile, which
# only stack-up activates) — the e2e doesn't exercise it, and pulling/starting it is the bulk of the
# stack's startup. Everything else comes up, so new game services are picked up automatically.
[private]
e2e-stack-up:
    docker network inspect rift >/dev/null 2>&1 || docker network create rift
    {{compose}} up -d --build --wait

e2e-run filter="":
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; source docker/.env.test; set +a
    cd e2e
    [ -d node_modules ] || npm ci
    export LP_NUM_THREADS=2 # bound each headed browser's Mesa threads so parallel workers don't thrash
    if [ "${E2E_ALL_BROWSERS:-}" = "1" ]; then
      xvfb-run -a npx playwright test {{filter}}
    else
      npx playwright test {{filter}}
    fi

demo_dir := "target/demo"

# Record demo chapters (e2e/demos/, one per file) of the real game against a fresh stack, each to
# target/demo/<chapter>.webm. FILTER picks chapters by file name, e.g. `just demo combat`.
demo filter="": build wasm e2e-stack-up (demo-run filter)

# Every chapter in file-name order, stitched into one video of the whole game: target/demo/showcase.webm.
showcase: build wasm e2e-stack-up
    rm -f {{demo_dir}}/[0-9]*.webm
    just demo-run
    cd {{demo_dir}} && ls [0-9]*.webm | sed "s/.*/file '&'/" > chapters.txt \
      && ffmpeg -v error -y -f concat -i chapters.txt -c copy showcase.webm && rm chapters.txt
    @echo "{{demo_dir}}/showcase.webm"

demo-run filter="":
    #!/usr/bin/env bash
    set -euo pipefail
    set -a; source docker/.env.test; set +a
    export RIFT_DEMO_DIR="$PWD/{{demo_dir}}"
    cd e2e
    [ -d node_modules ] || npm ci
    npx playwright test -c demo.config.ts {{filter}}

# Record the ui crate's component gallery opened on SCENE (by its tab name), on a private virtual
# display, to target/demo/gallery-<scene>.webm. INPUT is an xdotool command chain played against it
# while recording, in window pixels, e.g. `just gallery-demo "toasts (sonner)" 8 "mousemove 800 523 click 1"`.
gallery-demo scene seconds="6" input="":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p ui --bin gallery
    mkdir -p {{demo_dir}}
    out="{{demo_dir}}/gallery-$(echo '{{scene}}' | tr 'A-Z' 'a-z' | tr -cs 'a-z0-9' '-' | sed 's/^-//;s/-$//').webm"
    xvfb-run -a -s "-screen 0 2560x1440x24" bash -euo pipefail -c '
      target/release/gallery "$1" & app=$!
      window=$(xdotool search --sync --name "rift ui gallery" | head -1)
      xdotool windowmove --sync "$window" 0 0
      eval "$(xdotool getwindowgeometry --shell "$window")"
      # The window maps before bevy has rendered into it.
      sleep 2
      ffmpeg -v error -y -f x11grab -framerate 30 -video_size "${WIDTH}x${HEIGHT}" -i "$DISPLAY+$X,$Y" \
        -t "$2" -c:v libvpx -b:v 4M -deadline realtime -cpu-used 8 "$3" & recording=$!
      [ -z "$4" ] || xdotool $4
      wait $recording
      kill $app' _ '{{scene}}' '{{seconds}}' "$out" '{{input}}'
    echo "$out"

# Frames sampled evenly across VIDEO, tiled into one image beside it (<video>.png): a recording at a
# glance, e.g. to check a demo shows what it should before sharing it.
contact-sheet video:
    #!/usr/bin/env bash
    set -euo pipefail
    seconds=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "{{video}}")
    ffmpeg -v error -y -i "{{video}}" -vf "fps=16/$seconds,scale=640:-1,tile=4x4:padding=4" -frames:v 1 \
      "{{without_extension(video)}}.png"
    echo "{{without_extension(video)}}.png"
