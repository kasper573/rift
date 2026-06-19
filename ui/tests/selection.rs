//! Selection components are controlled: the app owns the selected value (or set), the component renders
//! from it, and an item requests the next selection through the change callback. The matching content or
//! indicator mounts for the active item(s).

mod harness;

use std::collections::HashSet;

use bevy_view::view;
use harness::{State, Ui};
use ui::{
    Accordion, AccordionContent, AccordionHeader, AccordionItem, AccordionTrigger, RadioGroup,
    RadioGroupIndicator, RadioGroupItem, Tabs, TabsContent, TabsList, TabsTrigger,
};

#[test]
fn tabs_show_the_panel_for_the_active_trigger() {
    let mut ui = Ui::new();
    let value = State::new(None);
    let tree = {
        let value = value.clone();
        move || {
            let set = value.clone();
            view! {
                <Tabs value={value.get()} on_value_change={move |_w, next| set.set(next)}>
                    <TabsList>
                        <TabsTrigger value="a"><text>"A"</text></TabsTrigger>
                        <TabsTrigger value="b"><text>"B"</text></TabsTrigger>
                    </TabsList>
                    <TabsContent value="a"><text>"panel-a"</text></TabsContent>
                    <TabsContent value="b"><text>"panel-b"</text></TabsContent>
                </Tabs>
            }
        }
    };

    ui.render(tree());
    let wrapper = ui.children()[0];
    let list = ui.children_of(wrapper)[0];
    let trigger_a = ui.children_of(list)[0];
    let trigger_b = ui.children_of(list)[1];

    ui.activate_click(trigger_a);
    ui.render(tree());
    assert!(ui.texts().contains(&"panel-a".to_owned()));
    assert!(!ui.texts().contains(&"panel-b".to_owned()));

    ui.activate_click(trigger_b);
    ui.render(tree());
    assert!(ui.texts().contains(&"panel-b".to_owned()));
    assert!(!ui.texts().contains(&"panel-a".to_owned()));
}

#[test]
fn radio_group_marks_the_selected_item() {
    let mut ui = Ui::new();
    let value = State::new(None);
    let tree = {
        let value = value.clone();
        move || {
            let set = value.clone();
            view! {
                <RadioGroup value={value.get()} on_value_change={move |_w, next| set.set(next)}>
                    <RadioGroupItem value="a"><RadioGroupIndicator/></RadioGroupItem>
                    <RadioGroupItem value="b"><RadioGroupIndicator/></RadioGroupItem>
                </RadioGroup>
            }
        }
    };

    // Each item is a clickable row; the circle is its first child and holds the indicator.
    let circle = |ui: &Ui, index: usize| {
        let wrapper = ui.children()[0];
        let row = ui.children_of(wrapper)[index];
        (row, ui.children_of(row)[0])
    };

    ui.render(tree());
    let (row_a, circle_a) = circle(&ui, 0);
    assert!(
        ui.children_of(circle_a).is_empty(),
        "no indicator before selection"
    );

    ui.activate_click(row_a);
    ui.render(tree());
    let (_, circle_a) = circle(&ui, 0);
    let (row_b, circle_b) = circle(&ui, 1);
    assert_eq!(
        ui.children_of(circle_a).len(),
        1,
        "selected item shows its indicator"
    );
    assert!(ui.children_of(circle_b).is_empty());

    ui.activate_click(row_b);
    ui.render(tree());
    let (_, circle_a) = circle(&ui, 0);
    let (_, circle_b) = circle(&ui, 1);
    assert!(ui.children_of(circle_a).is_empty(), "selection moves to b");
    assert_eq!(ui.children_of(circle_b).len(), 1);
}

#[test]
fn accordion_single_keeps_one_section_open() {
    let mut ui = Ui::new();
    let value = State::new(HashSet::new());
    let tree = {
        let value = value.clone();
        move || {
            let set = value.clone();
            view! {
                <Accordion value={value.get()} on_value_change={move |_w, next| set.set(next)}>
                    <AccordionItem value="a">
                        <AccordionHeader><AccordionTrigger><text>"ta"</text></AccordionTrigger></AccordionHeader>
                        <AccordionContent><text>"ba"</text></AccordionContent>
                    </AccordionItem>
                    <AccordionItem value="b">
                        <AccordionHeader><AccordionTrigger><text>"tb"</text></AccordionTrigger></AccordionHeader>
                        <AccordionContent><text>"bb"</text></AccordionContent>
                    </AccordionItem>
                </Accordion>
            }
        }
    };

    ui.render(tree());
    let wrapper = ui.children()[0];
    let card = ui.children_of(wrapper)[0];
    let item_a = ui.children_of(card)[0];
    let item_b = ui.children_of(card)[1];
    let trigger_a = ui.children_of(ui.children_of(item_a)[0])[0];
    let trigger_b = ui.children_of(ui.children_of(item_b)[0])[0];

    // Sections collapse by height (the body stays mounted), so assert on the selection itself.
    ui.activate_click(trigger_a);
    ui.render(tree());
    assert!(value.get().contains("a"));

    ui.activate_click(trigger_b);
    ui.render(tree());
    assert!(value.get().contains("b"));
    assert!(
        !value.get().contains("a"),
        "single mode closes the previous section"
    );
}
