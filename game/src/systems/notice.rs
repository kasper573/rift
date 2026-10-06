use bevy::prelude::*;
use bevy::scene::EntityScene;
use bevy_replicon::prelude::{SendTargets, ToClients};
use serde::{Deserialize, Serialize};
use ui::tokens::{palette, typography};

use crate::core::sfx::SfxId;
use crate::core::sfx::playback::{PlaySfx, SfxPlace};
use crate::systems::player::Owner;
use crate::systems::scene::Scene as GameScene;
use crate::systems::visibility::PrivateSight;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.add_server_message::<Notice>(Channel::Ordered);
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub tone: NoticeTone,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    Info,
    Good,
    Bad,
}

pub fn tell(world: &mut World, player: Entity, text: impl Into<String>, tone: NoticeTone) {
    let Some(owner) = world.get::<Owner>(player).map(|owner| owner.client) else {
        return;
    };
    let viewers: Vec<Entity> = world
        .query::<(Entity, &PrivateSight)>()
        .iter(world)
        .filter(|(_, sight)| sight.own == owner || sight.watching == Some(owner))
        .map(|(conn, _)| conn)
        .collect();
    let notice = Notice {
        text: text.into(),
        tone,
    };
    for conn in viewers {
        world.write_message(ToClients {
            targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(conn)),
            message: notice.clone(),
        });
    }
}

pub struct ToasterPlugin;

impl Plugin for ToasterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameScene::Area), spawn_headlines)
            .add_systems(
                OnExit(GameScene::Area),
                crate::systems::scene::despawn_all::<Headlines>,
            )
            .add_systems(Update, show_notices.run_if(in_state(GameScene::Area)));
    }
}

#[derive(Component, Default, Clone)]
struct Headlines;

#[derive(Component, Default, Clone)]
struct NoticeToaster;

fn spawn_headlines(mut commands: Commands) {
    commands.spawn_scene(bsn! {
        Headlines
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px({ui::tokens::spacing::XL}),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
        }
        Pickable::IGNORE
        GlobalZIndex({ui::tokens::layer::NOTICES})
        Children [
            {EntityScene(ui::announcement_lane(ui::AnnouncementLane::default()))},
            (
                Node { width: Val::Percent(100.0), height: Val::Px(0.0) }
                Pickable::IGNORE
                Children [ ( {ui::toaster(ui::SonnerPosition::TopCenter)} NoticeToaster ) ]
            ),
        ]
    });
}

fn show_notices(
    mut notices: MessageReader<Notice>,
    toasters: Query<Entity, With<NoticeToaster>>,
    mut sounds: MessageWriter<PlaySfx>,
    mut commands: Commands,
) {
    let Ok(toaster) = toasters.single() else {
        return;
    };
    for notice in notices.read() {
        commands.spawn_scene(toast(notice)).insert(ChildOf(toaster));
        sounds.write(PlaySfx {
            id: SfxId::UiToast,
            place: SfxPlace::Interface,
        });
    }
}

fn toast(notice: &Notice) -> impl Scene {
    let ink = match notice.tone {
        NoticeTone::Info => ui::theme::theme().surface_canvas.on,
        NoticeTone::Good => palette::EMERALD_80,
        NoticeTone::Bad => palette::CRIMSON_80,
    };
    let text = ui::styled_text(notice.text.clone(), ink, typography::BODY);
    bsn! {
        {ui::toast()}
        Children [
            (
                Node { width: Val::Percent(100.0), height: Val::Percent(100.0), align_items: AlignItems::Center }
                Children [ {EntityScene(text)} ]
            )
        ]
    }
}
