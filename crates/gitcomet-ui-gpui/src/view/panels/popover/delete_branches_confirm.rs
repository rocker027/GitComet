use super::*;
use crate::localization::Language;
use gitcomet_state::name_summary::{LISTED_NAMES, branch_noun, elide_names, elision_suffix};

/// Confirms emptying a branch group.
///
/// The local variant offers two confirmations rather than a toggle: an unforced
/// delete refuses branches that are not fully merged, which is the normal state
/// of a finished feature group, so "Force delete" has to be reachable without
/// running the safe attempt first and reading the failure.
pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
    remote: Option<String>,
    group_label: String,
    names: Vec<String>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let count = names.len();
    let noun = branch_noun(count);

    // `group_label` is `origin/feat/` for a remote group, so naming it says both
    // which remote and which scope — "on origin" alone would read as the whole
    // remote when several groups live under it.
    let title = match (section, language) {
        (BranchSection::Remote, Language::English) => {
            format!("Delete {count} {noun} in {group_label} on the remote?")
        }
        (BranchSection::Local, Language::English) => {
            format!("Delete {count} {noun} in {group_label}?")
        }
        (BranchSection::Remote, Language::TraditionalChinese) => {
            format!("要從遠端刪除「{group_label}」中的 {count} 個分支嗎？")
        }
        (BranchSection::Local, Language::TraditionalChinese) => {
            format!("要刪除「{group_label}」中的 {count} 個本機分支嗎？")
        }
    };

    // Elided the same way the name list is. Spelling out 300 refs would wrap to
    // a hundred lines inside a 420px dialog, and a confirm dialog gets no
    // scroll wrapper — the buttons would end up off screen.
    let command = match (section, remote.as_deref()) {
        (BranchSection::Remote, Some(remote)) => {
            format!("git push --delete {remote} {}", elide_names(&names, " "))
        }
        // Both buttons are previewed, since the adjacent Force delete runs the
        // other one and a lone `-d` would read as "this cannot destroy work".
        _ => format!("git branch -d|-D {}", elide_names(&names, " ")),
    };

    let dialog = ConfirmDialog::new(title, DIALOG_420_WIDTH)
        .section(name_list(theme, &names, language))
        .text(
            theme,
            match section {
                BranchSection::Local => language.text(
                    "The current branch is never included. Branches that are not fully merged \
                         need Force delete.",
                    "目前的分支不會被刪除。尚未完全合併的分支須使用「強制刪除」。",
                ),
                BranchSection::Remote => language.text(
                    "This deletes the branches on the remote for everyone, not just here.",
                    "這會從遠端刪除這些分支，所有使用該遠端的人都會受到影響。",
                ),
            },
        )
        .command(theme, command);

    let cancel = dialog_cancel_button(
        "delete_branches_cancel",
        "delete_branches_cancel_hint",
        theme,
        cx,
    );

    match section {
        BranchSection::Local => {
            let force_names = names.clone();
            dialog.render(
                theme,
                cancel,
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        components::Button::new(
                            "delete_branches_go",
                            language.text("Delete", "刪除"),
                        )
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::DeleteBranches {
                                repo_id,
                                names: names.clone(),
                                force: false,
                            });
                            this.close_popover(cx);
                        }),
                    )
                    .child(
                        components::Button::new(
                            "delete_branches_force",
                            language.text("Force delete", "強制刪除"),
                        )
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store.dispatch(Msg::DeleteBranches {
                                repo_id,
                                names: force_names.clone(),
                                force: true,
                            });
                            this.close_popover(cx);
                        }),
                    ),
                cx,
            )
        }
        BranchSection::Remote => {
            let remote = remote.unwrap_or_default();
            dialog.render(
                theme,
                cancel,
                components::Button::new(
                    "delete_branches_go",
                    language.text("Delete on remote", "從遠端刪除"),
                )
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::DeleteRemoteBranches {
                        repo_id,
                        remote: remote.clone(),
                        branches: names.clone(),
                    });
                    this.close_popover(cx);
                }),
                cx,
            )
        }
    }
}

/// The branches about to go, capped so a large group cannot push the buttons
/// off screen. Shares its cap and its overflow wording with the command preview
/// above and with the failure toast a partial delete produces.
fn name_list(theme: AppTheme, names: &[String], language: Language) -> gpui::Div {
    let mut list = div()
        .px_2()
        .py_1()
        .flex()
        .flex_col()
        .text_size(theme.ui_text(14.0))
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_color(theme.colors.foreground.secondary);
    for name in names.iter().take(LISTED_NAMES) {
        list = list.child(
            div()
                .whitespace_nowrap()
                .overflow_hidden()
                .child(name.clone()),
        );
    }
    if let Some(suffix) = elision_suffix(names.len()) {
        let suffix = match language {
            Language::English => suffix,
            Language::TraditionalChinese => format!("…另有 {} 個分支", names.len() - LISTED_NAMES),
        };
        list = list.child(div().child(suffix));
    }
    list
}
