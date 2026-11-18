use engine::core::card::CardStyleFixture;
use engine::core::content::fixture::FixtureLock;
use engine::core::content::{ContentFile, Fixture};
use engine::core::vfx::{ParamKind, ShaderParam};
use engine::systems::area::world_map::WorldMap;
use engine::systems::attention::AttentionMark;
use engine::systems::chat::ChatChannel;
use engine::systems::class::LevelVfx;
use engine::systems::combat::DefeatVfx;
use engine::systems::input::map::{BoundAction, InputAction, InputBinding, InputDef, InputFixture};
use engine::systems::interface::{CursorShape, InterfaceIcon, InterfaceSound};
use engine::systems::notification::pop::PopShader;
use engine::systems::skill::aim::AreaShader;
use engine::systems::stat::VitalVfx;
use serde_json::{Value, json};
use strum::VariantArray;

use crate::Game;

impl Game {
    /// Every row the engine itself requires, each in its plainest form that keeps what the engine
    /// reads from it, so a game of only the rows a test declares loads as a whole one. A row the
    /// test declares after replaces the plain one.
    pub fn engine_rows(self) -> Game {
        let game = self.inputs();
        let game = each::<AttentionMark>(game, |game, key, _| game.mark(key));
        let game = each::<InterfaceSound>(game, |game, key, _| game.sfx(key));
        let game = each::<CursorShape>(game, |game, key, _| game.cursor(key));
        let game = each::<InterfaceIcon>(game, |game, key, _| game.interface_icon(key));
        let game = each::<ChatChannel>(game, |game, key, _| {
            let channel = json!({ "name": key, "color": [255, 255, 255] });
            game.row("chat_channel", key, channel)
        });
        let game = each::<VitalVfx>(game, |game, key, _| game.vfx(key));
        let game = each::<DefeatVfx>(game, |game, key, _| game.vfx(key));
        let game = each::<LevelVfx>(game, |game, key, _| game.vfx(key));
        let game = each::<PopShader>(game, shader_fixture);
        let game = each::<AreaShader>(game, shader_fixture);
        let game = each::<WorldMap>(game, |game, key, _| game.row("world_map", key, json!({})));
        each::<CardStyleFixture>(game, |game, key, _| game.card_style(key, json!({})))
    }

    /// The input file with every action named and none bound: what a server needs to word the
    /// inputs a line of text names.
    pub fn inputs(self) -> Game {
        self.bound(&[])
    }

    /// The input file with every action named, and bound only as `bound` says.
    pub fn bound(self, bound: &[(InputAction, InputBinding)]) -> Game {
        let file = InputDef {
            actions: InputAction::VARIANTS
                .iter()
                .map(|&action| {
                    let label = <&str>::from(action).to_owned();
                    let bindings = bound
                        .iter()
                        .filter(|(to, _)| *to == action)
                        .map(|(_, binding)| *binding)
                        .collect();
                    (action, BoundAction { label, bindings })
                })
                .collect(),
        };
        let file = serde_json::to_value(file).expect("an input file");
        each::<InputFixture>(self, |game, key, _| {
            game.row(InputDef::TABLE, key, file.clone())
        })
    }
}

fn shader_fixture(game: Game, key: &str, locks: Vec<FixtureLock>) -> Game {
    game.shader(key).edit("vfx_shader", key, |shader| {
        shader["params"] = Value::Array(locks.iter().filter_map(param).collect());
        for lock in &locks {
            if let FixtureLock::Field { field, value } = lock
                && let Some(held) = shader.pointer_mut(field)
            {
                *held = value.clone();
            }
        }
    })
}

fn each<F: Fixture>(game: Game, add: impl Fn(Game, &str, Vec<FixtureLock>) -> Game) -> Game {
    F::VARIANTS.iter().fold(game, |game, fixture| {
        add(game, fixture.name(), fixture.locks())
    })
}

/// The shader parameter a lock keeps, holding its kind's blank value.
fn param(lock: &FixtureLock) -> Option<Value> {
    let FixtureLock::Entry { list, entry } = lock else {
        return None;
    };
    if list != "/params" {
        return None;
    }
    let name = entry["name"].as_str()?.to_owned();
    let kind: ParamKind = serde_json::from_value(entry["kind"].clone()).ok()?;
    let param = ShaderParam::of(&name, kind);
    serde_json::to_value(param).ok()
}
