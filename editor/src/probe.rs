//! The editor's probe: answers a driver's questions about the editor as it runs, each with a frame
//! after it, as the client's probe answers about the game.

use std::net::SocketAddr;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContext, EguiPostUpdateSet, PrimaryEguiContext};
use engine::core::gui;
use engine::core::math::{Pos, Size};
use engine::core::units::Meters;
use engine::systems::probe::socket::ProbeSocket;
use engine::systems::probe::{
    self, DrawnUi, EditorFrameQuestion, EditorPanel, EditorScenery, EditorSnapshot, EditorStage,
    EditorStageMark, EditorTree, EditorTreeRow, EditorTrees, FrameSamples, ProbeInput,
    ProbeInputSeen, ProbeInterfacePlugin,
};

use crate::app::{DrawnPasses, Editor, keep_pass, primary_context};
use crate::shown;

/// Marks an editor a probe watches, which then reports what only a probe reads.
#[derive(Resource)]
pub struct Probed;

/// Answers the driver at `driver`, a frame after each question.
pub fn install(app: &mut App, driver: SocketAddr) {
    let socket = ProbeSocket::<EditorFrameQuestion>::connect(driver, |line| {
        bevy::log::error!("the editor's probe was asked {line}, which it doesn't answer");
        std::process::exit(1);
    });
    app.insert_non_send(EditorProbe(socket))
        .insert_resource(Probed)
        .add_plugins(ProbeInterfacePlugin)
        .add_systems(PreUpdate, read_accessibly)
        .add_systems(
            PostUpdate,
            count_clicks.after(EguiPostUpdateSet::ProcessOutput),
        )
        .add_systems(Last, answer.after(keep_pass));
}

struct EditorProbe(ProbeSocket<EditorFrameQuestion>);

// Editors never pay for the accessibility tree; only one the driver watches turns it on.
fn read_accessibly(mut contexts: Query<&mut EguiContext, Added<PrimaryEguiContext>>) {
    for mut context in &mut contexts {
        context.get_mut().enable_accesskit();
    }
}

fn count_clicks(
    mut contexts: Query<&mut EguiContext, With<PrimaryEguiContext>>,
    mut seen: ResMut<ProbeInputSeen>,
) {
    if let Ok(mut context) = contexts.single_mut() {
        probe::count_click_run(context.get_mut(), &mut seen);
    }
}

fn answer(world: &mut World) {
    let asked = world.non_send::<EditorProbe>().0.asked();
    if asked.is_empty() {
        return;
    }
    let answers = world.non_send::<EditorProbe>().0.answers();
    let Some(ctx) = primary_context(world) else {
        return;
    };
    for question in asked {
        let answer = match question {
            EditorFrameQuestion::Snapshot => to_json(&snapshot(world, &ctx)),
            EditorFrameQuestion::Frames { since } => {
                to_json(&world.resource::<FrameSamples>().since(since))
            }
            EditorFrameQuestion::TreeRows => to_json(&trees(&ctx)),
        };
        answers.give(&answer);
    }
}

fn snapshot(world: &mut World, ctx: &ui::egui::Context) -> EditorSnapshot {
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()
        .cloned();
    let system_cursor = world
        .query_filtered::<&CursorOptions, With<PrimaryWindow>>()
        .single(world)
        .is_ok_and(|options| options.visible);
    let latest = world.resource::<DrawnPasses>().latest().cloned();
    let editor = world.resource::<Editor>();
    let (undoable, redoable) = editor.project.as_ref().map_or((0, 0), |project| {
        let applied = project.history.applied();
        (applied, project.history.steps().len() - applied)
    });
    let selected = if editor.selection.parts.is_empty() {
        editor.selection.keys.clone()
    } else {
        editor.selection.parts.clone()
    };
    let busy = editor.busy.is_busy();
    let shown = shown::pass(ctx);
    let ui = world
        .resource::<DrawnUi>()
        .tree()
        .map(|tree| probe::interface_elements(ctx, tree, |_| false))
        .unwrap_or_default();
    EditorSnapshot {
        frame: world.resource::<FrameCount>().0,
        input: ProbeInput {
            cursor: window
                .as_ref()
                .and_then(Window::cursor_position)
                .map(|at| Pos::new(at.x, at.y)),
            system_cursor,
            click_run_open: probe::click_run_still_open(ctx),
            seen: *world.resource::<ProbeInputSeen>(),
        },
        screen: window
            .as_ref()
            .map(|window| Size::new(window.width(), window.height())),
        adapter: world
            .get_resource::<RenderAdapterInfo>()
            .map(|adapter| adapter.name.clone()),
        ui,
        covered: probe::interface_covers(ctx),
        menu_open: ui::egui::Popup::is_any_open(ctx),
        panels: shown
            .panels
            .iter()
            .map(|&(name, rect)| EditorPanel {
                name: name.to_owned(),
                rect: gui::window_rect(rect),
            })
            .collect(),
        tab: latest.as_ref().and_then(|pass| pass.tab.clone()),
        stage: shown.stage.map(|stage| EditorStage {
            viewport: gui::window_rect(stage.rect),
            view_proj: stage.view_proj,
            target: stage.camera.target,
            yaw: stage.camera.yaw,
            pitch: stage.camera.pitch,
            distance: stage.camera.distance,
            farthest: Meters(stage.farthest),
            marks: stage
                .marks
                .into_iter()
                .map(|(row, at)| EditorStageMark {
                    row,
                    at: Pos::new(at.x, at.y),
                })
                .collect(),
            picked: stage.picked,
        }),
        scenery: latest
            .as_ref()
            .map_or(EditorScenery::None, |pass| pass.scenery),
        sceneries_standing: latest.as_ref().map_or(0, |pass| pass.sceneries_standing),
        on_its_way: latest.as_ref().map_or_default(|pass| pass.on_its_way),
        busy,
        selected,
        undoable,
        redoable,
    }
}

fn trees(ctx: &ui::egui::Context) -> EditorTrees {
    EditorTrees {
        trees: shown::pass(ctx)
            .trees
            .into_iter()
            .map(|tree| EditorTree {
                name: tree.name.to_owned(),
                rect: gui::window_rect(tree.rect),
                rows: tree
                    .rows
                    .into_iter()
                    .map(|row| EditorTreeRow {
                        id: row.id,
                        rect: gui::window_rect(row.rect),
                        folder: row.folder,
                        open: row.open,
                    })
                    .collect(),
                selected: tree.selected,
            })
            .collect(),
    }
}

fn to_json(answer: &impl serde::Serialize) -> String {
    serde_json::to_string(answer).expect("a serializable answer")
}
