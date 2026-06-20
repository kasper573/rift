mod harness;

use bevy_view::view;
use harness::Ui;
use ui::{
    Avatar, AvatarFallback, Progress, ProgressIndicator, ScrollArea, ScrollAreaViewport, Separator,
};

#[test]
fn separator_renders_a_node() {
    let mut ui = Ui::new();
    ui.render(view! { <Separator/> });
    assert_eq!(ui.children().len(), 1);
}

#[test]
fn progress_renders_its_indicator() {
    let mut ui = Ui::new();
    ui.render(view! { <Progress value=40.0 max=100.0><ProgressIndicator/></Progress> });
    let bar = ui.children()[0];
    assert_eq!(
        ui.children_of(bar).len(),
        1,
        "the indicator mounts inside the bar"
    );
}

#[test]
fn avatar_shows_its_fallback() {
    let mut ui = Ui::new();
    ui.render(view! { <Avatar><AvatarFallback><text>"AB"</text></AvatarFallback></Avatar> });
    assert_eq!(ui.texts(), vec!["AB".to_owned()]);
}

#[test]
fn scroll_area_renders_its_viewport_content() {
    let mut ui = Ui::new();
    ui.render(view! {
        <ScrollArea><ScrollAreaViewport><text>"long"</text></ScrollAreaViewport></ScrollArea>
    });
    assert_eq!(ui.texts(), vec!["long".to_owned()]);
}
