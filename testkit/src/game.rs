use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use engine::core::audio::playback::SFX_CLIP;
use engine::core::babble::BABBLE_LETTERS;
use engine::core::content::{
    CONFIG_FILE, Content, ContentFile, ContentId, ContentRegistry, LoadReport, MemoryFolder,
    SETTINGS_FILE, Shape, content_path, name_of, table_of, version,
};
use engine::core::ground::{GroundSize, MIN_SIDE, Paint, SAMPLES_PER_METER};
use engine::core::time::{Millis, Seconds};
use engine::core::units::{Meters, MetersPerSec};
use engine::core::vfx::shader::{SHADER_SOURCE, Stage};
use engine::systems::actor::Action;
use engine::systems::actor::model::{MODEL_BUSTS, MODEL_GLB};
use engine::systems::actor::skeleton::{CLIP_LIBRARIES, RIG_GLB};
use engine::systems::area;
use engine::systems::area::prop::PROP_GLB;
use engine::systems::area::scene::SCENE;
use engine::systems::item;
use serde_json::{Value, json};
use strum::VariantArray;

use crate::art::{glb, library_glb, png, rig, rig_glb, skin_glb, wav, wav_of};

/// A game folder held in memory, of only the files a test declares, loaded by the engine's own
/// loader against every type a game is made of. A file the test leaves out is simply absent, so a
/// test that leans on more than it declares fails loudly at load or where the engine reads it.
pub struct Game {
    files: BTreeMap<String, Arc<[u8]>>,
    settings: Option<Value>,
    whole: bool,
}

// Built once a process: hashing every shape for the content version takes a while.
static TABLES: LazyLock<ContentRegistry> = LazyLock::new(engine::systems::tables);
static SETTLED_TABLES: LazyLock<ContentRegistry> = LazyLock::new(engine::systems::settled_tables);

/// Looks up the files a test declared by the names it declared them under.
pub trait Declared {
    /// The file of `Row`'s type declared as `name`.
    fn declared<Row: ContentFile>(&self, name: &str) -> Option<ContentId<Row>>;
}

impl Declared for Content {
    fn declared<Row: ContentFile>(&self, name: &str) -> Option<ContentId<Row>> {
        self.by_path::<Row>(&path(Row::TABLE, name))
    }
}

/// How long every clip of a declared model plays.
pub const CLIP: Millis = Millis(300.0);

/// When a declared model's attack lands.
pub const APEX: Millis = Millis(100.0);

/// The ground a declared skeleton's walk, run and sprint cover, at scale 1.
pub const WALK_PACE: MetersPerSec = MetersPerSec(Meters(1.0));
pub const RUN_PACE: MetersPerSec = MetersPerSec(Meters(3.0));
pub const SPRINT_PACE: MetersPerSec = MetersPerSec(Meters(6.0));

/// The skeleton every declared model wears unless the test names another.
pub const BODY: &str = "body";

/// The library every declared skeleton keeps its clips in.
pub const LIBRARY: &str = "moves";

/// The clips a declared skeleton's library carries: one per action, each `CLIP` long.
pub fn rig_clips() -> Vec<(&'static str, Seconds)> {
    Action::VARIANTS
        .iter()
        .map(|action| (action.as_ref(), CLIP.seconds()))
        .collect()
}

/// An area's ground as text, one string per row: `.` walkable, `#` blocked.
#[derive(Clone)]
pub struct Map(pub Vec<String>);

/// The prop every item a test declares is lying on the ground as.
const DROP: &str = "drop";

/// The terrains and the footstep of every area a test declares.
pub const TURF: &str = "turf";
pub const ROCK: &str = "rock";
const STEP: &str = "step";

/// The path of the file of type `table` a test declares as `name`: the fixture of that name where
/// the engine has one, and otherwise `<name>.<table>.json`, or the bundle `<name>.<table>`, at the
/// top of the game folder.
pub fn path(table: &str, name: &str) -> String {
    let registry = engine::systems::registry();
    if let Some(fixture) = registry
        .fixture_paths()
        .find(|path| table_of(path) == Some(table) && name_of(path) == name)
    {
        return fixture.to_owned();
    }
    let shape = registry
        .shape_of(table)
        .unwrap_or_else(|| panic!("no content type `{table}`"));
    content_path("", name, table, shape)
}

impl Game {
    pub fn new() -> Game {
        let mut game = Game {
            files: BTreeMap::new(),
            settings: None,
            whole: false,
        };
        game.put(SETTINGS_FILE, b"{}".to_vec());
        game
    }

    /// The file of type `table` named `name`, of `fields`: the file itself, or a bundle's config.
    pub fn row(mut self, table: &str, name: &str, fields: Value) -> Game {
        let file = fields_path(table, name);
        self.put(&file, json_bytes(&fields));
        self
    }

    /// A part of the bundle of type `table` named `name`, at `part` inside it.
    pub fn part(mut self, table: &str, name: &str, part: &str, bytes: impl Into<Vec<u8>>) -> Game {
        self.put(&format!("{}/{part}", path(table, name)), bytes.into());
        self
    }

    /// A script file of `nodes`, run by a `runs_on`, that a script module names by its path.
    pub fn script(self, name: &str, runs_on: &str, nodes: Value) -> Game {
        self.row(
            "script",
            name,
            json!({ "runs_on": runs_on, "nodes": nodes }),
        )
    }

    /// A media file at `path` in the folder, which content names by that path.
    pub fn asset(mut self, path: &str, bytes: Vec<u8>) -> Game {
        self.put(path, bytes);
        self
    }

    /// A file at `path` in the folder, as is.
    pub fn file(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Game {
        self.put(path, bytes.into());
        self
    }

    pub fn remove(mut self, path: &str) -> Game {
        self.files
            .remove(path)
            .unwrap_or_else(|| panic!("no file `{path}` declared"));
        self
    }

    /// The game's own settings, read from `fixtures/game.config.json`, at the content version this
    /// build reads unless they name one.
    pub fn settings(mut self, settings: Value) -> Game {
        self.settings = Some(settings);
        self
    }

    /// Changes the settings declared before.
    pub fn edit_settings(mut self, change: impl FnOnce(&mut Value)) -> Game {
        change(self.settings.as_mut().expect("settings declared"));
        self
    }

    /// A playable character (its model, class and player file) and settings that start every new
    /// character at `x`, `y` in `area`.
    pub fn hero(self, area: &str, x: f32, y: f32) -> Game {
        self.model("hero", 2.0, json!([]))
            .row(
                "class",
                "hero",
                json!({
                    "name": "Hero",
                    "icon": "icons/hero.png",
                    "levels": [{ "xp": 0, "skill_points": 1 }],
                }),
            )
            .asset("icons/hero.png", png(1, 1))
            .row(
                "player",
                "hero",
                json!({
                    "model": path("model", "hero"),
                    "class": path("class", "hero"),
                    "stats": stats(30.0),
                }),
            )
            .settings(json!({
                "name": "Test",
                "initial_spawn": { "area": path("area", area), "x": x, "y": y },
                "player": path("player", "hero"),
            }))
    }

    /// A sword that, held, lets its wearer strike: a basic attack on one enemy, landing the wearer's
    /// damage at the height of its swing; it puts its wearer in the sword stance and takes the main
    /// hand.
    pub fn sword(self) -> Game {
        self.row(
            "skill",
            "strike",
            json!({
                "name": "Strike",
                "icon": "icons/strike.png",
                "target": "single_enemy",
                "basic_attack": true,
                "levels": [{ "cost": 1, "stats": { "power": 1.0 } }],
                "modules": [{ "type": "script", "script": path("script", "strike") }],
            }),
        )
        .script(
            "strike",
            "skill",
            json!({
                "used": { "type": "on", "event": "used", "next": "swing" },
                "swing": { "type": "animate", "action": "attack", "wait": "apex", "next": "hit" },
                "hit": {
                    "type": "damage",
                    "who": { "type": "skill_targets" },
                    "by": { "type": "stat", "stat": "damage" },
                },
            }),
        )
        .asset("icons/strike.png", png(1, 1))
        .stance("sword", 1, "main")
        .item(
            "sword",
            json!({ "kind": {
                "type": "equipment",
                "slot": "hand",
                "stance": path("stance", "sword"),
                "grants": [path("skill", "strike")],
            } }),
        )
    }

    /// A stance of `hands` hands whose clips were made for `hand` (`main`, `off` or `either`),
    /// pairing with nothing.
    pub fn stance(self, name: &str, hands: u8, hand: &str) -> Game {
        self.row(
            "stance",
            name,
            json!({ "name": name, "hands": hands, "hand": hand }),
        )
    }

    /// A skeleton of the test rig, with a right and a left hand, its library carrying a clip named
    /// after every action, each [`CLIP`] long, which its base set plays; its attack lands at
    /// [`APEX`].
    pub fn skeleton(self, name: &str) -> Game {
        self.skeleton_of(name, &rig_clips(), json!({}))
    }

    /// A skeleton of the test rig, its library carrying `clips`, of `fields` over the plain
    /// skeleton's: a right and a left hand, a base set playing each action's clip of its name, the
    /// attack landing at [`APEX`], its walk, run and sprint covering [`WALK_PACE`], [`RUN_PACE`] and
    /// [`SPRINT_PACE`].
    pub fn skeleton_of(self, name: &str, clips: &[(&str, Seconds)], fields: Value) -> Game {
        let base: serde_json::Map<String, Value> = Action::VARIANTS
            .iter()
            .map(|action| (action.as_ref().to_owned(), json!(action.as_ref())))
            .collect();
        let mut row = json!({
            "name": name,
            "hands": [
                { "name": "right", "joint": "hand.r" },
                { "name": "left", "joint": "hand.l" },
            ],
            "base": { "clips": base },
            "cues": { "attack": [{ "at": APEX.0, "apex": true }] },
            "paces": { "walk": WALK_PACE.0.0, "run": RUN_PACE.0.0, "sprint": SPRINT_PACE.0.0 },
        });
        for (field, value) in fields.as_object().expect("a skeleton's fields") {
            row[field] = value.clone();
        }
        let tree = rig();
        self.row("skeleton", name, row)
            .part("skeleton", name, &RIG_GLB.named("", "glb"), rig_glb(&tree))
            .part(
                "skeleton",
                name,
                &CLIP_LIBRARIES.named(LIBRARY, "glb"),
                library_glb(&tree, clips),
            )
    }

    /// A memory of whether something happened, labelled with its name and forgotten as `resets`
    /// says (`never`, `daily` or `leaving_area`).
    pub fn memory(self, name: &str, resets: &str) -> Game {
        self.row(
            "memory",
            name,
            json!({ "label": name, "type": "boolean", "resets": resets }),
        )
    }

    /// A resource item that stacks up to `max` in a bag slot.
    pub fn resource(self, name: &str, max: u32) -> Game {
        self.item(
            name,
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": max }] }),
        )
    }

    /// An item of `fields`, named after its file, with an icon and a quiet sound for every way it
    /// changes hands.
    pub fn item(self, name: &str, fields: Value) -> Game {
        let icon = format!("icons/{name}.png");
        let card = format!("cards/{name}_card.png");
        let sfx = path("sfx", "item");
        let mut row = json!({
            "name": name,
            "look": path("prop", DROP),
            "icon": icon,
            "card": card,
            "sfx": { "pickup": sfx, "trade": sfx, "drop": sfx },
        });
        for (field, value) in fields.as_object().expect("an item's fields") {
            row[field] = value.clone();
        }
        self.sfx("item")
            .prop(DROP, json!({ "type": "none" }))
            .row("item", name, row)
            .asset(&icon, png(item::ICON_SIZE.0, item::ICON_SIZE.1))
            .asset(&card, png(item::CARD_SIZE.0, item::CARD_SIZE.1))
    }

    /// Every new character starts with `items` equipped, the items declared under those names.
    pub fn equipped(self, items: &[&str]) -> Game {
        let items: Vec<String> = items.iter().map(|item| path("item", item)).collect();
        self.edit("player", "hero", |hero| hero["equipped"] = json!(items))
    }

    /// Changes the fields of a file declared before.
    pub fn edit(mut self, table: &str, name: &str, change: impl FnOnce(&mut Value)) -> Game {
        let file = fields_path(table, name);
        let bytes = self
            .files
            .get(&file)
            .unwrap_or_else(|| panic!("no {table} `{name}` declared"));
        let mut row: Value = serde_json::from_slice(bytes).expect("a declared file is json");
        change(&mut row);
        self.put(&file, json_bytes(&row));
        self
    }

    /// A character of `health`, drawn with a model of its own of the same name, that does only what
    /// its `modules` make it do.
    pub fn npc(self, name: &str, health: f32, modules: Value) -> Game {
        self.model(name, 1.5, json!([])).row(
            "npc",
            name,
            json!({
                "name": name,
                "model": path("model", name),
                "stats": stats(health),
                "modules": modules,
            }),
        )
    }

    /// Gives the model named `name` a bust for every generic expression, so its lines can show a
    /// face.
    pub fn busts(self, name: &str) -> Game {
        const EXPRESSIONS: [&str; 6] =
            ["neutral", "happy", "sad", "angry", "surprised", "thinking"];
        let game = EXPRESSIONS.iter().fold(self, |game, expression| {
            game.part(
                "model",
                name,
                &MODEL_BUSTS.named(expression, "png"),
                png(4, 4),
            )
        });
        game.edit("model", name, |model| {
            model["modules"]
                .as_array_mut()
                .expect("a model's modules")
                .push(json!({ "type": "bust" }))
        })
    }

    /// A sound of a moment's silence.
    pub fn sfx(self, name: &str) -> Game {
        self.row("sfx", name, json!({ "volume": 1.0, "pitch": 1.0 }))
            .part("sfx", name, &SFX_CLIP.named("", "wav"), wav())
    }

    /// A babbling voice of silent letters, a character's `voice` module names it by its path.
    pub fn babble(self, name: &str) -> Game {
        let game = self
            .row("babble_bank", name, json!({ "volume": 0.22 }))
            .row(
                "babble",
                name,
                json!({
                    "babble_bank": path("babble_bank", name),
                    "pitch": 1.0,
                    "range": 1.5,
                    "pace": 12.0,
                }),
            );
        ('a'..='z').fold(game, |game, letter| {
            let letter = BABBLE_LETTERS.named(&letter.to_string(), "wav");
            game.part("babble_bank", name, &letter, wav())
        })
    }

    /// An attention mark of a plain icon.
    pub fn mark(self, name: &str) -> Game {
        let icon = format!("icons/marks/{name}.png");
        self.row("attention", name, json!({ "icon": icon }))
            .asset(&icon, png(1, 1))
    }

    /// A visual effect shader that draws nothing, of no parameters, on a billboard.
    pub fn shader(self, name: &str) -> Game {
        self.shader_on(name, Stage::Billboard)
    }

    /// A visual effect shader that draws nothing, of no parameters, on `stage`; a cover that holds
    /// still.
    pub fn shader_on(self, name: &str, stage: Stage) -> Game {
        let mut source =
            String::from("fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(0.0);\n}\n");
        if stage == Stage::Cover {
            source.push_str(
                "fn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {\n    return world;\n}\n",
            );
        }
        self.row(
            "vfx_shader",
            name,
            json!({ "name": name, "stage": stage, "params": [] }),
        )
        .part(
            "vfx_shader",
            name,
            &SHADER_SOURCE.named("", "wgsl"),
            source.into_bytes(),
        )
    }

    /// A visual effect of one transparent frame, shown for a second.
    pub fn vfx(self, name: &str) -> Game {
        let sheet = format!("vfx/{name}.png");
        self.row(
            "vfx",
            name,
            json!({
                "name": name,
                "layers": [{
                    "type": "sprite",
                    "sheet": sheet,
                    "frame": { "width": 1, "height": 1 },
                    "frames": 1,
                    "fps": 1.0,
                }],
            }),
        )
        .asset(&sheet, png(1, 1))
    }

    /// A model nobody would notice on the plain skeleton [`BODY`], declared with it unless the test
    /// declared it before, half a metre around and `height` tall.
    pub fn model(self, name: &str, height: f32, modules: Value) -> Game {
        let game = if self.files.contains_key(&fields_path("skeleton", BODY)) {
            self
        } else {
            self.skeleton(BODY)
        };
        game.model_on(name, BODY, height, modules)
    }

    /// A model nobody would notice on the skeleton declared as `skeleton`, half a metre around and
    /// `height` tall, carrying no clips of its own.
    pub fn model_on(self, name: &str, skeleton: &str, height: f32, modules: Value) -> Game {
        self.row(
            "model",
            name,
            json!({
                "skeleton": path("skeleton", skeleton),
                "hitbox": { "radius": 0.5, "height": height },
                "modules": modules,
            }),
        )
        .part(
            "model",
            name,
            &MODEL_GLB.named("", "glb"),
            skin_glb(&rig(), &[]),
        )
    }

    /// A prop of a model nobody would notice, taking up `footprint` of the ground.
    pub fn prop(self, name: &str, footprint: Value) -> Game {
        self.row("prop", name, json!({ "footprint": footprint }))
            .part("prop", name, &PROP_GLB.named("", "glb"), glb(&[]))
    }

    /// A blueprint `width` × `height` metres, of `variables`, whose scene holds `nodes`.
    pub fn blueprint(
        self,
        name: &str,
        (width, height): (u32, u32),
        variables: Value,
        nodes: Value,
    ) -> Game {
        let mut fields = json!({
            "size": { "width": width, "height": height },
        });
        if variables
            .as_object()
            .is_some_and(|variables| !variables.is_empty())
        {
            fields["variables"] = variables;
        }
        self.row("blueprint", name, fields).part(
            "blueprint",
            name,
            &SCENE.named("", "json"),
            json_bytes(&nodes),
        )
    }

    /// Changes the scene of the area or blueprint named `name`, declared before.
    pub fn edit_scene(mut self, table: &str, name: &str, change: impl FnOnce(&mut Value)) -> Game {
        let file = format!("{}/{}", path(table, name), SCENE.named("", "json"));
        let bytes = self
            .files
            .get(&file)
            .unwrap_or_else(|| panic!("no {table} `{name}` declared"));
        let mut scene: Value = serde_json::from_slice(bytes).expect("a declared scene is json");
        change(&mut scene);
        self.put(&file, json_bytes(&scene));
        self
    }

    /// An area of `map`'s ground: flat turf, painted rock where the map is blocked and beyond a map
    /// smaller than the smallest terrain, lit by day, with `modules` in its scene.
    pub fn area(self, name: &str, map: &Map, modules: Value) -> Game {
        let (width, height) = map.size();
        let size = GroundSize {
            width: (width as u32).max(MIN_SIDE),
            height: (height as u32).max(MIN_SIDE),
        };
        let blocked = |x: usize, y: usize| {
            map.0
                .get(y)
                .and_then(|row| row.as_bytes().get(x))
                .is_none_or(|cell| *cell == b'#')
        };
        let game = self.terrains().row(
            "area",
            name,
            json!({
                "name": name,
                "size": { "width": size.width, "height": size.height },
                "max_slope": 38.0,
                "terrains": [path("terrain", TURF), path("terrain", ROCK)],
                "lighting": {
                    "sun": { "elevation": 52.0, "azimuth": 215.0, "color": [255, 236, 190] },
                    "ambient": [98, 122, 160],
                    "grade": { "color": [255, 255, 255], "strength": 0.0 },
                },
            }),
        );
        let game = game.part("area", name, &SCENE.named("", "json"), json_bytes(&modules));
        let padded = size.width as usize > width || size.height as usize > height;
        if !padded && !map.0.iter().any(|row| row.contains('#')) {
            return game;
        }
        let paint = Paint::from_fn(size, |column, row| {
            let (x, y) = (
                (column / SAMPLES_PER_METER) as usize,
                (row / SAMPLES_PER_METER) as usize,
            );
            u8::from(blocked(x, y))
        });
        game.part("area", name, &area::PAINT.named("", "png"), paint.encode())
    }

    /// The two terrains every declared area is painted with: turf to walk on and rock that
    /// blocks, each sounding like a step, a tick of a clip unlike any other declared sound.
    pub fn terrains(self) -> Game {
        self.sfx(STEP)
            .part(
                "sfx",
                STEP,
                &SFX_CLIP.named("", "wav"),
                wav_of(&[0, 64, 0, -64, 0, 64, 0, -64]),
            )
            .row(
                "surface",
                TURF,
                json!({ "name": "Turf", "footstep": path("sfx", STEP) }),
            )
            .row(
                "terrain",
                TURF,
                json!({
                    "name": "Turf",
                    "color": [106, 143, 59, 255],
                    "surface": path("surface", TURF),
                    "walk": "walkable",
                }),
            )
            .row(
                "terrain",
                ROCK,
                json!({
                    "name": "Rock",
                    "color": [96, 96, 104, 255],
                    "surface": path("surface", TURF),
                    "walk": "blocks",
                }),
            )
    }

    pub fn load(self) -> Content {
        self.try_load().unwrap_or_else(|report| panic!("{report}"))
    }

    /// Holds the game to everything a whole game must provide besides what it declares: every file
    /// the engine itself requires, and every quest and shop within a player's reach.
    pub fn whole(mut self) -> Game {
        self.whole = true;
        self
    }

    pub fn try_load(self) -> Result<Content, LoadReport> {
        engine::core::content::load(
            self.registry(),
            Arc::new(self.folder()),
            version::build(),
            "the test game did not load",
        )
    }

    /// What the game is held to: a whole game's every type, settings and fixture, or, unless it is
    /// whole, only the types of the files declared, with the settings if there are any.
    pub fn registry(&self) -> &'static ContentRegistry {
        match (self.whole, &self.settings) {
            (true, _) => engine::systems::registry(),
            (false, Some(_)) => &SETTLED_TABLES,
            (false, None) => &TABLES,
        }
    }

    /// Writes the folder's files under `dir`, as a game folder on disk.
    pub fn write_to(&self, dir: &Path) {
        for (path, bytes) in self.folder().files {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().expect("a parent folder")).expect("folders");
            std::fs::write(file, bytes).expect("a written file");
        }
    }

    /// The folder of every file declared, and of the settings file holding the settings.
    pub fn folder(&self) -> MemoryFolder {
        let mut files = self.files.clone();
        if let Some(settings) = &self.settings {
            let mut settings = settings.clone();
            if settings.get("content_version").is_none() {
                settings["content_version"] = json!(self.registry().version().0);
            }
            files.insert(SETTINGS_FILE.to_owned(), json_bytes(&settings).into());
        }
        MemoryFolder {
            name: "test game".to_owned(),
            files,
        }
    }

    fn put(&mut self, path: &str, bytes: Vec<u8>) {
        self.files.insert(path.to_owned(), bytes.into());
    }
}

/// The file holding the fields of the file of type `table` named `name`.
fn fields_path(table: &str, name: &str) -> String {
    let file = path(table, name);
    match engine::systems::registry().shape_of(table) {
        Some(Shape::Bundle) => format!("{file}/{CONFIG_FILE}"),
        _ => file,
    }
}

impl Default for Game {
    fn default() -> Game {
        Game::new()
    }
}

impl Map {
    /// Open ground, `width` × `height`.
    pub fn open(width: usize, height: usize) -> Map {
        Map(vec![".".repeat(width); height])
    }

    pub fn of(rows: &[&str]) -> Map {
        Map(rows.iter().map(|row| (*row).to_owned()).collect())
    }

    fn size(&self) -> (usize, usize) {
        (self.0.first().map_or(0, String::len), self.0.len())
    }
}

/// A fighter's stats at full `health`: six damage a hit at 1.2 hits a second from a metre and a half
/// away, walking four metres a second.
pub fn stats(health: f32) -> Value {
    json!({
        "health": health,
        "max_health": health,
        "damage": 6.0,
        "attack_speed": 1.2,
        "attack_delay": 200.0,
        "movement_speed": 4.0,
        "range": 1.5,
    })
}

fn json_bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).expect("json")
}
