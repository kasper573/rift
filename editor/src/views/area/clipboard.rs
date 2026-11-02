use ui::egui::Ui;
use ui::{panel, theme, words};

use crate::app::Editor;
use crate::clipboard::Clipboard;

pub(super) fn clipboard_panel(editor: &Editor, ui: &mut Ui) {
    let Some(clip) = editor.clipboard.as_ref().and_then(Clipboard::scene) else {
        return;
    };
    ui.label(
        theme::muted(format!(
            "clipboard · {} row{}",
            clip.count,
            words::plural(clip.count)
        ))
        .monospace()
        .size(13.0),
    );
    ui.add_space(4.0);
    let text = clip.nodes.text();
    let shown: String = text.lines().take(40).collect::<Vec<_>>().join("\n");
    panel::code_block(ui, &shown);
    if text.lines().count() > 40 {
        ui.label(theme::faint("…").size(12.5));
    }
    ui.add_space(6.0);
}
