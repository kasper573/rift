use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use engine::core::audio::playback::SFX_CLIP;
use engine::core::babble::BABBLE_LETTERS;
use engine::core::content::{
    CONFIG_FILE, Content, ContentFile, ContentId, ContentRegistry, LoadReport, MemoryFolder,
    SETTINGS_FILE, Shape, content_path, name_of, table_of, version,
};
use engine::core::time::{Millis, Seconds};
use engine::core::vfx::shader::SHADER_SOURCE;
use engine::systems::actor::Action;
use engine::systems::actor::model::{MODEL_BUSTS, MODEL_GLB};
use engine::systems::area::scene::{LAYERS, PALETTE, SCENE, WALK};
use engine::systems::area::tileset::TILESET_IMAGE;
use serde_json::{Value, json};
use strum::VariantArray;

use crate::art::{glb, png, wav};

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

/// The size in pixels of the tiles of every area and blueprint a test declares.
pub const TILE: u32 = 16;

/// How long every clip of a declared model plays.
pub const CLIP: Millis = Millis(300.0);

/// When a declared model's attack lands.
pub const APEX: Millis = Millis(100.0);

/// An area's ground as text, one string per row: `.` walkable, `#` blocked.
#[derive(Clone)]
pub struct Map(pub Vec<String>);

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

    /// A sword that, worn, lets its wearer strike: a basic attack on one enemy, landing the wearer's
    /// damage at the height of its swing.
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
        .item(
            "sword",
            json!({ "kind": {
                "type": "equipment",
                "slot": "weapon",
                "grants": [path("skill", "strike")],
            } }),
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
        let sfx = path("sfx", "item");
        let mut row = json!({
            "name": name,
            "icon": icon,
            "sfx": { "pickup": sfx, "trade": sfx, "drop": sfx },
        });
        for (field, value) in fields.as_object().expect("an item's fields") {
            row[field] = value.clone();
        }
        self.sfx("item")
            .row("item", name, row)
            .asset(&icon, png(1, 1))
    }

    /// Every new character starts wearing `items`, the items declared under those names.
    pub fn wearing(self, items: &[&str]) -> Game {
        let items: Vec<String> = items.iter().map(|item| path("item", item)).collect();
        self.edit("player", "hero", |hero| hero["wearing"] = json!(items))
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

    /// A visual effect shader that draws nothing, of no parameters.
    pub fn shader(self, name: &str) -> Game {
        self.row("vfx_shader", name, json!({ "name": name, "params": [] }))
            .part(
                "vfx_shader",
                name,
                &SHADER_SOURCE.named("", "wgsl"),
                b"fn vfx(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(0.0);\n}\n".to_vec(),
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

    /// A model of a rig nobody would notice, half a metre around and `height` tall, with a clip
    /// named after every action, each [`CLIP`] long; its attack lands at [`APEX`].
    pub fn model(self, name: &str, height: f32, modules: Value) -> Game {
        let clips: serde_json::Map<String, Value> = Action::VARIANTS
            .iter()
            .map(|action| {
                let cues = match action {
                    Action::Attack => json!([{ "at": APEX.0, "apex": true }]),
                    _ => json!([]),
                };
                (
                    action.as_ref().to_owned(),
                    json!({ "name": action.as_ref(), "cues": cues }),
                )
            })
            .collect();
        let rig: Vec<(&str, Seconds)> = Action::VARIANTS
            .iter()
            .map(|action| (action.as_ref(), CLIP.seconds()))
            .collect();
        self.row(
            "model",
            name,
            json!({
                "hitbox": { "radius": 0.5, "height": height },
                "clips": clips,
                "modules": modules,
            }),
        )
        .part("model", name, &MODEL_GLB.named("", "glb"), glb(&rig))
    }

    /// A blueprint `width` × `height` tiles, of `variables`, whose scene holds `nodes`.
    pub fn blueprint(
        self,
        name: &str,
        (width, height): (u32, u32),
        variables: Value,
        nodes: Value,
    ) -> Game {
        let mut fields = json!({
            "map": { "size": { "width": width, "height": height }, "tile": TILE },
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

    /// An area of `map`'s ground, painted in one ground layer with one plain tile, with `modules`
    /// after it in its scene.
    pub fn area(self, name: &str, map: &Map, modules: Value) -> Game {
        let tileset = format!("{name}_ground");
        let (width, height) = map.size();
        let ground = vec![vec!["0"; width]; height];
        let walk: Vec<Vec<&str>> = map
            .0
            .iter()
            .map(|row| {
                row.chars()
                    .map(|cell| if cell == '#' { "0" } else { "1" })
                    .collect()
            })
            .collect();
        let mut scene = vec![json!({ "type": "ground_layer", "name": "ground" })];
        scene.extend(
            modules
                .as_array()
                .expect("an area's modules")
                .iter()
                .cloned(),
        );
        self.row("tileset", &tileset, json!({ "tile": TILE, "tiles": {} }))
            .part(
                "tileset",
                &tileset,
                &TILESET_IMAGE.named("", "png"),
                png(TILE, TILE),
            )
            .row(
                "area",
                name,
                json!({
                    "name": name,
                    "blueprint": {
                        "map": { "size": { "width": width, "height": height }, "tile": TILE },
                    },
                }),
            )
            .part(
                "area",
                name,
                &SCENE.named("", "json"),
                json_bytes(&json!(scene)),
            )
            .part(
                "area",
                name,
                &PALETTE.named("", "csv"),
                format!("tileset,tile,flip\n{},0,\n", path("tileset", &tileset)),
            )
            .part("area", name, &LAYERS.named("ground", "csv"), csv(&ground))
            .part("area", name, &WALK.named("", "csv"), csv(&walk))
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

/// A fighter's stats at full `health`: six damage a hit at 1.2 hits a second from a tile and a half
/// away, walking four tiles a second.
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

fn csv(rows: &[Vec<&str>]) -> Vec<u8> {
    let mut text = String::new();
    for row in rows {
        text.push_str(&row.join(","));
        text.push('\n');
    }
    text.into_bytes()
}
