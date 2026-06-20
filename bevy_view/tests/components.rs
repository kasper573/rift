mod harness;

use bevy_view::{View, node, text, view};
use harness::Ui;
#[derive(Default)]
struct Panel {
    title: String,
    body: Vec<View>,
}

impl Panel {
    fn title(mut self, title: impl Into<String>) -> Panel {
        self.title = title.into();
        self
    }

    fn child(mut self, child: impl Into<View>) -> Panel {
        self.body.push(child.into());
        self
    }
}

impl From<Panel> for View {
    fn from(panel: Panel) -> View {
        node().child(text(panel.title)).children(panel.body).into()
    }
}

#[test]
fn a_component_tag_lowers_to_its_builder() {
    let mut ui = Ui::new();
    ui.render(view! {
        <Panel title={"Inventory"}>
            <text>"slot"</text>
        </Panel>
    });
    assert_eq!(ui.texts(), vec!["Inventory".to_owned(), "slot".to_owned()]);
}

#[test]
fn a_component_composes_with_intrinsics_around_it() {
    let mut ui = Ui::new();
    ui.render(view! {
        <node>
            <Panel title={"A"}/>
            <Panel title={"B"}/>
        </node>
    });
    assert_eq!(ui.texts(), vec!["A".to_owned(), "B".to_owned()]);
}
