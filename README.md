# rift

A game engine and editor for online RPGs, written in Rust on [Bevy](https://bevyengine.org/).

The engine plays whatever game folder it's given: the same server and clients run any game, without a
rebuild. It's released as binaries, and a game is made, played, tested and hosted with those binaries
alone.

## What's here

| Folder | What it is |
| --- | --- |
| [`engine/`](engine) | The engine: the simulation the server and clients share, the game server, and the browser (wasm) and desktop clients. |
| [`editor/`](editor) | The editor: opens a game folder, edits it through the engine's own loader and writer, and play-tests it on the engine's binaries. New games start from its templates. |
| [`cli/`](cli) | `rift`, the command line tool: checks, renders, benches, load tests and profiles a game. |
| [`web-server/`](web-server) | A game's website: its pages, the browser client, sign-in, and the desktop client's downloads. |
| [`oidc/`](oidc), [`service/`](service) | Sign-in through any OpenID Connect provider, and what the HTTP services share: metrics and profiles. |
| [`ui/`](ui) | The interface the editor and the game share: colors, shapes, text roles, fonts and every general-purpose egui component, so both look and behave alike. |
| [`bevy/`](bevy) | Bevy plugins that know nothing of rift: a server-authoritative terminal. |
| [`testkit/`](testkit) | What the tests and the server bench stand on: small games declared in code, and a bare world of only the systems a test runs. |

## Releases

Every push to `main` publishes a release, tagged by its day and commit (`2026.10.29-a06ecda6`). Releases
are never replaced, so a game can pin one and move on when it chooses. Each release holds an archive per
binary and platform, `<binary>-<platform>.tar.gz` (`.zip` on Windows), each holding just the binary:

| Binary | Platforms | What it is |
| --- | --- | --- |
| `game-server` | Linux, Windows, macOS | Runs a game folder for players to join. |
| `desktop-client` | Linux, Windows, macOS | The game, for players. |
| `editor` | Linux, Windows, macOS | Makes and changes games. |
| `rift` | Linux, Windows, macOS | The command line tool. |
| `web-server` | Linux | A game's website. |
| `game-server-profiling` | Linux | `game-server` with the symbols and frame pointers a sampling profiler reads. |

`wasm-client.tar.gz` holds the browser client, the `.wasm` and its JavaScript glue, which `web-server`
serves at `/wasm`.

Archives extracted into one folder make a **binaries folder**: `editor` and `rift` find `game-server`,
`desktop-client` and `game-server-profiling` beside themselves. `just dist` builds this machine's
binaries into such a folder, `target/dist/`, so an unreleased build runs exactly as a release would.

## The rift tool

```sh
rift check <game>                    # the game loads whole, and every file reads as the editor writes it
rift format <game>                   # rewrites every file that doesn't read as the editor writes it
rift ask <game>                      # the probe's questions about the game itself, without a client
rift render area <game> <shot> <out> # a picture of an area, drawn off screen through the world's own camera
rift bench                           # the server's capacity, on a large game generated in code
rift bench <game>                    # …on the game, then the desktop client's frame rate and memory (Linux)
rift loadtest --http <url> --ws <url> [count]
rift profile --out <dir> [game]      # game-server-profiling under load, sampled by perf (Linux)
rift schema probe                    # the JSON Schema of the desktop client's probe
rift schema server                   # …of what a game server prints once ready, and its test routes
```

`rift bench <game>` starts `game-server` and `desktop-client` from its binaries folder; it plays the
game's first test character, staging each scene on the server. The client plays on a desktop of the
bench's own: a headless `weston` screen it draws on through `Xwayland`, its window seated there by
`xdotool`, and a silent sink on a PulseAudio-compatible sound server (`pactl`). Its frame rate is then its own, and nothing reaches the
machine's display or speakers.

## A game is a folder

A game is a folder of folders, arranged however the game likes, in which each file says what it is
by its name: `fire_priestess.npc.json` is an npc, and the folder `emberfall_shrine.area/` is an area
whose scene and grids are its parts. The game's settings are `fixtures/game.config.json`, and the content
the engine itself asks for has fixed places under `fixtures/`; everything else is found by its path,
so folders only organize. The server loads the folder at start and serves it; clients sync it and keep
it on the device; the same binaries run any folder that fits their build.

The editor edits the folder in place, with undo across every file, and plays it through the two game
binaries it finds beside itself (or in the folder its settings name). Art comes in as glTF: drop a
`.glb` onto a model or a prop, and the editor keeps the cues and footprint that still fit the new rig,
and watches the file for changes.

A content deploy needs no client build: the server restarts on the new folder, and players reconnect,
sync the changed files and rejoin.

## Running a game

Every binary reads its settings from the environment, over a `rift.env` file beside it. Each setting is
required unless it says otherwise, and a binary refuses to start without one.

- **`game-server`** reads `RIFT_GAME_SERVER_*`: where it listens (`BIND`, `PORT`, `WS_PORT`), the game
  folder (`CONTENT_DIR`), `MAX_CLIENTS`, `PLAYER_SPAWN_POLICY` (`map` or `dist`), `PLAYER_IMMORTAL`,
  `DAILY_RESET_UTC_HOUR`, `AUTH` (`oidc`, or `fake`, which takes a token's claims without checking
  its signature, so an unsigned JWT signs in any account), `TEST_CHARACTERS`, `EXIT_ON_STDIN_EOF`
  and, optionally, `RNG_SEED`. Started on port 0 it picks free ports, and prints them on stdout
  once players can connect.
- **`desktop-client`** reads `RIFT_CLIENT_*`: the game server (`GAME_SERVER_URL`, `GAME_SERVER_WS_URL`),
  `AUTH`, `PROFILE_DIR`, `EXIT_ON_STDIN_EOF` and, optionally, `ACCESS_TOKEN` and `PROBE`.
- **`web-server`** reads `RIFT_WEB_SERVER_*`: `BIND`, `PORT`, `REDIRECT_URI`, the game server as
  browsers reach it (`GAME_SERVER_URL`, `GAME_SERVER_WS_URL`) and as it reaches it itself
  (`GAME_SERVER_INTERNAL_URL`), `WASM_DIR`, and the release whose desktop client its downloads offer
  (`DOWNLOADS_URL`, `RELEASE`). The `rift.env` it serves to desktop clients points them at its game
  server and at the sign-in pages in `RIFT_WEB_SERVER_DESKTOP_AUTH_*` (`CLIENT_ID`,
  `AUTHORIZATION_URI`, `SIGN_UP_URI`, `TOKEN_URI`). The site is named after the game its game server
  serves.

### Protocols

The runtimes know no infrastructure, only protocols:

- **Sign-in is OpenID Connect.** `RIFT_AUTH_*` names every endpoint: `ISSUER`, `CLIENT_ID`,
  `JWKS_URI`, `TOKEN_URI`, `AUTHORIZATION_URI`, `SIGN_UP_URI` (a page that takes the same
  authorization request but asks for a new account), `END_SESSION_URI` and, optionally, `ACCOUNT_URI`.
  Each binary reads the ones it uses. A token is taken when its audience holds the client or the
  client is its authorized party, and an account is named by `preferred_username`, else `name`, else
  `sub`.
- **Metrics** are served in the Prometheus exposition format at `/metrics`, and **profiles** in pprof's
  format at `/debug/pprof/profile?seconds=N` (CPU) and `/debug/pprof/heap`, on Linux and macOS. Both
  servers serve them beside their own routes.
- **Logs** go to stdout.
- **The probe.** A desktop client started with `RIFT_CLIENT_PROBE=<host:port>` connects there and
  answers questions about what it shows, one JSON line each: a frame's snapshot, or where the game's
  things are and how its ground walks. `rift ask <game>` answers the questions about the game
  itself the same way, without a client, and `rift schema probe` prints the protocol's schema.
- **Test routes.** A game server with `TEST_CHARACTERS` set hands the game's test characters out
  over HTTP, to whoever drives it: `/test/character` makes the asking account's next sign-in play
  one, and `/test/stage` has the character that account plays walk, fight, talk or move.
  `rift schema server` prints their schema.

## Development

One-time setup:

- [Rust](https://rustup.rs/) — rustup installs the repo's pinned toolchain on first build
- `wasm-bindgen-cli` for building the browser client: `cargo install wasm-bindgen-cli --locked`
- `perf`, for `just profile`
- `weston`, `Xwayland`, `xdotool` and a PulseAudio-compatible sound server, for `rift bench <game>`

Then:

```sh
just editor           # build editor, game-server and desktop-client, and open the editor
just editor ~/my-game # …on a game folder
just dist             # this machine's binaries, as a release lays them out, in target/dist
just bench            # the server bench on the generated game
just lint             # rustfmt, clippy, the repo's own lints, and `rift check` on the editor's templates
just test             # every crate's tests
```

## Tests

`just test` runs every crate's tests, in seconds once built: unit and integration tests of the
engine's mechanics, the editor and the sign-in, none of which starts a server, a client or a window. A
test declares the small game it needs in code, with [`testkit`](testkit)'s `Game` (only the files it
reads, loaded through the engine's own loader), and plays it out in `testkit`'s `Sim`: a bare world
holding that game, stepped tick by tick through only the engine systems the test adds, in the
server's order. The sign-in is tested against a fake OpenID Connect provider run in the test itself.
