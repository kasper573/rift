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
| [`smoke/`](smoke) | The website, the game server and the game client, smoke-tested together: a player of a stand-in identity provider downloads the client from the site, starts it as extracted, and its window must come to look like `smoke/snapshot.png`. |
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

Every archive comes with its minisign signature beside it, `<archive>.minisig`, made with the release
key whose public half is [`release.pub`](release.pub). A build reports its release's tag as its build
(`--build-info`), and a release build has the key built in: a game client whose server runs another
build downloads that build's `game-client` archive, installs it only when the key signed it, and starts
again on it. A build made without the key (`RIFT_RELEASE_KEY` unset when it was built), or a client
started without `RIFT_CLIENT_RELEASES_URL`, tells the player to start a client of the server's build
instead.

Archives extracted into one folder make a **binaries folder**: `editor` and `rift` find `game-server`,
`game-client` and `game-server-profiling` beside themselves. `just dist` builds this machine's
binaries into such a folder, `target/dist/`, so an unreleased build runs as a release would.

## The rift tool

`rift` is the command line tool a game is checked, measured and inspected with. It works on a game
folder, and starts the other binaries from its own binaries folder when a command needs them.

| Command | What it does |
| --- | --- |
| [`rift check <game>`](#checking-a-game) | Checks that the game loads whole, reads as the editor writes it, and has its maps baked. |
| [`rift format <game>`](#checking-a-game) | Rewrites the files that don't. |
| [`rift fit <game> <prop>…`](#fitting-footprints) | Shapes props' footprints to their models. |
| [`rift bake maps <game>`](#baking-the-maps) | Bakes the maps of every area into its bundle. |
| [`rift ask <game>`](#asking-about-a-game) | Answers the probe's questions about the game, without a client. |
| [`rift render area <game> <shot> <out>`](#rendering-an-area) | Draws a picture of an area off screen. |
| [`rift render dialogue <game> <out>`](#rendering-the-dialogue) | Draws the dialogue in every style it sets, over many grounds. |
| [`rift bench [game]`](#benching) | Measures the server's ticks under a load of players playing, and on a game also the client's and the editor's frame rates and memory. |
| [`rift loadtest --http <url> [count]`](#load-testing-a-server) | Connects simulated players to a running game server. |
| [`rift play <game> --http <url>`](#playing-without-a-window) | Plays characters on a running game server with no window, doing what it is told on stdin. |
| [`rift profile --out <dir> [game]`](#profiling-the-server) | Samples the game server under load. |
| [`rift records push <game> --store <address>`](#pushing-the-records) | Brings what a game server keeps to the shapes this build reads, dropping what no longer fits. |
| [`rift schema <protocol>`](#schemas) | Prints the JSON Schema of a protocol the binaries speak. |

`rift help <command>` lists every flag of a command.

### Checking a game

```sh
rift check <game>
rift format <game>
```

`rift check` fails unless the game loads whole, every file reads exactly as the editor writes it, so
that saving the game in the editor changes only what was edited, and the map of every area is baked
from the area as it is (see [baking the maps](#baking-the-maps)). It bakes nothing itself, and takes
no GPU: a map's keys are worked out from the area's files alone. `rift format` rewrites the files that
read otherwise: what each holds stays, only its layout changes.

### Fitting footprints

```sh
rift fit <game> [--tall <meters>] <prop>…
```

Shapes the footprint of each prop named, by its path in the game folder, to the ground its model
takes from whoever walks up to it, as the editor's fit does one prop at a time. The footprint goes
round all of the model a walker would meet, not only what touches the ground: a boulder that bulges
over a narrow foot blocks as far as it bulges, and a crown overhead is walked under. A walker is as
tall as the game's player unless `--tall` says otherwise; a low height fits a tree to its trunk alone.

### Baking the maps

```sh
rift bake maps <game> [--profile <dir>]
```

The maps a player opens show each area as a survey map draws it, from pictures the designer bakes
into the area's bundle: `map/<level>_<column>_<row>.webp`, a picture of each square of every level,
and `map.json`, the key each was drawn from. The game only ever reads them, so they cost a player's
machine nothing to draw. The editor bakes them from its File menu, and bakes an area that has no map
at all on its own (a new area, or a game never baked) as soon as it holds one; `rift bake maps`
bakes them off screen the same way. Both need a GPU, as each prop is drawn from its impostor.

A bake draws again only the squares whose key changed: a square's key is made of the ground, the
props and the style it shows, so an edit puts only the squares it reaches out of date. Until the maps
are baked again, the game shows them as they were baked, and `rift check` names every area whose map
is out of date. `--profile` keeps the models' impostors between bakes, as a player's profile keeps
them.

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

### Rendering the dialogue

```sh
rift render dialogue <game> <out> [--over <picture>…]
```

Draws the game's dialogue off screen into the folder `<out>`, once for every style a line is set in
(each ink, voice and motion, alone and together, keys, a narrator, a line part typed) and every state
a choice and its tags show, over the lightest, darkest and most saturated grounds and bands of all of
them side by side, and over each picture `--over` names, such as frames of the game. Each picture is
named `<ground>-<specimen>.png`, so how the dialogue reads over whatever a game puts behind it is seen
in one go.

### Benching

```sh
rift bench                  # the server's ticks, on a large game generated in code
rift bench <game>           # the whole bench on a game
```

Without a game, `rift bench` measures only the server. On a game it runs four parts in order, or only
those `--only` names:

| Part | What it measures | Needs |
| --- | --- | --- |
| `server` | How long the server takes to tick copies of the game's spawn area, each full of players playing. | |
| `client` | The game client's frame rate through a catalogue of scenarios. | Linux |
| `editor` | The editor's frame rate through scenarios of its own. | Linux |
| `memory` | Whether the server and the client hold their memory level as a player plays. | Linux |

The server part runs inside `rift` itself. The other three start `game-server`, `game-client` and
`editor` from the binaries folder, and play them on a desktop of the bench's own: a headless `weston`
screen drawn on through `Xwayland`, the window seated there by `xdotool`, and a silent sink on a
PulseAudio-compatible sound server (`pactl`). The frame rate is then the program's alone, and
nothing reaches the machine's display or speakers.

#### The server part

The server part ticks 100 copies of the game's spawn area, each a world of its own as the
server keeps one, with 25 players in each and the game's own NPCs about them, the clients
connected and sent everything that changes. The players join as the game's test characters
and play through the game's own requests, each in a role:

| Role | Share | What it does |
| --- | --- | --- |
| fighters | two in five | Cast the skills on their bars at the nearest foe, and use what their bags hold to be used. |
| walkers | two in five | Click a stride ahead across the area every second and a half, never standing long. |
| talkers | one in five | Walk up to the nearest one who talks, and pick their way through the conversation. |

One player in twenty also stands at the market, searching it, listing goods and buying, and the
market starts holding ten listings for every player, as a server's does once its players have
traded a while. Without a
game the bench plays the crowd, a game generated in code to load a server: a wide area of raiders
that hunt whoever comes near and townsfolk who roam and stop to talk, and test characters who
carry swords, potions, gold and goods.

It measures the simulation alone first, then with the clients connected, each over 200 ticks
once the players' ground is held and the warmup is past, and reports the mean, p50, p99 and
worst tick, what replication adds, the market's traffic, and how many players were walking and
talking as the run ended. `--areas` measures another load, and `--capacity` searches for the
most areas the server ticks within its budget instead, which takes minutes and reports nothing
to compare by.

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
| `ui-storm` | Every tab of the menu opened and swept, then the interface dragged, scrolled and typed into. |
| `players` | Fifty other players walking about the game's spawn, in view. |
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
| `--repeat <n>` | Runs each scenario, and the server part, `n` times. The report keeps the median run, and how far the others strayed from it. |
| `--areas <n>` | Measures the server part at `n` areas rather than 100. |
| `--capacity` | Finds the most areas the server ticks within its budget, rather than measuring one load. |
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
| `tabs` | The input actions that open the tabs of the menu `ui-storm` opens, in turn. | Every tab of the menu, the Character tab last. |
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
passed now fails, a mean frame rate fell by more than a tenth, a p99 frame time rose by more than
a fifth, or, measured at the same load, the server's mean tick rose by more than a tenth or its p99
by more than a fifth:

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

### Playing without a window

```sh
rift play <game> --http <url>
```

Plays any number of characters on a game server that is already running, each through a headless
client: a client that signs in, joins and holds its session like a player's, so its character is in
the world as any other is, but that opens no window and draws nothing. What the characters do is up
to whoever runs it: a bot, a load of its own, a film's supporting cast.

`<game>` is the game folder the server runs, which `rift play` names content by; it refuses a server
that runs another game. Every line of stdin is one JSON object telling a client, by a name of your
choosing, what to do:

```json
{"join": {"player": "ada", "token": "<access token>"}}
{"request": {"player": "ada", "request": {"MoveRequest": {"pos": [12.5, 40.0], "seq": 1}}}}
{"look": {"player": "ada"}}
{"leave": {"player": "ada"}}
```

- `join` signs a client in with an access token and joins the game as its account's character.
- `request` sends the server a request, exactly as a game client sends it: any of the requests a
  game client can make, under its name. The server answers it as it answers a player's.
- `look` asks what the client sees around its character: the part of the probe's snapshot that
  takes no screen to tell (the character, who and what stands near it, what lies on the ground,
  its bag).
- `leave` closes the client, as a player leaving the game does.

Whatever stands in the world is named by its id, the same in every client's eyes: the ids a `look`
and a game client's probe report are the ones a request takes. Content is named by its path in the
game folder. Stdout says how it goes, one JSON line each: `in_world` once a character stands in the
world and `out_of_world` when it leaves it (as it does between areas), `sight` in answer to a `look`,
`dropped` when the server closes a connection, and `refused` for a line that could not be done.
`rift play` runs until stdin closes. `rift schema play` prints the schema of both sides.

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

### Pushing the records

```sh
rift records push <game> --store <address> [--over <user@host>] [--write]
```

Saved data carries no compatibility promise: a build may change the shape of what a game server
keeps, and a record an earlier build wrote may then not read. A saved character that doesn't read
refuses its player's sign-in, and any other record that doesn't read keeps the server from
starting. `rift records push` brings the store to the shapes this `rift`'s build reads, without
migrating it, as a database tool's push does: it reads every record as the game server reads it, for
`<game>`, and drops whatever of it doesn't read, the least it can: a field the build no longer has, a
list entry without what the build needs of it, and the whole record only when it is without what it
can't be without. Everything that still reads is kept as it is.

It only tells what it would drop, record by record, until it is given `--write`, which writes all of
it in one transaction. Run it with the `rift` of the release the game server runs, while no game
server uses the store: a running server holds characters in memory and writes them back.

`<address>` is the store's address as the game server's `STORE` names it (`postgres://…` or
`sqlite://…`). It runs as well on a store on your own machine, during development, as on one on a
server.

Tip: a database that only its server's machine reaches needs no port opened to it. With
`--over <user@host>`, the push reaches the store through a tunnel over `ssh` to that machine, with
your own keys and `ssh` settings, and `<address>` names the store as that machine reaches it, port
included:

```sh
rift records push <game> --store postgres://<user>:<password>@127.0.0.1:5432/<database> --over <user@host>
rift records push <game> --store postgres://<user>:<password>@127.0.0.1:5432/<database> --over <user@host> --write
```

### Schemas

```sh
rift schema probe    # what a game client started with a probe address answers on it
rift schema server   # what a game server prints once ready, and what its test routes take
rift schema bench    # the report `rift bench` writes
rift schema play     # what `rift play` takes on stdin, every request of a game client among it, and what it prints
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
  folder (`CONTENT_DIR`), `MAX_CLIENTS`, how many instances of the game's instanced areas it runs
  at once (`MAX_INSTANCES`), `CHARACTER_SPAWN_POLICY` (`map` or `dist`), `CHARACTERS_IMMORTAL`,
  `DAYLIGHT` (`turning`; `still`, which holds the game's day at its start hour for tests that
  compare how the world looks; or `fixed`, which holds it at `HOUR`, 0 to 24), `WEATHER` (`turning`,
  or `still`, which holds every area's weather at its first spell), `DAILY_RESET_UTC_HOUR`, `AUTH` (`oidc`, or `fake`, which takes a token's claims without checking
  its signature, so an unsigned JWT signs in any account), `TEST_CHARACTERS`, `EXIT_ON_STDIN_EOF`,
  where the game is saved (`STORE`: `postgres://…` for a database server, or `sqlite://…` for a
  file, in which it makes its one table when there is none), how many seconds it plays on between
  two saves (`CHECKPOINT_EVERY`) and, optionally, `RNG_SEED`. Started on port 0 it picks free
  ports, and prints them on stdout once players can connect. It does not start when it cannot
  reach its store, and refuses a sign-in whose saved character it cannot read (see
  [pushing the records](#pushing-the-records)). It saves every account and character as they stand at each save, and once more
  when it is told to stop, trying again for as long as its store refuses; told to stop a second
  time, it stops with what was not written named in its log.
- **`game-client`** reads `RIFT_CLIENT_*`: the game server (`GAME_SERVER_URL`),
  `AUTH`, `PROFILE_DIR`, `EXIT_ON_STDIN_EOF` and, optionally, `ACCESS_TOKEN`, `PROBE` and where
  releases are published (`RELEASES_URL`, as the web-server's `DOWNLOADS_URL`), which it updates
  itself from. It sets `UPDATED_FROM` itself, on the start after an update.
- **`web-server`** reads `RIFT_WEB_SERVER_*`: `BIND`, `PORT`, `REDIRECT_URI`, the game server as
  game clients reach it (`GAME_SERVER_URL`) and as it reaches it itself
  (`GAME_SERVER_INTERNAL_URL`), the release whose game client its downloads offer
  (`DOWNLOADS_URL`, `RELEASE`), and the video file its landing page plays (`PREVIEW_VIDEO`). Its
  downloads are each platform's `game-client` archive of that release, remade with a `rift.env`
  beside the client, both in a folder named after the game: extracted, it plays. That `rift.env`
  points the client at its game server, at its releases, and at the sign-in pages in
  `RIFT_WEB_SERVER_DESKTOP_AUTH_*` (`CLIENT_ID`, `AUTHORIZATION_URI`, `SIGN_UP_URI`, `TOKEN_URI`).
  The site is named after the game its game server serves.

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
  between players and the server must pass those UDP ports through as they are. A sign-in whose
  saved character the server cannot read is answered 503, and gets no character in its place.
- **Metrics** are served in the Prometheus exposition format at `/metrics`, and **profiles** in pprof's
  format at `/debug/pprof/profile?seconds=N` (CPU) and `/debug/pprof/heap`, on Linux and macOS. Both
  servers serve them beside their own routes. The game server's tell how saving goes:
  `rift_last_checkpoint_timestamp_seconds`, the last save that worked and the one to alert on,
  `rift_records_unwritten`, what stopping right now would lose, `rift_checkpoints_total` by
  `outcome`, `rift_checkpoint_duration_seconds`, `rift_checkpoint_records_written_total`, and
  `rift_sign_ins_total` by where each `character` was found (`in_play`, `memory`, `store`,
  `refused`).
- **Logs** go to stdout.
- **The probe.** A game client started with `RIFT_CLIENT_PROBE=<host:port>` connects there and
  answers questions about what it shows, one JSON line each: a frame's snapshot, a picture of its
  window, or where the game's things are and how its ground walks. `rift ask <game>` answers the questions about the game
  itself the same way, without a client. The editor started with `--probe <host:port>` answers its
  own: a frame's snapshot (its input, interface, panels, stage and selection), the rows its trees
  drew, and its frames. `rift schema probe` prints the protocol's schema. An id a snapshot gives
  someone or something in the world is the server's, so it means the same to every client.
- **Test routes.** A game server with `TEST_CHARACTERS` set hands the game's test characters out
  over HTTP, to whoever drives it: `/test/character` makes the asking account's next sign-in play
  one, and `/test/stage` has the character that account plays walk, fight or move, and sets the day
  and the weather.
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
just bench            # the server bench on the generated game, in a few seconds
just lint             # rustfmt, clippy, the repo's own lints, and `rift check` on the editor's templates
just test             # every crate's tests
just smoke            # download the game client from the website, start it, and see it draw the game
just build-times      # how long each of these builds takes, as the build cache stands
```

Every recipe builds on the `dev` profile, made for iterating, and every recipe shares one build, so
switching between them rebuilds nothing. The binaries that ship are built on `release`: CI builds
them with `just archives`, the one way to a release asset. `just profile=release <recipe>` runs any
recipe on release, for an issue only the shipped build shows.

## Tests

`just test` runs every crate's tests, in seconds once built: unit and integration tests of the
engine's mechanics, the editor and the sign-in, none of which starts a server, a client or a window. A
test declares the small game it needs in code, with [`testkit`](testkit)'s `Game` (only the files it
reads, loaded through the engine's own loader), and plays it out in `testkit`'s `Sim`: a bare world
holding that game, stepped tick by tick through only the engine systems the test adds, in the
server's order. The sign-in is tested against a fake OpenID Connect provider run in the test itself.
