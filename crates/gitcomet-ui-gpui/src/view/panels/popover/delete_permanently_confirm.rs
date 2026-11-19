use super::*;
use crate::localization::{Language, text};

fn resolve(
    prompt_id: u64,
    confirmed: bool,
) -> impl Fn(&mut PopoverHost, &ClickEvent, &mut Window, &mut gpui::Context<PopoverHost>) + 'static
{
    move |this, _e, window, cx| {
        let _ = this.root_view.request(
            RootRequest::ResolveDeletePermanently {
                prompt_id,
                confirmed,
            },
            Some(window),
            cx,
        );
        this.close_popover_and_restore_focus(window, cx);
    }
}

/// Explorer "Delete permanently": the one file operation that skips the trash
/// and the undo journal.
pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: DeletePermanentlyPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let prompt_id = prompt.prompt_id;
    let count = prompt.names.len();
    let detail = match Language::current(cx) {
        Language::English if count == 1 => {
            "1 item will be permanently removed. This cannot be undone.".to_string()
        }
        Language::English => {
            format!("{count} items will be permanently removed. This cannot be undone.")
        }
        Language::TraditionalChinese => format!("將永久刪除 {count} 個項目。此操作無法復原。"),
    };

    ConfirmDialog::new(
        text(cx, "Delete permanently?", "要永久刪除嗎？"),
        DIALOG_440_WIDTH,
    )
    .text(theme, detail)
    .file_list(theme, &prompt.names)
    .render(
        theme,
        cancel_button_labeled(
            "delete_permanently_cancel",
            "delete_permanently_cancel_hint",
            text(cx, "Cancel", "取消"),
            theme,
        )
        .on_click(theme, cx, resolve(prompt_id, false)),
        components::Button::new(
            "delete_permanently_confirm",
            text(cx, "Delete permanently", "永久刪除"),
        )
        .style(components::ButtonStyle::Danger)
        .on_click(theme, cx, resolve(prompt_id, true))
        .debug_selector(|| "delete_permanently_confirm".to_string()),
        cx,
    )
}
