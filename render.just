# Rasterize a whole map to an image to preview it (e.g. `just render map island island.png`).
# Defaults the output to `<map>.png`.
map map out="":
    cargo run -p bevy_tiled --bin render -- {{map}} {{out}}

# Film actors' animations through the game's own sprite renderer into an MP4 under target/render,
# all eight directions side by side, each animation looped `loops` times (e.g.
# `just render actor adventurer,dire-wolf walk,attack 2`). Empty names or animations mean all.
actor names="" animations="" loops="4":
    #!/usr/bin/env bash
    set -euo pipefail
    slug() { if [ -n "$1" ]; then echo "${1//,/+}"; else echo all; fi; }
    out="target/render/actor-$(slug '{{names}}')-$(slug '{{animations}}')-x{{loops}}.mp4"
    RIFT_RENDER_ASSETS_DIR=assets cargo run --quiet -p game --bin render-actor -- \
      '{{names}}' '{{animations}}' '{{loops}}' "$out"
