use super::*;
use crate::localization::{Language, text};

pub(super) fn model(host: &PopoverHost, repo_id: RepoId, cx: &gpui::App) -> ContextMenuModel {
    let repo = host.state.repos.iter().find(|repo| repo.id == repo_id);
    let current_scope = repo
        .map(|repo| repo.history_state.history_scope)
        .unwrap_or_default();
    let focused_refs = repo
        .filter(|repo| !repo.history_state.history_focus.is_empty())
        .map(|repo| crate::view::history_focus::history_focus_count(repo).focused);
    model_for_scope(repo_id, current_scope, focused_refs, cx)
}

fn model_for_scope(
    repo_id: RepoId,
    current_scope: gitcomet_core::domain::LogScope,
    focused_refs: Option<usize>,
    cx: &gpui::App,
) -> ContextMenuModel {
    let mut items = vec![
        ContextMenuItem::Header(text(cx, "History mode", "歷史顯示模式").into()),
        ContextMenuItem::Separator,
    ];
    items.extend(
        crate::view::history_mode::history_mode_ui_specs()
            .iter()
            .map(|spec| ContextMenuItem::Entry {
                label: spec.localized_label(cx).into(),
                icon: (spec.mode == current_scope).then_some("icons/check.svg".into()),
                shortcut: Some(spec.shortcut.into()),
                disabled: false,
                action: Box::new(ContextMenuAction::SetHistoryScope {
                    repo_id,
                    scope: spec.mode,
                }),
            }),
    );
    // Focus is set from a ref's own menu in the sidebar, but it has to be
    // clearable from the header too: the ref it was set on can be scrolled out
    // of view, collapsed, or deleted outright. The section counts the refs
    // rather than naming them; a long list of names is noise in this menu.
    if let Some(focused_refs) = focused_refs {
        let language = Language::current(cx);
        let count = crate::view::history_focus::refs_label(focused_refs, language);
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Header(
            match language {
                Language::English => format!("Focus: {count}"),
                Language::TraditionalChinese => format!("聚焦：{count}"),
            }
            .into(),
        ));
        items.push(ContextMenuItem::Entry {
            label: text(cx, "Stop focus", "取消聚焦").into(),
            icon: Some("icons/eye_off.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetHistoryFocus {
                repo_id,
                focus: gitcomet_core::domain::HistoryFocusSet::default(),
            }),
        });
    }
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn model_marks_current_history_mode(cx: &mut gpui::TestAppContext) {
        let model = cx.update(|cx| {
            super::model_for_scope(
                RepoId(11),
                gitcomet_core::domain::LogScope::MergesOnly,
                None,
                cx,
            )
        });

        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, icon, .. }
                    if label.as_ref() == "Merges only"
                        && icon
                            .as_ref()
                            .is_some_and(|icon| icon.as_ref() == "icons/check.svg")
            )
        }));
    }

    /// The focus section counts the refs instead of listing them, and offers
    /// the one way out.
    #[gpui::test]
    fn model_counts_focused_refs_without_naming_them(cx: &mut gpui::TestAppContext) {
        let model = cx.update(|cx| {
            super::model_for_scope(
                RepoId(11),
                gitcomet_core::domain::LogScope::FullReachable,
                Some(8),
                cx,
            )
        });

        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Header(text) if text.as_ref() == "Focus: 8 refs"
        )));
        assert!(
            !model
                .items
                .iter()
                .any(|item| matches!(item, ContextMenuItem::Label(_))),
            "the focused refs must not be listed by name"
        );
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Stop focus"
                    && matches!(
                        **action,
                        ContextMenuAction::SetHistoryFocus { ref focus, .. } if focus.is_empty()
                    )
        )));
    }

    #[gpui::test]
    fn model_has_no_focus_section_while_nothing_is_focused(cx: &mut gpui::TestAppContext) {
        let model = cx.update(|cx| {
            super::model_for_scope(
                RepoId(11),
                gitcomet_core::domain::LogScope::FullReachable,
                None,
                cx,
            )
        });

        assert!(!model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { label, .. } if label.as_ref() == "Stop focus"
        )));
    }

    #[gpui::test]
    fn history_mode_menu_localizes_labels_without_changing_actions(cx: &mut gpui::TestAppContext) {
        let model = cx.update(|cx| {
            cx.set_global(Language::TraditionalChinese);
            super::model_for_scope(
                RepoId(11),
                gitcomet_core::domain::LogScope::FirstParent,
                Some(2),
                cx,
            )
        });
        assert!(
            matches!(&model.items[0], ContextMenuItem::Header(label) if label.as_ref() == "歷史顯示模式")
        );
        assert!(model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { label, icon: Some(icon), shortcut: Some(shortcut), action, .. }
                if label.as_ref() == "僅沿主線"
                    && icon.as_ref() == "icons/check.svg" && shortcut.as_ref() == "P"
                    && matches!(**action, ContextMenuAction::SetHistoryScope {
                        repo_id: RepoId(11), scope: gitcomet_core::domain::LogScope::FirstParent,
                    })
        )));
        assert!(model.items.iter().any(|item| matches!(
            item, ContextMenuItem::Header(label) if label.as_ref() == "聚焦：2 個參照"
        )));
    }
}
