use crate::support::assets;

#[test]
fn shipped_content_passes_the_startup_checks() {
    game::systems::check_content(&assets());
}
