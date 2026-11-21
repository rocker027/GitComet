use super::*;
use crate::localization::{Language, text};

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let lease = this.force_push_lease.clone();
    let dialog = ConfirmDialog::new(text(cx, "Force push", "強制推送"), DIALOG_420_WIDTH);
    let dialog = if let Some(lease) = lease.as_ref() {
        let message = match Language::current(cx) {
            Language::English => format!(
                "This will update {}/{} only if it still points at {} and {} is still checked out at {}.",
                lease.remote, lease.branch, lease.expected, lease.local_branch, lease.local_head
            ),
            Language::TraditionalChinese => format!(
                "只有在 {}/{} 仍指向 {}，且目前仍在分支 {} 的提交 {} 時，才會更新遠端分支。",
                lease.remote, lease.branch, lease.expected, lease.local_branch, lease.local_head
            ),
        };
        dialog.text(theme, message).command(
            theme,
            format!(
                "git push --force-with-lease=refs/heads/{}:{} {} {}:refs/heads/{}",
                lease.branch, lease.expected, lease.remote, lease.local_head, lease.branch
            ),
        )
    } else {
        dialog.text(
            theme,
            text(cx, "The upstream branch's tip has not loaded, so there is nothing to check the push against.", "尚未載入上游分支最新提交，無法核對推送條件。"),
        )
    };
    dialog.render(
        theme,
        dialog_cancel_button("force_push_cancel", "force_push_cancel_hint", theme, cx),
        components::Button::new(
            "force_push_go",
            text(cx, "Force push with lease", "強制推送（含租約檢查）"),
        )
        .style(components::ButtonStyle::Danger)
        .disabled(lease.is_none())
        .on_click(theme, cx, move |this, _e, _w, cx| {
            if let Some(lease) = lease.clone() {
                this.store
                    .dispatch(Msg::ForcePushWithLease { repo_id, lease });
                this.close_popover(cx);
            }
        }),
        cx,
    )
}
