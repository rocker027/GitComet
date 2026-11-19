use super::*;
use crate::localization::Language;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    name: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);

    ConfirmDialog::new(language.text("Remove remote", "移除遠端"), DIALOG_420_WIDTH)
        .text(
            theme,
            match language {
                Language::English => format!("Remote: {name}"),
                Language::TraditionalChinese => format!("遠端：{name}"),
            },
        )
        .render(
            theme,
            dialog_cancel_button(
                "remove_remote_cancel",
                "remove_remote_cancel_hint",
                theme,
                cx,
            ),
            components::Button::new("remove_remote_go", language.text("Remove", "移除"))
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::RemoveRemote {
                        repo_id,
                        name: name.clone(),
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
