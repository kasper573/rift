//! The slider is controlled: the app owns `value`, the thumb reads it from context, and a drag requests
//! a new value clamped to `min..max` through `on_value_change`.

mod harness;

use bevy_math::{DVec2, Vec2};
use bevy_ui::{ComputedNode, UiGlobalTransform};
use bevy_view::view;
use bevy_window::{PrimaryWindow, Window};
use harness::{State, Ui};
use ui::{Slider, SliderRange, SliderThumb, SliderTrack};

#[test]
fn dragging_the_thumb_requests_a_new_value_within_bounds() {
    let mut ui = Ui::new();
    let value = State::new(10.0);
    let tree = {
        let value = value.clone();
        move || {
            let set = value.clone();
            view! {
                <Slider value={value.get()} min=0.0 max=50.0 on_value_change={move |_w, next| set.set(next)}>
                    <SliderTrack>
                        <SliderRange/>
                        <SliderThumb/>
                    </SliderTrack>
                </Slider>
            }
        }
    };

    ui.render(tree());
    let slider = ui.children()[0];
    let track = ui.children_of(slider)[0];
    let thumb = ui.children_of(track)[1];

    // The thumb follows the cursor: give the track screen bounds spanning x in 0..50, then place the
    // cursor along it. (Both are physical pixels, the same space as the real window cursor.)
    let mut computed = ComputedNode::default();
    computed.size.x = 50.0;
    ui.world()
        .entity_mut(track)
        .insert((computed, UiGlobalTransform::from_xy(25.0, 0.0)));
    let mut window = Window::default();
    window.set_physical_cursor_position(Some(DVec2::new(40.0, 0.0)));
    let window_entity = ui.world().spawn((window, PrimaryWindow)).id();

    ui.activate_drag(thumb, Vec2::new(1.0, 0.0));
    assert_eq!(value.get(), 40.0, "value follows the cursor on the track");

    // Cursor past the track end clamps to max.
    if let Some(mut window) = ui.world().entity_mut(window_entity).get_mut::<Window>() {
        window.set_physical_cursor_position(Some(DVec2::new(200.0, 0.0)));
    }
    ui.activate_drag(thumb, Vec2::new(1.0, 0.0));
    assert_eq!(value.get(), 50.0, "clamped to max");
}
