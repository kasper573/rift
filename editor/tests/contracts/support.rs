use std::path::{Path, PathBuf};
use std::sync::Arc;
use ui::egui;

use editor::app::Editor;
use editor::project::{Project, ProjectFiles};
use editor::schema::SchemaShapes;
use editor::user::{EditorSettings, UserFolder};
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::systems::GameSettings;
use engine::systems::area::AreaModule;
use engine::systems::area::object::{ObjectDef, ObjectShape};
use engine::systems::area::scene::{ModuleNode, SceneFolder, SceneNode};
use serde_json::json;
use testkit::{Game, Map};

/// The area every game of the editor's tests starts in.
pub const AREA: &str = "start";

/// The path of the area every game of the editor's tests starts in.
pub fn area() -> String {
    testkit::path("area", AREA)
}

/// A whole game of nothing but a field to start in and the rows the engine itself requires,
/// written to `folder`, which it returns.
pub fn game_folder(folder: PathBuf) -> PathBuf {
    game().write_to(&folder);
    folder
}

pub fn game() -> Game {
    Game::new()
        .area(AREA, &Map::open(8, 8), json!([]))
        .hero(AREA, 1.0, 1.0)
        .engine_rows()
        .whole()
}

pub fn game_name(folder: &Path) -> String {
    let folder = Arc::new(engine::core::content::FilesystemFolder(
        folder.to_path_buf(),
    ));
    engine::core::content::FolderReader::new(engine::systems::registry(), folder)
        .settings::<GameSettings>()
        .expect("the settings read")
        .name
}

/// `game` written into `scratch` and open in an editor, its folder loading whole.
pub fn open(game: Game, scratch: &tempfile::TempDir) -> Editor {
    let mut editor = open_as_is(game, scratch);
    settled(&mut editor);
    editor
}

/// `game` written into `scratch` and open in an editor, however it loads.
pub fn open_as_is(game: Game, scratch: &tempfile::TempDir) -> Editor {
    let folder = scratch.path().join("game");
    game.write_to(&folder);
    let mut project = Project::open(&folder, Arc::new(SchemaShapes::load())).expect("opens");
    project.settle();
    let mut editor = Editor::new(
        UserFolder::at(scratch.path().join("user")),
        EditorSettings::default(),
    );
    editor.project = Some(project);
    editor
}

/// The project open in `editor` once what it is loading settles, its folder loading whole.
pub fn settled(editor: &mut Editor) -> &Project {
    let project = editor.project.as_mut().expect("a project");
    project.settle();
    if let Some(report) = project.report.as_ref().filter(|_| !project.loads()) {
        panic!("{report}");
    }
    project
}

/// An object named `name` at a point of the map.
pub fn object(name: &str) -> SceneNode {
    SceneNode::Module(ModuleNode::new(AreaModule::Object(ObjectDef {
        name: Some(name.to_owned()),
        title: None,
        shape: ObjectShape::Point(Pos::new(1.5, 1.5)),
        look: None,
        shown: None,
        scripts: Vec::new(),
    })))
}

/// A folder labelled `label` holding `children`.
pub fn folder(label: &str, children: Vec<SceneNode>) -> SceneNode {
    SceneNode::Folder(SceneFolder {
        label: label.to_owned(),
        hidden: false,
        locked: false,
        children,
    })
}

/// What names each node of `nodes`, depth first: an object's name, a folder's label.
pub fn names(nodes: &[SceneNode]) -> Vec<String> {
    let mut found = Vec::new();
    editor::views::area::nodes::walk(nodes, &mut |_, node| {
        let name = match node {
            SceneNode::Module(ModuleNode {
                module: AreaModule::Object(object),
                ..
            }) => object.name.clone(),
            node => node.label().map(str::to_owned),
        };
        found.push(name.unwrap_or_default());
    });
    found
}

/// What the project open in `editor` loads once what it is loading settles.
pub fn content(editor: &mut Editor) -> &Content {
    settled(editor).content.as_ref().expect("the folder loads")
}

/// The files of the project open in `editor` once what it is loading settles, however they load.
pub fn files(editor: &mut Editor) -> &ProjectFiles {
    let project = editor.project.as_mut().expect("a project");
    project.settle();
    &project.files
}

/// The npcs living in the start area, by path, in the order its scene lists them.
pub fn residents(editor: &mut Editor) -> Vec<String> {
    files(editor);
    let reader = editor.reader().expect("a reader");
    editor::views::area::nodes::modules(&editor::views::area::nodes::scene_of(editor, &area()))
        .iter()
        .filter_map(|module| match module.module() {
            AreaModule::Resident(resident) => reader.path_of(resident.npc).map(str::to_owned),
            _ => None,
        })
        .collect()
}

/// A view drawn frame after frame in one context on a screen of `screen`, as the editor draws it.
pub struct Frames {
    ctx: egui::Context,
    screen: egui::Vec2,
    time: f64,
}

impl Frames {
    pub fn new(screen: egui::Vec2) -> Frames {
        let ctx = egui::Context::default();
        ui::apply(&ctx);
        Frames {
            ctx,
            screen,
            time: 0.0,
        }
    }

    /// Draws a frame of `events` with `show`, giving the texts it drew.
    pub fn run(&mut self, events: Vec<egui::Event>, show: impl FnMut(&mut egui::Ui)) -> Drawn {
        self.time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.screen)),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, show);
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            texts_of(&clipped.shape, &mut texts);
        }
        Drawn(texts)
    }
}

/// The texts a frame drew, each where it was drawn.
#[derive(Debug, Default)]
pub struct Drawn(pub Vec<(String, egui::Rect)>);

impl Drawn {
    /// Where `text` is drawn, the first place when it is drawn more than once.
    pub fn rect(&self, text: &str) -> Option<egui::Rect> {
        self.0
            .iter()
            .find(|(drawn, _)| drawn == text)
            .map(|(_, rect)| *rect)
    }

    /// The middle of the last place `text` is drawn.
    pub fn at(&self, text: &str) -> egui::Pos2 {
        self.0
            .iter()
            .rev()
            .find(|(drawn, _)| drawn == text)
            .map(|(_, rect)| rect.center())
            .unwrap_or_else(|| panic!("{text} is drawn: {self:?}"))
    }

    pub fn any(&self, holds: impl Fn(&str) -> bool) -> bool {
        self.0.iter().any(|(text, _)| holds(text))
    }
}

/// The frames of a click at `at`: the pointer moving there, pressing, then letting go.
pub fn click(at: egui::Pos2) -> [Vec<egui::Event>; 3] {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    [
        vec![egui::Event::PointerMoved(at)],
        vec![button(true)],
        vec![button(false)],
    ]
}

/// A press and release of `key` held with `modifiers`, in one frame.
pub fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    let held = (!modifiers.is_none()).then_some(egui::Event::ModifiersChanged(modifiers));
    held.into_iter()
        .chain([true, false].map(|pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }))
        .collect()
}

fn texts_of(shape: &egui::Shape, texts: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Text(text) => texts.push((
            text.galley.text().to_owned(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                texts_of(shape, texts);
            }
        }
        _ => {}
    }
}
