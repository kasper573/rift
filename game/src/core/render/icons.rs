use bevy::prelude::*;

const ICON_SIZE: u32 = 128;

#[derive(Resource, Default)]
pub(super) struct ResidentIcons(Vec<Handle<Image>>);

// Icons come and go with whatever shows them, and an unloaded one shown again waits on a fresh
// load, popping in a few frames late. They are small, so once loaded they stay loaded.
pub(super) fn keep_icons_resident(
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    mut resident: ResMut<ResidentIcons>,
) {
    for event in events.read() {
        let AssetEvent::LoadedWithDependencies { id } = *event else {
            continue;
        };
        let small = images
            .get(id)
            .is_some_and(|image| image.width() <= ICON_SIZE && image.height() <= ICON_SIZE);
        if small && let Some(handle) = images.get_strong_handle(id) {
            resident.0.push(handle);
        }
    }
}
