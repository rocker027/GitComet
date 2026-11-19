use crate::localization::text;
use gitcomet_core::domain::HistoryMode;
use gpui::App;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HistoryModeUiSpec {
    pub mode: HistoryMode,
    pub label: &'static str,
    traditional_chinese_label: &'static str,
    pub shortcut: &'static str,
    settings_description: &'static str,
    traditional_chinese_description: &'static str,
    pub settings_row_id: &'static str,
}

impl HistoryModeUiSpec {
    pub(crate) fn localized_label(&self, cx: &App) -> &'static str {
        text(cx, self.label, self.traditional_chinese_label)
    }

    pub(crate) fn localized_description(&self, cx: &App) -> &'static str {
        text(
            cx,
            self.settings_description,
            self.traditional_chinese_description,
        )
    }
}

const HISTORY_MODE_UI_SPECS: [HistoryModeUiSpec; 5] = [
    HistoryModeUiSpec {
        mode: HistoryMode::FullReachable,
        label: "Full reachable",
        traditional_chinese_label: "完整歷史",
        shortcut: "F",
        settings_description: "Show every commit reachable from the current branch tip.",
        traditional_chinese_description: "顯示目前分支可追溯的所有提交。",
        settings_row_id: "settings_window_git_log_default_mode_full_reachable",
    },
    HistoryModeUiSpec {
        mode: HistoryMode::FirstParent,
        label: "First-parent",
        traditional_chinese_label: "僅沿主線",
        shortcut: "P",
        settings_description: "Follow only the mainline path through merges.",
        traditional_chinese_description: "遇到合併時只沿第一個父提交回溯，保留主線的合併提交。",
        settings_row_id: "settings_window_git_log_default_mode_first_parent",
    },
    HistoryModeUiSpec {
        mode: HistoryMode::NoMerges,
        label: "No merges",
        traditional_chinese_label: "不顯示合併",
        shortcut: "N",
        settings_description: "Hide merge commits and show only ordinary commits.",
        traditional_chinese_description: "僅顯示一般提交，略過合併提交。",
        settings_row_id: "settings_window_git_log_default_mode_no_merges",
    },
    HistoryModeUiSpec {
        mode: HistoryMode::MergesOnly,
        label: "Merges only",
        traditional_chinese_label: "僅顯示合併",
        shortcut: "M",
        settings_description: "Show only merge commits for high-level integration history.",
        traditional_chinese_description: "僅顯示合併提交，方便查看分支的整合歷程。",
        settings_row_id: "settings_window_git_log_default_mode_merges_only",
    },
    HistoryModeUiSpec {
        mode: HistoryMode::AllBranches,
        label: "All branches",
        traditional_chinese_label: "所有分支",
        shortcut: "A",
        settings_description: "Show commits reachable from all refs, not just the current branch.",
        traditional_chinese_description: "顯示所有分支與其他納入的參照可追溯的提交。",
        settings_row_id: "settings_window_git_log_default_mode_all_branches",
    },
];

pub(crate) fn history_mode_ui_specs() -> &'static [HistoryModeUiSpec] {
    &HISTORY_MODE_UI_SPECS
}

pub(crate) fn localized_history_mode_label(mode: HistoryMode, cx: &App) -> &'static str {
    history_mode_ui_specs()
        .iter()
        .find(|spec| spec.mode == mode)
        .map(|spec| spec.localized_label(cx))
        .unwrap_or_else(|| text(cx, "Unknown", "未知"))
}
