# rift

An mmorpg engine and editor, written in Rust on [Bevy](https://bevyengine.org/).

The engine plays whatever game folder it's given: the same server and clients run any game, without a
rebuild. It's released as binaries, and a game is made, played, tested and hosted with those binaries
alone.

> An example game is available at http://k573.dev

## What's here

| Folder | What it is |
| --- | --- |
| [`engine/`](engine) | The engine: the simulation the server and client share, the game server, and the desktop client. |
| [`editor/`](editor) | The editor: opens a game folder, edits it through the engine's own loader and writer, and play-tests it on the engine's binaries. New games start from its templates. |
| [`cli/`](cli) | `rift`, the command line tool: checks, renders, benches, load tests and profiles a game. |
| [`web-server/`](web-server) | A game's website: its pages, sign-in, and the desktop client's downloads. |
| [`oidc/`](oidc), [`service/`](service) | Sign-in through any OpenID Connect provider, and what the HTTP services share: metrics and profiles. |
| [`ui/`](ui) | The interface the editor and the game share: colors, shapes, text roles, fonts and every general-purpose egui component, so both look and behave alike. |
| [`bevy/`](bevy) | Bevy plugins that know nothing of rift: entity replication and a server-authoritative terminal. |
| [`smoke/`](smoke) | The website, the game server and the desktop client, smoke-tested together: a player of a stand-in identity provider starts the client from the `rift.env` the site hands out, and its window must come to look like `smoke/snapshot.png`. |
| [`testkit/`](testkit) | What the tests and the server bench stand on: small games declared in code, and a bare world of only the systems a test runs. |

## Releases

Every push to `main` publishes a release, tagged by its day and commit (`2026.10.29-a06ecda6`). Releases
are never replaced, and each publish thins out the older ones, deleting them along with their tags:

- every release less than 2 days old stays;
- from 2 to 30 days old, only the last release of each day stays;
- after that, only the last release of each month stays.

A game pins a release and moves on when it chooses, within that schedule. Each release holds an archive per
binary and platform, `<binary>-<platform>.tar.gz` (`.zip` on Windows), each holding just the binary:

| Binary | Platforms | What it is |
| --- | --- | --- |
| `game-server` | Linux, Windows, macOS | Runs a game folder for players to join. |
| `desktop-client` | Linux, Windows, macOS | The game, for players. |
| `editor` | Linux, Windows, macOS | Makes and changes games. |
| `rift` | Linux, Windows, macOS | The command line tool. |
| `web-server` | Linux, Windows, macOS | A game's website. |
| `game-server-profiling` | Linux | `game-server` with the symbols and frame pointers a sampling profiler reads. |

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
rift bench <game>                    # …on the game, then the client's and the editor's frame rates and memory (Linux)
rift bench <game> --only memory      # …only the parts named: server, client, editor, memory
rift bench <game> --only scale       # …the spawn area grown side by side, against as many areas: run only when named
rift bench <game> --scenario 'orbit*' --video lowest --repeat 3
                                     # …only the scenarios named, at one preset, each run three times
rift bench <game> --report out.json --compare before.json
                                     # …the report written there, and held against an earlier run's
rift snapshot <game> <shots> <out>   # every shot of a folder drawn into a set of pictures, by adapter and preset
rift compare <baseline> <candidate>  # two sets held against each other, shot by shot, with a report
rift noise <game> <shots> <out>      # every shot drawn again and again, and each shot's allowance from the noise
rift loadtest --http <url> --ws <url> [count]
rift profile --out <dir> [game]      # game-server-profiling under load, sampled by perf (Linux)
rift schema probe                    # the JSON Schema of the desktop client's probe
rift schema server                   # …of what a game server prints once ready, and its test routes
rift schema bench                    # …of the report `rift bench` writes
```

`rift bench <game>` starts `game-server` and `desktop-client` from its binaries folder; it plays a
test character of the game, or a fresh one where the game has none, through a catalogue of
scenarios: each staged on the server, then played as a player would through the client's own
bindings, its input confirmed through the probe. What the scenarios play with comes from the
game's scenarios file, `bench/scenarios.json` in the game's folder unless `--scenarios` names
another, every slot it leaves empty filled from the content:
`places` (the first the home, each a `name`, `area` and `at`), a `light_place`, a `talk_place`, a
`crowd_place`, the ground an `empty_view` looks toward, the `spells` of the home area's climate,
the `windows` to open by their actions, ground to `walk_to` and `far_walk_to`, and the test
`character` to play. The catalogue spans a still view, orbits, tilts, zooms, walks, teleport
arrivals, the interface stormed, the pointer swept, clicks to move, combat, dialogue, the weather's
spells, warp crossings, the worst case, video settings switched live, the window resized, dusk
turning, fifty other players in view, the emptiest view and a ten-minute soak. Before them, the
`desktop` check plays the emptiest view of the engine's own new game at the lowest settings, which
must exceed 300 fps whatever the game benched costs to draw, or the desktop is holding the frames
and the harness fails. Every scenario's frames are held to the same criteria: a mean at or over
the floor of its video preset (60 fps at the highest, 90 at the lowest), at most 1% of frames
slower than the floor's period, no frame past 48 ms, and nothing the view takes in missing. The
client plays on a desktop of the
bench's own: a headless `weston` screen it draws on through `Xwayland`, its window seated there by
`xdotool`, and a silent sink on a PulseAudio-compatible sound server (`pactl`). Its frame rate is
then its own, and nothing reaches the machine's display or speakers; a load average over half the
cores is warned about and kept in the report. The editor plays on
such a desktop too, opened on the area the game starts in and worked as a designer works it, its
input confirmed through its own probe: its opening until the area stands, then the scene tree
hovered, opened, selected, given its menus, dragged in and scrolled; the stage turned, panned and
flicked out and in, what it holds hovered, clicked, dragged and boxed, its menus opened and its
terrain sculpted; the pages and tabs switched, edits undone and redone fifty times, a field typed
into, folders hidden and shown, the panels resized, and the tree hovered between turns of the
stage. Its frames are held to the client's criteria. The run's report, `bench-report.json` unless `--report`
says where, is described by `rift schema bench`; `--compare` holds it against an earlier run's and
fails when a scenario that passed now fails, a mean fell by more than a tenth or a p99 rose by more
than a fifth. The exit code says how the run went: 0 passed, 1 a scenario fell short of its
criteria or the comparison got worse, 2 a scenario could not be played through, 3 a process died.

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

- **`game-server`** reads `RIFT_GAME_SERVER_*`: where it listens (`BIND`, `PORT` for HTTP, `UDP_PORT`
  for play), the address players reach the machine at (`PUBLIC_IP`), the game
  folder (`CONTENT_DIR`), `MAX_CLIENTS`, `PLAYER_SPAWN_POLICY` (`map` or `dist`), `PLAYER_IMMORTAL`,
  `DAYLIGHT` (`turning`; `still`, which holds the game's day at its start hour for tests that
  compare how the world looks; or `fixed`, which holds it at `HOUR`, 0 to 24), `WEATHER` (`turning`,
  or `still`, which holds every area's weather at its first spell), `DAILY_RESET_UTC_HOUR`, `AUTH` (`oidc`, or `fake`, which takes a token's claims without checking
  its signature, so an unsigned JWT signs in any account), `TEST_CHARACTERS`, `EXIT_ON_STDIN_EOF`
  and, optionally, `RNG_SEED`. Started on port 0 it picks free ports, and prints them on stdout
  once players can connect.
- **`desktop-client`** reads `RIFT_CLIENT_*`: the game server (`GAME_SERVER_URL`),
  `AUTH`, `PROFILE_DIR`, `EXIT_ON_STDIN_EOF` and, optionally, `ACCESS_TOKEN` and `PROBE`.
- **`web-server`** reads `RIFT_WEB_SERVER_*`: `BIND`, `PORT`, `REDIRECT_URI`, the game server as
  desktop clients reach it (`GAME_SERVER_URL`) and as it reaches it itself
  (`GAME_SERVER_INTERNAL_URL`), and the release whose desktop client its downloads offer
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
- **Play is netcode over UDP.** A client posts its access token to the game server's `/session`
  and gets a connect token for `PUBLIC_IP` and one of its UDP ports (`UDP_PORT` and the ports after
  it, one for every 1024 of `MAX_CLIENTS`), good for one connection within half a minute;
  the connection is encrypted, and ends once it falls silent for a few seconds. Whatever stands
  between players and the server must pass those UDP ports through as they are.
- **Metrics** are served in the Prometheus exposition format at `/metrics`, and **profiles** in pprof's
  format at `/debug/pprof/profile?seconds=N` (CPU) and `/debug/pprof/heap`, on Linux and macOS. Both
  servers serve them beside their own routes.
- **Logs** go to stdout.
- **The probe.** A desktop client started with `RIFT_CLIENT_PROBE=<host:port>` connects there and
  answers questions about what it shows, one JSON line each: a frame's snapshot, a picture of its
  window, or where the game's things are and how its ground walks. `rift ask <game>` answers the questions about the game
  itself the same way, without a client. The editor started with `--probe <host:port>` answers its
  own: a frame's snapshot (its input, interface, panels, stage and selection), the rows its trees
  drew, and its frames. `rift schema probe` prints the protocol's schema.
- **Test routes.** A game server with `TEST_CHARACTERS` set hands the game's test characters out
  over HTTP, to whoever drives it: `/test/character` makes the asking account's next sign-in play
  one, and `/test/stage` has the character that account plays walk, fight, talk or move.
  `rift schema server` prints their schema.

## Development

One-time setup:

- [Rust](https://rustup.rs/) — rustup installs the repo's pinned toolchain on first build
- `Xvfb` and a Vulkan driver with the `llvmpipe` software rasterizer (Mesa's lavapipe), for `just smoke`
- `perf`, for `just profile`
- `weston`, `Xwayland`, `xdotool` and a PulseAudio-compatible sound server, for `rift bench <game>`

Then:

```sh
just editor           # build editor, game-server and desktop-client, and open the editor
just editor ~/my-game # …on a game folder
just dist             # this machine's binaries, as a release lays them out, in target/dist
just dist target/dist profiling # …and game-server-profiling, on Linux
just bench            # the server bench on the generated game
just lint             # rustfmt, clippy, the repo's own lints, and `rift check` on the editor's templates
just test             # every crate's tests
just smoke            # start the desktop client from the website's rift.env and see it draw the game
```

## Tests

`just test` runs every crate's tests, in seconds once built: unit and integration tests of the
engine's mechanics, the editor and the sign-in, none of which starts a server, a client or a window. A
test declares the small game it needs in code, with [`testkit`](testkit)'s `Game` (only the files it
reads, loaded through the engine's own loader), and plays it out in `testkit`'s `Sim`: a bare world
holding that game, stepped tick by tick through only the engine systems the test adds, in the
server's order. The sign-in is tested against a fake OpenID Connect provider run in the test itself.
