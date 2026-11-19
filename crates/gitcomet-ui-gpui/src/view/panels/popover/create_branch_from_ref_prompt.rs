use super::*;
use crate::kit::interaction as controls;
use crate::localization::text;

fn checkout_toggle(
    theme: AppTheme,
    enabled: bool,
    focus_handle: &FocusHandle,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Stateful<gpui::Div> {
    super::checkbox_row(
        "create_branch_checkout_toggle",
        "create_branch_checkout_toggle",
        text(cx, "Checkout", "切換到此分支"),
        enabled,
        focus_handle,
        theme,
        cx,
    )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    target: String,
    source_selectable: bool,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let is_bare = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .is_some_and(RepoState::is_bare);
    let can_create = this.can_submit_create_branch(cx);
    let branch_placeholder = text(cx, "branch-name", "分支名稱");
    if this.create_branch_input.read(cx).placeholder() != branch_placeholder {
        this.create_branch_input.update(cx, |input, cx| {
            input.set_placeholder(branch_placeholder, cx);
        });
    }
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

    let source_row = if source_selectable {
        let search = this
            .branch_picker_search_input
            .clone()
            .expect("branch_picker_search_input must be initialized");
        let is_focused = search
            .read_with(cx, |input, _| input.focus_handle())
            .is_focused(window);
        let placeholder = text(cx, "Filter branches", "篩選分支");
        search.update(cx, |input, cx| {
            if input.placeholder() != placeholder {
                input.set_placeholder(placeholder, cx);
            }
            input.set_chromeless(is_focused, cx);
            input.set_leading_icon(is_focused.then_some("icons/git_branch.svg"), cx);
        });

        if is_focused {
            let query = search.read(cx).text().trim().to_string();
            let built = branch_picker::ref_rows_cached(
                this,
                branch_picker::RefRowsSpec::source_ref(),
                &query,
            );
            let names = std::rc::Rc::clone(&built.payloads);

            div()
                .flex()
                .flex_col()
                .child(super::popover_detail(theme, text(cx, "Source:", "來源：")))
                .child(
                    div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                        branch_picker::ref_picker_prompt(
                            search,
                            this.picker_prompt_scroll.clone(),
                            &built,
                            cx,
                        )
                        .tooltip_host(this.tooltip_host.clone())
                        .empty_text(text(cx, "No matches", "沒有相符的項目"))
                        .max_height(scaled_px(branch_picker::REF_PICKER_LIST_MAX_HEIGHT_PX))
                        .selected_index(this.branch_picker_selected_index)
                        .render(
                            theme,
                            ui_scale_percent,
                            cx,
                            move |this, ix, _e, window, cx| {
                                let Some(name) = names.get(ix).cloned() else {
                                    return;
                                };
                                let repo_id = this.active_repo_id().unwrap_or(RepoId(0));
                                this.handle_inline_branch_picker_select(name, repo_id, window, cx);
                            },
                        ),
                    ),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .child(super::popover_detail(theme, text(cx, "Source:", "來源：")))
                .child(div().px_2().pb_1().w_full().min_w(px(0.0)).child(search))
        }
    } else {
        super::popover_detail(
            theme,
            format!("{} {target}", text(cx, "Source branch:", "來源分支：")),
        )
    };

    div()
        .flex()
        .flex_col()
        .w(scaled_px(540.0))
        .child(popover_title(theme, text(cx, "Create branch", "建立分支")))
        .child(super::popover_rule(theme))
        .child(source_row)
        .child(input_label(
            theme,
            text(cx, "New branch name", "新分支名稱"),
        ))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_branch_input.clone()),
        )
        .child(
            checkout_toggle(
                theme,
                this.create_branch_checkout_enabled,
                &this.create_branch_from_ref_checkout_focus_handle,
                cx,
            )
            .on_activate(
                is_bare,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _w, cx| {
                    this.create_branch_checkout_enabled = !this.create_branch_checkout_enabled;
                    cx.notify();
                }),
            )
            .when(is_bare, |toggle| {
                toggle
                    .text_color(theme.disabled_color(theme.colors.foreground.primary))
                    .gitcomet_tooltip(
                        theme,
                        text(
                            cx,
                            gitcomet_core::domain::WORKING_TREE_REQUIRED_MESSAGE,
                            "這項操作需要工作樹。此儲存庫是裸儲存庫。",
                        )
                        .into(),
                    )
            }),
        )
        .child(super::popover_rule(theme))
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button_labeled(
                        "create_branch_from_ref_cancel",
                        "create_branch_from_ref_cancel_hint",
                        text(cx, "Cancel", "取消"),
                        theme,
                    )
                    .focus_handle(this.create_branch_from_ref_focus.cancel.clone())
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new(
                        "create_branch_from_ref_go",
                        text(cx, "Create", "建立"),
                    )
                    .focus_handle(this.create_branch_from_ref_focus.submit.clone())
                    .separated_end_slot(hotkey_hint(
                        theme,
                        "create_branch_from_ref_go_hint",
                        "Enter",
                    ))
                    .style(components::ButtonStyle::Filled)
                    .disabled(!can_create)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.submit_create_branch(window, cx);
                    }),
                ),
        )
}
