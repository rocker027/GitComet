use super::*;
use crate::localization::text;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let title = text(cx, "Pull: choose strategy", "選擇拉取策略");

    ConfirmDialog::new(title, DIALOG_440_WIDTH)
        .text(
            theme,
            text(
                cx,
                "Fast-forward isn't possible. Choose whether to merge or rebase.",
                "無法快轉。請選擇合併或變基。",
            ),
        )
        .text(
            theme,
            text(
                cx,
                "Merge combines your changes with the upstream branch.",
                "合併會整合你的變更與上游分支。",
            ),
        )
        .text(
            theme,
            text(
                cx,
                "Rebase replays your commits on the upstream branch.",
                "變基會在上游分支上重新套用你的提交。",
            ),
        )
        .render(
            theme,
            dialog_cancel_button(
                "pull_reconcile_cancel",
                "pull_reconcile_cancel_hint",
                theme,
                cx,
            ),
            div()
                .flex()
                .gap_1()
                .child(
                    components::Button::new("pull_reconcile_merge", text(cx, "Merge", "合併"))
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::Pull {
                                repo_id,
                                mode: PullMode::Merge,
                            });
                            this.close_popover(cx);
                        }),
                )
                .child(
                    components::Button::new("pull_reconcile_rebase", text(cx, "Rebase", "變基"))
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::Pull {
                                repo_id,
                                mode: PullMode::Rebase,
                            });
                            this.close_popover(cx);
                        }),
                ),
            cx,
        )
}
