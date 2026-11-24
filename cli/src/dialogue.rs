//! The dialogue drawn off screen, once for every style a line is set in, alone and together, and
//! every state a choice and its tags show, over grounds from the lightest to the darkest and the
//! busiest, and over any pictures given: how it reads over whatever a game puts behind it, seen in
//! one go.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::window::ExitCondition;
use bevy_egui::{EguiPrimaryContextPass, PrimaryEguiContext};
use engine::core::assets::loader::PngLoader;
use engine::core::content::{Content, LoadPurpose};
use engine::core::gui::{self, Band, GuiPlugin};
use engine::core::text::{LineText, SpanText, TextFx, TextInk, TextMotion, TextVoice};
use engine::systems::GameSettings;
use engine::systems::dialogue::sheet::{self, DialogueSheet, SheetChoice};
use engine::systems::dialogue::{ChoiceChip, ChoiceTag, ChoiceView};
use engine::systems::input::map::{InputAction, InputMap};
use engine::systems::item::inline::InlineItem;
use engine::systems::item::{ItemDef, ItemId};
use engine::systems::npc::NpcDef;
use ui::egui::{Color32, ColorImage, LayerId, Pos2, Rect, TextureHandle, TextureOptions};
use ui::motion::SheetPose;

/// Draws the dialogue of the game in `folder` over every ground and every picture of `over`, as
/// the pictures `<ground>-<specimen>.png` of the folder `out`.
pub fn run(folder: &Path, out: &Path, over: &[PathBuf]) {
    let folder = std::fs::canonicalize(folder).expect("a game folder");
    let content = crate::folder::load(&folder, "the game did not load", LoadPurpose::Play)
        .unwrap_or_else(|report| panic!("{report}"));
    std::fs::create_dir_all(out).expect("a folder to draw into");
    let settings = content.built::<GameSettings>().clone();
    let inputs = InputMap::new(&content);
    let mut grounds = grounds();
    grounds.extend(over.iter().map(|path| picture(path)));
    let specimens = specimens(&content);
    let out = out.to_path_buf();
    let mut app = App::new();
    engine::core::assets::serve_folder(&mut app, &folder);
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>(),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
        1.0 / 60.0,
    )))
    .add_plugins(GuiPlugin)
    .register_asset_loader(PngLoader)
    .insert_resource(inputs.key_names())
    .insert_resource(inputs)
    .insert_resource(settings.lettering)
    .insert_resource(settings.typing)
    .insert_resource(settings.cards)
    .insert_resource(settings)
    .insert_resource(content)
    .insert_resource(Drawing {
        out,
        grounds,
        specimens,
        at: 0,
        frames: 0,
        height: 0.0,
        reading: None,
        image: None,
    })
    .init_resource::<Read>()
    .add_observer(|read: On<ReadbackComplete>, mut taken: ResMut<Read>| {
        taken.0.push((read.entity, read.data.clone()));
    })
    .add_systems(Startup, aim)
    .add_systems(EguiPrimaryContextPass, draw.in_set(Band::Stage))
    .add_systems(Update, step)
    .run();
}

/// How large the dialogue is drawn: a screen many play on.
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
/// Frames a sheet stands before it is read back: it lays out over two passes, and its pictures
/// load.
const SETTLE: u32 = 20;

/// A sheet of the dialogue and the name of the pictures it is drawn in.
struct Specimen {
    name: &'static str,
    sheet: DialogueSheet,
}

/// What the dialogue is drawn over.
struct Ground {
    name: String,
    fill: Fill,
}

enum Fill {
    /// Bands of every colour in turn, across the screen.
    Bands(Vec<Color32>),
    Picture(ColorImage),
}

#[derive(Resource)]
struct Drawing {
    out: PathBuf,
    grounds: Vec<Ground>,
    specimens: Vec<Specimen>,
    /// Which sheet over which ground is on show: the specimens of each ground in turn.
    at: usize,
    frames: u32,
    height: f32,
    reading: Option<Entity>,
    image: Option<Handle<Image>>,
}

impl Drawing {
    fn ground(&self) -> &Ground {
        &self.grounds[self.at / self.specimens.len()]
    }

    fn specimen(&self) -> &Specimen {
        &self.specimens[self.at % self.specimens.len()]
    }

    fn all(&self) -> usize {
        self.grounds.len() * self.specimens.len()
    }
}

#[derive(Resource, Default)]
struct Read(Vec<(Entity, Vec<u8>)>);

/// The lightest, the darkest and the most saturated grounds, each alone and side by side.
fn grounds() -> Vec<Ground> {
    let light = Color32::from_rgb(244, 241, 232);
    let dark = Color32::from_rgb(12, 12, 14);
    let wheat = Color32::from_rgb(217, 184, 96);
    let grass = Color32::from_rgb(127, 168, 74);
    let sky = Color32::from_rgb(140, 190, 230);
    let blood = Color32::from_rgb(170, 40, 36);
    let alone = |name: &str, color: Color32| Ground {
        name: name.to_owned(),
        fill: Fill::Bands(vec![color]),
    };
    vec![
        alone("light", light),
        alone("dark", dark),
        alone("wheat", wheat),
        alone("grass", grass),
        Ground {
            name: "bands".to_owned(),
            fill: Fill::Bands(vec![light, dark, wheat, grass, sky, blood, light, dark]),
        },
    ]
}

fn picture(path: &Path) -> Ground {
    let picture = image::open(path)
        .unwrap_or_else(|error| panic!("{} is no picture: {error}", path.display()))
        .to_rgba8();
    let size = [picture.width() as usize, picture.height() as usize];
    Ground {
        name: path.file_stem().map_or_else(
            || "picture".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        ),
        fill: Fill::Picture(ColorImage::from_rgba_unmultiplied(size, picture.as_raw())),
    }
}

fn aim(world: &mut World) {
    let mut image = Image::new_target_texture(WIDTH, HEIGHT, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let image = world.resource_mut::<Assets<Image>>().add(image);
    world.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::Image(image.clone().into()),
        Msaa::Off,
        PrimaryEguiContext,
    ));
    world.resource_mut::<Drawing>().image = Some(image);
}

fn draw(world: &mut World, mut pictures: Local<Vec<(usize, TextureHandle)>>) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    world.resource_scope(|world, mut drawing: Mut<Drawing>| {
        if drawing.at >= drawing.all() {
            return;
        }
        let ground = drawing.at / drawing.specimens.len();
        let screen = gui.content_rect();
        let painter = gui.layer_painter(LayerId::background());
        match &drawing.ground().fill {
            Fill::Bands(colors) => {
                let width = screen.width() / colors.len() as f32;
                for (index, color) in colors.iter().enumerate() {
                    let left = screen.left() + width * index as f32;
                    let band = Rect::from_min_max(
                        Pos2::new(left, screen.top()),
                        Pos2::new(left + width, screen.bottom()),
                    );
                    painter.rect_filled(band, 0.0, *color);
                }
            }
            Fill::Picture(image) => {
                let texture = match pictures.iter().find(|(of, _)| *of == ground) {
                    Some((_, texture)) => texture.clone(),
                    None => {
                        let texture = gui.load_texture(
                            drawing.ground().name.clone(),
                            image.clone(),
                            TextureOptions::LINEAR,
                        );
                        pictures.push((ground, texture.clone()));
                        texture
                    }
                };
                painter.image(
                    texture.id(),
                    screen,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        let drawn = sheet::draw(
            world,
            &gui,
            &drawing.specimen().sheet,
            SheetPose::REST,
            drawing.height,
        );
        drawing.height = drawn.height;
    });
}

fn step(world: &mut World) {
    world.resource_scope(|world, mut drawing: Mut<Drawing>| {
        if drawing.at >= drawing.all() {
            return;
        }
        let Some(reading) = drawing.reading else {
            drawing.frames += 1;
            if drawing.frames >= SETTLE {
                let image = drawing.image.clone().expect("the picture to draw into");
                drawing.reading = Some(world.spawn(ReadbackOnce::texture(image)).id());
            }
            return;
        };
        let mut read = world.resource_mut::<Read>();
        let Some(at) = read.0.iter().position(|(entity, _)| *entity == reading) else {
            return;
        };
        let pixels = read.0.remove(at).1;
        let name = format!(
            "{}-{:02}-{}.png",
            drawing.ground().name,
            drawing.at % drawing.specimens.len(),
            drawing.specimen().name
        );
        image::RgbaImage::from_raw(WIDTH, HEIGHT, pixels)
            .expect("a whole picture read back")
            .save(drawing.out.join(name))
            .expect("the picture saved");
        drawing.at += 1;
        drawing.frames = 0;
        drawing.height = 0.0;
        drawing.reading = None;
        if drawing.at >= drawing.all() {
            world.write_message(AppExit::Success);
        }
    });
}

/// Every style a line is set in and every state a choice shows, said by the game's first
/// townsperson with the game's first items, in words that tell what each specimen shows.
fn specimens(content: &Content) -> Vec<Specimen> {
    let speaker = content
        .table::<NpcDef>()
        .rows()
        .next()
        .map(|npc| npc.name.clone());
    let mut items = content.ids::<ItemDef>();
    let item = items.next().expect("a game with an item");
    let other = items.next().unwrap_or(item);
    let said = |line: LineText| DialogueSheet {
        speaker: speaker.clone(),
        line: Some(line),
        typed: None,
        waits: true,
        choices: Vec::new(),
    };
    let lines = [
        (
            "plain",
            vec![plain("A plain line, as most of what is said reads.")],
        ),
        (
            "inks",
            vec![
                plain("Words in every ink: "),
                fx("danger", &[TextFx::Ink(TextInk::Danger)]),
                plain(", "),
                fx("an item", &[TextFx::Ink(TextInk::Item)]),
                plain(", "),
                fx("a place", &[TextFx::Ink(TextInk::Place)]),
                plain(", "),
                fx("a name", &[TextFx::Ink(TextInk::Name)]),
                plain(" and "),
                fx("magic", &[TextFx::Ink(TextInk::Magic)]),
                plain("."),
            ],
        ),
        (
            "whisper",
            vec![fx(
                "A whole line whispered, soft and slanted, the way a secret is told.",
                &[TextFx::Voice(TextVoice::Whisper)],
            )],
        ),
        (
            "shout",
            vec![
                fx("Shouted", &[TextFx::Voice(TextVoice::Shout)]),
                plain(" at the start, in the "),
                fx("middle", &[TextFx::Voice(TextVoice::Shout)]),
                plain(" and at the "),
                fx("end!", &[TextFx::Voice(TextVoice::Shout)]),
            ],
        ),
        (
            "voices",
            vec![
                fx("Whispered, ", &[TextFx::Voice(TextVoice::Whisper)]),
                plain("then plain, then "),
                fx("shouted", &[TextFx::Voice(TextVoice::Shout)]),
                plain(", then "),
                fx("whispered again.", &[TextFx::Voice(TextVoice::Whisper)]),
            ],
        ),
        (
            "motions",
            vec![
                plain("Words that "),
                fx("wave", &[TextFx::Motion(TextMotion::Wave)]),
                plain(", "),
                fx("shake", &[TextFx::Motion(TextMotion::Shake)]),
                plain(" and "),
                fx("pulse", &[TextFx::Motion(TextMotion::Pulse)]),
                plain(", and "),
                fx("words said slowly", &[TextFx::Slow]),
                plain("."),
            ],
        ),
        (
            "together",
            vec![
                fx(
                    "Shouted danger shaking",
                    &[
                        TextFx::Voice(TextVoice::Shout),
                        TextFx::Ink(TextInk::Danger),
                        TextFx::Motion(TextMotion::Shake),
                    ],
                ),
                plain(", "),
                fx(
                    "whispered magic pulsing",
                    &[
                        TextFx::Voice(TextVoice::Whisper),
                        TextFx::Ink(TextInk::Magic),
                        TextFx::Motion(TextMotion::Pulse),
                    ],
                ),
                plain(" and "),
                fx(
                    "a waving place",
                    &[
                        TextFx::Ink(TextInk::Place),
                        TextFx::Motion(TextMotion::Wave),
                    ],
                ),
                plain("."),
            ],
        ),
        (
            "keys",
            vec![
                plain("Keys stand in a line: "),
                InputAction::Interact.into(),
                plain(" to act, "),
                InputAction::AddToChat.into(),
                plain(" to share, and "),
                InputAction::OpenCharacter.into(),
                plain(" for the character."),
            ],
        ),
        (
            "long",
            vec![
                plain("A long line wraps over rows and keeps every style where it falls: "),
                fx("a name", &[TextFx::Ink(TextInk::Name)]),
                plain(" here, "),
                fx("something whispered", &[TextFx::Voice(TextVoice::Whisper)]),
                plain(" there, a "),
                fx("SHOUT", &[TextFx::Voice(TextVoice::Shout)]),
                plain(" in the middle of a row, "),
                InputAction::Interact.into(),
                plain(" at a row's end, and "),
                fx("danger", &[TextFx::Ink(TextInk::Danger)]),
                plain(" near the last of it, so every row keeps one baseline."),
            ],
        ),
    ];
    let mut specimens: Vec<Specimen> = lines
        .into_iter()
        .map(|(name, spans)| Specimen {
            name,
            sheet: said(LineText(spans)),
        })
        .collect();
    specimens.push(Specimen {
        name: "narrator",
        sheet: DialogueSheet {
            speaker: None,
            ..said(LineText(vec![plain(
                "The narrator speaks with no name before the line, as the world itself would.",
            )]))
        },
    });
    specimens.push(Specimen {
        name: "typing",
        sheet: DialogueSheet {
            typed: Some(24),
            waits: false,
            ..said(LineText(vec![plain(
                "A line halfway typed out shows only what has been said so far.",
            )]))
        },
    });
    let ask = |choices: Vec<SheetChoice>| DialogueSheet {
        waits: false,
        choices,
        ..said(LineText(vec![plain("Choices, in every state they show.")]))
    };
    specimens.push(Specimen {
        name: "choices",
        sheet: ask(vec![
            choice(vec![plain("An open choice.")], Vec::new(), false, false),
            choice(
                vec![plain("The choice the player is on.")],
                Vec::new(),
                true,
                false,
            ),
            choice(
                vec![plain("A locked choice.")],
                vec![ChoiceChip::Needs {
                    what: LineText::plain("a need unmet"),
                    met: false,
                }],
                false,
                true,
            ),
            choice(
                vec![plain("A need met.")],
                vec![ChoiceChip::Needs {
                    what: LineText::plain("a need met"),
                    met: true,
                }],
                false,
                false,
            ),
            choice(
                vec![plain("Paid for with what the player has.")],
                vec![pays(item, 2, 5)],
                false,
                false,
            ),
            choice(
                vec![plain("Paid for with more than the player has.")],
                vec![pays(other, 9, 1)],
                false,
                true,
            ),
            choice(
                vec![plain("A reward.")],
                vec![ChoiceChip::Gets { item, count: 3 }],
                false,
                false,
            ),
            choice(
                vec![plain("Everything at once.")],
                vec![
                    ChoiceChip::Needs {
                        what: LineText::plain("a need met"),
                        met: true,
                    },
                    pays(item, 1, 5),
                    ChoiceChip::Gets {
                        item: other,
                        count: 1,
                    },
                ],
                false,
                false,
            ),
            choice(vec![plain("Goodbye.")], Vec::new(), false, false),
        ]),
    });
    let mut warned = choice(
        vec![plain("A choice with a warning.")],
        Vec::new(),
        false,
        false,
    );
    warned.view.warn = Some("A warning".to_owned());
    let mut refused = choice(
        vec![plain("A choice just turned down.")],
        Vec::new(),
        false,
        false,
    );
    refused.refused = true;
    let mut pictured = choice(
        vec![plain("A choice with a picture.")],
        Vec::new(),
        false,
        false,
    );
    pictured.view.icon = Some(item.get(content).icon.clone());
    specimens.push(Specimen {
        name: "choices-styled",
        sheet: ask(vec![
            choice(
                vec![
                    plain("A choice with "),
                    fx("a name", &[TextFx::Ink(TextInk::Name)]),
                    plain(", "),
                    fx("a whisper", &[TextFx::Voice(TextVoice::Whisper)]),
                    plain(" and a "),
                    fx("SHOUT", &[TextFx::Voice(TextVoice::Shout)]),
                    plain("."),
                ],
                Vec::new(),
                false,
                false,
            ),
            choice(
                vec![plain("Trade "), InlineItem::of(item).count(2).into(), plain(".")],
                Vec::new(),
                false,
                false,
            ),
            choice(
                vec![plain(
                    "A long choice that wraps over more than one row of the dialogue, beside its tags.",
                )],
                vec![pays(item, 1, 5), ChoiceChip::Gets { item: other, count: 2 }],
                true,
                false,
            ),
            warned,
            refused,
            pictured,
            choice(
                vec![plain("A locked choice the player is on.")],
                vec![ChoiceChip::Needs {
                    what: LineText::plain("a need unmet"),
                    met: false,
                }],
                true,
                true,
            ),
        ]),
    });
    specimens
}

fn plain(text: &str) -> SpanText {
    fx(text, &[])
}

fn fx(text: &str, fx: &[TextFx]) -> SpanText {
    SpanText::Text {
        text: text.to_owned(),
        fx: fx.to_vec(),
    }
}

fn pays(item: ItemId, count: u32, have: u32) -> ChoiceChip {
    ChoiceChip::Pays { item, count, have }
}

fn choice(label: Vec<SpanText>, chips: Vec<ChoiceChip>, lit: bool, locked: bool) -> SheetChoice {
    SheetChoice {
        view: ChoiceView {
            label: LineText(label),
            tag: ChoiceTag::Dialogue,
            icon: None,
            chips,
            locked,
            warn: None,
        },
        lit,
        refused: false,
    }
}
