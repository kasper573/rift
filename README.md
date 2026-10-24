# rift

A game engine and editor for online RPGs, written in Rust on [Bevy](https://bevyengine.org/), and an
example game built in the open on top of them.

The engine plays whatever game folder it's given: the same server and clients run any game, without a
rebuild. The example game in [`example_game/`](example_game) is a game folder like any other, edited in
the editor and played on the engine's own binaries, and it's the game production runs. The engine and
editor grow by building it: every feature is made for it and proven on it.

## What's here

| Folder | What it is |
| --- | --- |
| [`engine/`](engine) | The engine: the simulation the server and clients share, the game server, the browser (wasm) and desktop clients, and the bench, load test and lint tools. |
| [`editor/`](editor) | The editor: opens a game folder, edits it through the engine's own loader and writer, and play-tests it on the engine's binaries. |
| [`example_game/`](example_game) | The example game: a game folder like any other, and what production runs. |
| [`ui/`](ui) | The interface the editor and the game share: colors, shapes, text roles, fonts and every general-purpose egui component, so both look and behave alike. |
| [`bevy/`](bevy) | Bevy plugins that know nothing of rift: a server-authoritative terminal. |
| [`testkit/`](testkit), [`driver/`](driver), [`showcase/`](showcase) | Testing and showing the game: small games declared in code and a bare world of only the systems a test runs, and real clients driven by real input for the showcase's recorded chapters. |
| [`website/`](website), [`service/`](service), [`docker/`](docker), [`e2e/`](e2e) | The hosted game: the website, the stack it runs in, and the browser suite that tests it. |

## Development

One-time setup:

- [Docker](https://www.docker.com/) — runs the local stack (auth, website, game server,
  observability, HTTPS proxy)
- [Rust](https://rustup.rs/) — rustup installs the repo's pinned toolchain on first build
- `wasm-bindgen-cli` for building the wasm client:
  `cargo install wasm-bindgen-cli --locked`
- `cargo-watch` for the dev loop: `cargo install cargo-watch --locked`
- To record the showcase (`just showcase`): `weston`, `xwayland`, `xdotool`,
  [ffmpeg](https://ffmpeg.org/) built with libass, and a PulseAudio server (PipeWire's will do) with
  its `pactl` and `parecord`. A GPU keeps it fast; without one it renders in software.
- To run the browser suite (`just e2e`): [Node.js](https://nodejs.org/), Google Chrome and `xvfb`. It
  is [Playwright](https://playwright.dev) (in [`e2e/`](e2e)); `just e2e` installs its npm deps on
  first run and drives Chrome on a virtual display, so it's zero-config and never touches your
  desktop. CI fans out across Chrome, Firefox and Safari.
- The editor recordings need `xvfb`, `xdotool` and ffmpeg.

Then:

```sh
just dev
```

This deploys the stack (website serving the baked wasm) and rebuilds the wasm on change. There is
no wasm hot reload: sign in at the printed URL and reload the page to pick up rebuilds.
`just stack` deploys the stack alone; `just reset` tears it down and wipes its data.

To see the game in action without playing it, record its showcase chapters
([`showcase/src/chapters/`](showcase/src/chapters)):

```sh
just showcase combat travel   # one video per chapter: target/showcase/NN-<chapter>.mp4
just showcase                 # all of them, stitched: target/showcase/showcase.mp4
```

The website lives at <https://rift.lan> and Grafana at <https://grafana.rift.lan>. Point `rift.lan` and its
subdomains (`auth.`, `game-server.`, `grafana.`) at 127.0.0.1 in your hosts file, and trust Caddy's local root
certificate to get past the browser's warning.

## Tests

`just test` runs every crate's tests, in seconds once built: unit and integration tests of the
engine's mechanics and the editor, none of which starts a server, a client or the stack. A test
declares the small game it needs in code, with [`testkit`](testkit)'s `Game` (only the files it reads,
loaded through the engine's own loader), and plays it out in `testkit`'s `Sim`: a bare world holding
that game, stepped tick by tick through only the engine systems the test adds, in the server's order.
No test reads [`example_game/`](example_game) or the editor's templates; `just lint` checks that those
load and read exactly as the editor writes them.

Playing the real game is for the showcase and the browser suite alone. The showcase plays every
mechanic through the [`driver`](driver), with desktop clients against a game server, as a player
would. `just e2e` is the browser suite, run against the docker stack: the website and the game as a
browser gets them. Gameplay has a single smoke test there, a new player signing up and appearing in
the game at the spawn location (compared against the reference image beside it in
[`e2e/tests/`](e2e/tests)).

## A game is a folder

A game is a folder of folders, arranged however the game likes, in which each file says what it is
by its name: `fire_priestess.npc.json` is an npc, and the folder `emberfall_shrine.area/` is an area
whose scene and grids are its parts. The game's settings are `fixtures/game.config.json`, and the content
the engine itself asks for has fixed places under `fixtures/`; everything else is found by its path,
so folders only organize. [`example_game/`](example_game) is one. The server loads the folder at
start and serves it; clients sync it and keep it on the device; the same binaries run any folder that
fits their build.

```sh
just editor               # build editor, game-server and desktop-client, open the editor on example_game/
just editor ~/my-game     # …or on another folder
just archive              # the release: the three binaries and the example content, in target/archive
```

The editor edits the folder in place, with undo across every file, and plays it through the two game
binaries it finds beside itself (or in the folder its settings name). New games start from the
templates built into the editor. Art comes in as glTF: drop a `.glb` onto a model or a prop, and the
editor keeps the cues and footprint that still fit the new rig, and watches the file for changes.

A content deploy needs no client build: the server restarts on the new folder, and players reconnect,
sync the changed files and rejoin. The game server's image carries no game. Each stack mounts a game
folder at `/content`: the test stack the example game, production the content folder its deploy syncs.
Another game runs on the same image with its own folder mounted there.

## Release and deploy

Pushing to `main` builds the wasm client and stack images, and deploys them to production. The
website serves the wasm at `/play`; the game server serves the content folder, which the client
syncs at boot and keeps on the device. The same push builds the downloads for Linux, Windows and macOS
(`just release-files` on each). On `main` they're published as one GitHub release per platform, which
the website's downloads page links to.
