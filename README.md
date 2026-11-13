# rift

An mmorpg engine and editor, written in Rust on [Bevy](https://bevyengine.org/).

The engine plays whatever game folder it's given: the same server and clients run any game, without a
rebuild. It's released as binaries, and a game is made, played, tested and hosted with those binaries
alone.

> An example game is available at http://k573.dev

## What's here

| Folder | What it is |
| --- | --- |
| [`engine/`](engine) | The engine: the simulation the server and client share, the game server, and the game client. |
| [`editor/`](editor) | The editor: opens a game folder, edits it through the engine's own loader and writer, and play-tests it on the engine's binaries. New games start from its templates. |
| [`cli/`](cli) | `rift`, the command line tool: checks, renders, benches, load tests and profiles a game. |
| [`web-server/`](web-server) | A game's website: its pages, sign-in, and the game client's downloads. |
| [`oidc/`](oidc), [`service/`](service) | Sign-in through any OpenID Connect provider, and what the HTTP services share: metrics and profiles. |
| [`ui/`](ui) | The interface the editor and the game share: colors, shapes, text roles, fonts and every general-purpose egui component, so both look and behave alike. |
| [`bevy/`](bevy) | Bevy plugins that know nothing of rift: entity replication and a server-authoritative terminal. |
| [`smoke/`](smoke) | The website, the game server and the game client, smoke-tested together: a player of a stand-in identity provider starts the client from the `rift.env` the site hands out, and its window must come to look like `smoke/snapshot.png`. |
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
| `game-client` | Linux, Windows, macOS | The game, for players. |
| `editor` | Linux, Windows, macOS | Makes and changes games. |
| `rift` | Linux, Windows, macOS | The command line tool. |
| `web-server` | Linux, Windows, macOS | A game's website. |
| `game-server-profiling` | Linux | `game-server` with the symbols and frame pointers a sampling profiler reads. |

Archives extracted into one folder make a **binaries folder**: `editor` and `rift` find `game-server`,
`game-client` and `game-server-profiling` beside themselves. `just dist` builds this machine's
binaries into such a folder, `target/dist/`, so an unreleased build runs exactly as a release would.

## The rift tool

`rift` is the command line tool a game is checked, measured and inspected with. It works on a game
folder, and starts the other binaries from its own binaries folder when a command needs them.

| Command | What it does |
| --- | --- |
| [`rift check <game>`](#checking-a-game) | Checks that the game loads whole and reads as the editor writes it. |
| [`rift format <game>`](#checking-a-game) | Rewrites the files that don't. |
| [`rift fit <game> <prop>…`](#fitting-footprints) | Shapes props' footprints to their models. |
| [`rift ask <game>`](#asking-about-a-game) | Answers the probe's questions about the game, without a client. |
| [`rift render area <game> <shot> <out>`](#rendering-an-area) | Draws a picture of an area off screen. |
| [`rift bench [game]`](#benching) | Measures the server's capacity, and on a game also the client's and the editor's frame rates and memory. |
| [`rift loadtest --http <url> [count]`](#load-testing-a-server) | Connects simulated players to a running game server. |
| [`rift profile --out <dir> [game]`](#profiling-the-server) | Samples the game server under load. |
| [`rift schema <protocol>`](#schemas) | Prints the JSON Schema of a protocol the binaries speak. |

`rift help <command>` lists every flag of a command.

### Checking a game

```sh
rift check <game>
rift format <game>
```

`rift check` fails unless the game loads whole and every file reads exactly as the editor writes it,
so that saving the game in the editor changes only what was edited. `rift format` rewrites the files
that read otherwise: what each holds stays, only its layout changes.

### Fitting footprints

```sh
rift fit <game> [--tall <meters>] <prop>…
```

Shapes the footprint of each prop named, by its path in the game folder, to the ground its model
takes from whoever walks up to it, as the editor's fit does one prop at a time. The footprint goes
round all of the model a walker would meet, not only what touches the ground: a boulder that bulges
over a narrow foot blocks as far as it bulges, and a crown overhead is walked under. A walker is as
tall as the game's player unless `--tall` says otherwise; a low height fits a tree to its trunk alone.

### Asking about a game

```sh
rift ask <game>
```

Reads the [probe](#protocols)'s questions from stdin, one JSON line each, and answers each on stdout as
a game client playing the game would. It answers the questions about the game itself, such as
where its things are and how its ground walks; a snapshot of what a client shows needs a client.

### Rendering an area

```sh
rift render area <game> <shot> <out>
```

Draws the picture `<out>` of an area through the world's own camera and lights, to hold how the game
looks against a reference picture. The shot is a JSON file that names the area, the lens, and the
bodies standing in it. The same shot draws the same picture every time, and `--check-determinism`
proves it: it draws the shot twice, each in a process of its own, and fails when the two pictures
differ by more than the eye can tell.

### Benching

```sh
rift bench                  # the server's capacity, on a large game generated in code
rift bench <game>           # the whole bench on a game
```

Without a game, `rift bench` measures only the server. On a game it runs four parts in order, or only
those `--only` names:

| Part | What it measures | Needs |
| --- | --- | --- |
| `server` | How many copies of the game's spawn area, each full of players, the server ticks within its budget. | |
| `client` | The game client's frame rate through a catalogue of scenarios. | Linux |
| `editor` | The editor's frame rate through scenarios of its own. | Linux |
| `memory` | Whether the server and the client hold their memory level as a player plays. | Linux |

The server part runs inside `rift` itself. The other three start `game-server`, `game-client` and
`editor` from the binaries folder, and play them on a desktop of the bench's own: a headless `weston`
screen drawn on through `Xwayland`, the window seated there by `xdotool`, and a silent sink on a
PulseAudio-compatible sound server (`pactl`). The frame rate is then the program's alone, and
nothing reaches the machine's display or speakers.

#### Scenarios

A scenario is a few seconds of play: the server stages the scene, then the bench plays it as a person
would, through the program's own bindings, confirming each input through the probe before the next.

| Scenario | What plays |
| --- | --- |
| `desktop` | The emptiest view of the engine's own new game, at the lowest settings. It checks the bench's desktop rather than the game: see [below](#what-a-scenario-must-meet). |
| `open` | The client's opening, from the player's entry into the world until the scenery stands, on a profile no client has run the game on. |
| `still` | A still view of the game's home place. |
| `orbit` | The camera turned a full circle there. |
| `horizon` | The camera tilted there to look out over the land, as far as it lies. |
| `walk` | A walk from home across open ground, and back. |
| `crossings` | A warp out of the home area crossed, and back. |
| `combat` | Fights staged at the game's crowd place. |
| `ui-storm` | Every window opened, swept, dragged, scrolled and typed into. |
| `players` | Fifty other players in view at the game's spawn. |
| `editor-open` | The editor's opening, until the area the game starts in stands in view. |
| `editor-camera` | The stage of that area turned, panned, and flicked in and out. |
| `editor-brush` | Its terrain sculpted. |
| `editor-overview` | The stage of the home area turned and panned, drawn back as far as it goes. |

A scenario the game has nothing to play with is left out: `walk` without open ground to walk to,
`crossings` without a warp out of the home area, `combat` without a crowd place.

These flags choose what runs:

| Flag | What it does |
| --- | --- |
| `--only <part>` | Runs only the parts named: `server`, `client`, `editor`, `memory`. Repeat it to name several. |
| `--scenario <pattern>` | Runs only the scenarios whose names match, `*` standing for anything: `--scenario 'editor-*'`. |
| `--video <preset>` | The video presets the scenarios play at: `highest` (the default), `lowest` or `both`. |
| `--repeat <n>` | Runs each scenario `n` times. The report keeps the median run, and how far the others strayed from it. |
| `--spread` | Spreads the server part's players over the area, rather than crowding them on its spawn. |
| `--adapter <name>` | Refuses to run unless the client draws on this graphics adapter. |
| `--smoke` | Plays one short scenario per program, held to no criteria, to prove the harness alone. |

#### The scenarios file

What the scenarios play with comes from the game. Its scenarios file, `bench/scenarios.json` in the
game folder unless `--scenarios` names another, may name any of these, and the file itself is
optional:

| Field | What it names | When left out |
| --- | --- | --- |
| `home` | The place the scenarios play at, and the area `editor-overview` opens: an `area`, `x` and `y`. | Where the game's characters start. |
| `crowd_place` | A place where foes crowd, for `combat`. | `combat` is left out. |
| `walk_to` | The ground the `walk` ends at. | Open ground some way from home, found in the content. |
| `windows` | The input actions that open the windows `ui-storm` opens. | Every window of the game's interface. |
| `character` | The test character the scenarios play. | The game's first, or a fresh one where it has none. |

#### What a scenario must meet

Every scenario's frames are held to the same criteria:

- a mean frame rate at or over the floor of its video preset: 60 fps at the highest, 90 fps at the
  lowest;
- at most 1% of frames slower than the floor's period;
- no frame longer than 48 ms;
- nothing the view takes in missing. An opening is spared this one, as its view is still arriving.

The `desktop` scenario guards the measurement itself. The engine's emptiest view must draw at over
300 fps whatever the benched game costs to draw; when it doesn't, the desktop is holding the frames
back, every scenario would measure the desktop rather than the engine, and the harness fails. A
machine whose load average is over half its cores is warned about, as its frames measure that load
too.

The memory part has a rule of its own. It crosses a player back and forth between the spawn area and
the area its first warp leads to, then stands them still: once the first crossings have filled the
caches, the server and the client must hold the memory level those reached. A game with no warp out
of its spawn area is not measured.

#### The report

A run writes its report to `bench-report.json`, or wherever `--report` says, in the shape
`rift schema bench` describes. A log of every step the programs were put through is written beside
it.

`--compare <report>` holds the run against an earlier run's report, and fails when a scenario that
passed now fails, a mean frame rate fell by more than a tenth, or a p99 frame time rose by more than
a fifth:

```sh
rift bench <game> --report before.json    # before a change
rift bench <game> --compare before.json   # after it
```

The exit code says how the run went:

| Code | Meaning |
| --- | --- |
| 0 | Everything passed. |
| 1 | A scenario fell short of its criteria, or the comparison got worse. |
| 2 | A scenario could not be played through. |
| 3 | A process died. |

### Load testing a server

```sh
rift loadtest --http <url> [count]
```

Connects simulated players to a game server that is already running, at its HTTP address, over the
client's own transport. The server must take any account's sign-in (`AUTH` set to `fake`). With a
count, that many players connect and stay until `rift` is stopped. Without one, `rift` searches for
the highest player count the server holds within its tick budget.

### Profiling the server

```sh
rift profile --out <dir> [game]
```

Starts `game-server-profiling` on the game, or on a large game generated in code, loads it with as
many simulated players as it holds within its tick budget, and samples it with `perf` for twenty
seconds. Linux only. It writes into `<dir>`:

| File | What it holds |
| --- | --- |
| `perf.data` | The samples, for `perf` to read. |
| `cpu-by-function.txt` | The CPU time by function. |
| `cpu-by-system.txt` | The CPU time by engine system. |
| `allocations.pb.gz` | What the server's heap holds, in pprof's format. |

### Schemas

```sh
rift schema probe    # what a game client started with a probe address answers on it
rift schema server   # what a game server prints once ready, and what its test routes take
rift schema bench    # the report `rift bench` writes
```

Each prints the JSON Schema of one of the [protocols](#protocols) the binaries speak, for whatever
drives them to read.

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
- **`game-client`** reads `RIFT_CLIENT_*`: the game server (`GAME_SERVER_URL`),
  `AUTH`, `PROFILE_DIR`, `EXIT_ON_STDIN_EOF` and, optionally, `ACCESS_TOKEN` and `PROBE`.
- **`web-server`** reads `RIFT_WEB_SERVER_*`: `BIND`, `PORT`, `REDIRECT_URI`, the game server as
  game clients reach it (`GAME_SERVER_URL`) and as it reaches it itself
  (`GAME_SERVER_INTERNAL_URL`), and the release whose game client its downloads offer
  (`DOWNLOADS_URL`, `RELEASE`). The `rift.env` it serves to game clients points them at its game
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
- **The probe.** A game client started with `RIFT_CLIENT_PROBE=<host:port>` connects there and
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
just editor           # build editor, game-server and game-client, and open the editor
just editor ~/my-game # …on a game folder
just dist             # this machine's binaries, as a release lays them out, in target/dist
just dist target/dist profiling # …and game-server-profiling, on Linux
just bench            # the server bench on the generated game
just lint             # rustfmt, clippy, the repo's own lints, and `rift check` on the editor's templates
just test             # every crate's tests
just smoke            # start the game client from the website's rift.env and see it draw the game
```

## Tests

`just test` runs every crate's tests, in seconds once built: unit and integration tests of the
engine's mechanics, the editor and the sign-in, none of which starts a server, a client or a window. A
test declares the small game it needs in code, with [`testkit`](testkit)'s `Game` (only the files it
reads, loaded through the engine's own loader), and plays it out in `testkit`'s `Sim`: a bare world
holding that game, stepped tick by tick through only the engine systems the test adds, in the
server's order. The sign-in is tested against a fake OpenID Connect provider run in the test itself.
