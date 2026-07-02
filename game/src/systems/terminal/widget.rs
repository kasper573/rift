use crate::data::terminal::Id;
use bevy::prelude::*;
use bevy::scene::EntityScene;
use bevy_terminal::TerminalInput;

use crate::systems::hud::{Window, reconcile_children};
use crate::systems::terminal::Terminals;

pub struct TerminalWindow;

impl Window for TerminalWindow {
    fn title(&self) -> &'static str {
        "Terminal"
    }
    fn toggle(&self) -> KeyCode {
        KeyCode::KeyC
    }
    fn keybind(&self) -> &'static str {
        "C"
    }
    fn icon(&self) -> &'static str {
        "icons/misc/scroll.png"
    }
    fn order(&self) -> u32 {
        4
    }
    fn contents(&self, world: &World) -> Vec<ui::WindowContent> {
        let terminals = world.resource::<Terminals>();
        if terminals.tabs.is_empty() {
            return crate::systems::hud::single_tab("Terminal", ui::text("Waiting for server…"));
        }
        terminals
            .tabs
            .iter()
            .map(|&terminal| ui::WindowContent {
                title: tab_title(terminal).to_owned(),
                scene: Box::new(tab_scene(terminal)),
            })
            .collect()
    }
    fn sync(&self, world: &mut World) {
        sync_logs(world)
    }
}

#[derive(Component, Clone)]
struct TerminalLog {
    terminal: Id,
}

fn tab_title(terminal: Id) -> &'static str {
    match terminal {
        Id::Global => "Global",
        Id::Admin => "Admin",
    }
}

fn tab_scene(terminal: Id) -> impl Scene {
    bsn! {
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
        }
        Children [
            ( Node { flex_grow: 1.0, min_height: Val::Px(0.0), width: Val::Percent(100.0) }
              Children [
                ( {ui::scroll_area()}
                  Children [
                    ( {ui::scroll_viewport()}
                      {ui::component(ui::PinToBottom::default())}
                      Children [
                        (
                            Node {
                                flex_direction: FlexDirection::Column,
                                width: Val::Percent(100.0),
                                padding: {UiRect::all(Val::Px(4.0))},
                            }
                            {ui::component(TerminalLog { terminal })}
                        )
                      ]
                    ),
                    ( {ui::scroll_bar()} Children [ {EntityScene(ui::scroll_thumb())} ] )
                  ]
                )
              ]
            ),
            {EntityScene(ui::text_input(ui::TextInputOptions {
                on_submit: ui::OnSubmit::new(move |world, text| {
                    world.write_message(TerminalInput { terminal, text });
                }),
            }))}
        ]
    }
}

fn sync_logs(world: &mut World) {
    let logs: Vec<(Entity, Id)> = world
        .query::<(Entity, &TerminalLog)>()
        .iter(world)
        .map(|(entity, log)| (entity, log.terminal))
        .collect();
    for (container, terminal) in logs {
        let lines: Vec<(u64, String)> = world
            .resource::<Terminals>()
            .lines(terminal)
            .cloned()
            .collect();
        let keys: Vec<u64> = lines.iter().map(|(seq, _)| *seq).collect();
        reconcile_children(world, container, &keys, |index| {
            Box::new(ui::text(lines[index].1.clone()))
        });
    }
}
