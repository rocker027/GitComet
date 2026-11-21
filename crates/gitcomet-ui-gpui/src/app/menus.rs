//! Application menus: the macOS menu bar and recent repositories.

use super::*;
#[cfg(target_os = "macos")]
use crate::localization::Language;

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema, Action)]
#[action(namespace = app_menu)]
#[serde(deny_unknown_fields)]
pub(super) struct OpenRecentRepository {
    pub(super) storage_key: String,
}

#[cfg(target_os = "macos")]
pub(super) fn install_macos_app_menu(cx: &mut App, backend: Arc<dyn GitBackend>) {
    let recent_repo_backend = Arc::clone(&backend);
    cx.on_action(move |recent: &OpenRecentRepository, cx| {
        let path = session::path_from_storage_key(&recent.storage_key);
        let backend = Arc::clone(&recent_repo_backend);
        cx.defer(move |cx| {
            open_repository_in_existing_or_new_window(cx, backend, path);
        });
    });

    cx.on_action(|_: &ApplyPatch, cx| {
        cx.defer(prompt_apply_patch);
    });

    cx.on_action(|_: &CheckForUpdates, cx| {
        let _ = check_for_updates_in_active_or_existing_normal_window(cx);
    });

    let clone_backend = Arc::clone(&backend);
    cx.on_action(move |_: &CloneRepository, cx| {
        let backend = Arc::clone(&clone_backend);
        cx.defer(move |cx| open_clone_repository_in_existing_or_new_window(cx, backend));
    });

    let initialize_backend = Arc::clone(&backend);
    cx.on_action(move |_: &InitializeRepository, cx| {
        let backend = Arc::clone(&initialize_backend);
        cx.defer(move |cx| prompt_initialize_repository_in_existing_or_new_window(cx, backend));
    });

    refresh_macos_app_menus(cx);
    cx.observe_global::<Language>(refresh_macos_app_menus)
        .detach();
}

#[cfg(target_os = "macos")]
pub(super) fn normal_window_repository_is_bare(cx: &mut App) -> bool {
    find_normal_gitcomet_window(cx).is_some_and(|window| {
        window
            .view
            .read_with(cx, |view, _| view.active_repository_is_bare())
            .is_ok_and(|bare| bare)
    })
}

#[cfg(target_os = "macos")]
fn macos_app_menus(cx: &mut App, external_editor_configured: bool) -> Vec<Menu> {
    let ui_session = session::load();
    Language::initialize(ui_session.ui_language.as_deref(), cx);
    let bare = normal_window_repository_is_bare(cx);
    let mut menus = macos_app_menus_with_options(
        external_editor_configured,
        find_normal_gitcomet_window(cx).is_some(),
        bare,
        crate::view::extension_host::provides_update_check(cx),
        Language::current(cx),
        ui_session.recent_repos,
    );
    let extension_items = crate::view::extension_host::macos_menu_items(bare, cx);
    if !extension_items.is_empty()
        && let Some(file) = menus.get_mut(1)
    {
        // Above the Close group (separator, Close, Close Window), which stays last.
        let at = file.items.len().saturating_sub(3);
        file.items.splice(
            at..at,
            std::iter::once(MenuItem::separator()).chain(extension_items),
        );
    }
    menus
}

/// The menus as they look with a normal window open, so the tests that care
/// about the external-editor item do not have to restate that half.
#[cfg(all(test, target_os = "macos"))]
pub(super) fn macos_app_menus_with_external_editor(external_editor_configured: bool) -> Vec<Menu> {
    macos_app_menus_with_options(
        external_editor_configured,
        true,
        false,
        false,
        Language::English,
        Vec::new(),
    )
}

#[cfg(target_os = "macos")]
pub(super) fn macos_app_menus_with_options(
    external_editor_configured: bool,
    normal_window_available: bool,
    bare: bool,
    extension_update_check: bool,
    language: Language,
    recent_repos: Vec<PathBuf>,
) -> Vec<Menu> {
    let mut file_items = vec![
        MenuItem::action(language.text("New Window", "新增視窗"), NewWindow),
        MenuItem::separator(),
        MenuItem::action(
            language.text(crate::menu_labels::OPEN_REPOSITORY, "開啟儲存庫"),
            OpenRepository,
        ),
        MenuItem::action(
            language.text(crate::menu_labels::CLONE_REPOSITORY, "複製儲存庫"),
            CloneRepository,
        ),
        MenuItem::action(
            language.text(crate::menu_labels::INITIALIZE_REPOSITORY, "初始化儲存庫"),
            InitializeRepository,
        ),
        MenuItem::action(
            language.text("Switch Repository…", "切換儲存庫…"),
            SwitchRepository,
        ),
        MenuItem::action(
            language.text("Open Workspace…", "開啟工作區…"),
            OpenWorkspace,
        ),
    ];

    let recent_repo_items = recent_repo_menu_items(recent_repos);
    if !recent_repo_items.is_empty() {
        file_items.push(MenuItem::submenu(Menu {
            name: language
                .text("Recent Repositories", "最近使用的儲存庫")
                .into(),
            items: recent_repo_items,
            disabled: false,
        }));
    }
    file_items.push(MenuItem::separator());
    if external_editor_configured {
        file_items.push(MenuItem::action(
            language.text(
                crate::menu_labels::OPEN_IN_CODE_EDITOR,
                "在程式碼編輯器中開啟",
            ),
            OpenInCodeEditor,
        ));
    }

    file_items.extend([
        MenuItem::action(
            language.text(
                crate::menu_labels::OPEN_IN_FILE_EXPLORER,
                "在檔案總管中開啟",
            ),
            LocateFileInExplorer,
        ),
        MenuItem::action(
            language.text(
                crate::menu_labels::OPEN_REMOTE_IN_BROWSER,
                "在瀏覽器中開啟遠端",
            ),
            OpenRemoteInBrowser,
        ),
        MenuItem::action(
            language.text(crate::menu_labels::APPLY_PATCH, "套用修補檔…"),
            ApplyPatch,
        )
        .disabled(bare),
        MenuItem::action(
            language.text(crate::menu_labels::CHECK_FOR_UPDATES, "檢查更新"),
            CheckForUpdates,
        )
        .disabled(manual_update_check_menu_disabled(
            crate::view::manual_update_check_unavailable(extension_update_check),
            normal_window_available,
        )),
        MenuItem::separator(),
        MenuItem::action(language.text("Close", "關閉"), Close),
        MenuItem::action(language.text("Close Window", "關閉視窗"), CloseWindow),
    ]);

    let name = identity::current().display_name();
    let (hide, quit) = match language {
        Language::English => (format!("Hide {name}"), format!("Quit {name}")),
        Language::TraditionalChinese => (format!("隱藏 {name}"), format!("結束 {name}")),
    };
    vec![
        Menu {
            name: name.to_string().into(),
            items: vec![
                MenuItem::action(
                    language.text(crate::menu_labels::COMMAND_PALETTE, "命令面板"),
                    ToggleCommandPalette,
                ),
                MenuItem::action(
                    language.text(crate::menu_labels::SETTINGS, "設定…"),
                    OpenSettings,
                ),
                MenuItem::separator(),
                MenuItem::os_submenu(language.text("Services", "服務"), SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(hide, Hide),
                MenuItem::action(language.text("Hide Others", "隱藏其他項目"), HideOthers),
                MenuItem::action(language.text("Show All", "顯示全部"), ShowAll),
                MenuItem::separator(),
                MenuItem::action(quit, Quit),
            ],
            disabled: false,
        },
        Menu {
            name: language.text("File", "檔案").into(),
            items: file_items,
            disabled: false,
        },
        Menu {
            name: language.text("Edit", "編輯").into(),
            items: vec![
                MenuItem::os_action(
                    language.text("Undo", "復原"),
                    crate::kit::Undo,
                    OsAction::Undo,
                ),
                MenuItem::os_action(
                    language.text("Redo", "重做"),
                    crate::kit::Redo,
                    OsAction::Redo,
                ),
                MenuItem::separator(),
                MenuItem::os_action(language.text("Cut", "剪下"), crate::kit::Cut, OsAction::Cut),
                MenuItem::os_action(
                    language.text("Copy", "拷貝"),
                    crate::kit::Copy,
                    OsAction::Copy,
                ),
                MenuItem::os_action(
                    language.text("Paste", "貼上"),
                    crate::kit::Paste,
                    OsAction::Paste,
                ),
                MenuItem::separator(),
                MenuItem::os_action(
                    language.text("Select All", "全選"),
                    crate::kit::SelectAll,
                    OsAction::SelectAll,
                ),
            ],
            disabled: false,
        },
        Menu {
            name: language.text("View", "顯示").into(),
            items: vec![MenuItem::action(
                language.text("Reflog", "參照日誌"),
                ShowReflog,
            )],
            disabled: false,
        },
        Menu {
            // gpui_macos registers AppKit's windows menu by the exact title "Window".
            name: "Window".into(),
            items: vec![
                MenuItem::action(language.text("Minimize", "最小化"), MinimizeWindow),
                MenuItem::action(language.text("Zoom", "縮放視窗"), ZoomWindow),
                MenuItem::separator(),
                MenuItem::action(language.text("Zoom In", "放大"), IncreaseUiScale),
                MenuItem::action(language.text("Zoom Out", "縮小"), DecreaseUiScale),
                MenuItem::action(language.text("Reset Zoom", "重設縮放"), ResetUiScale),
                MenuItem::separator(),
                MenuItem::action(
                    language.text("Previous Repository", "上一個儲存庫"),
                    PreviousRepository,
                ),
                MenuItem::action(
                    language.text("Next Repository", "下一個儲存庫"),
                    NextRepository,
                ),
                MenuItem::separator(),
                MenuItem::action(
                    language.text("Toggle Full Screen", "切換全螢幕"),
                    ToggleFullScreen,
                ),
            ],
            disabled: false,
        },
    ]
}

#[cfg(target_os = "macos")]
pub(crate) fn refresh_macos_app_menus(cx: &mut App) {
    refresh_macos_app_menus_for_external_editor(
        cx,
        crate::external_editor::configured_setting().is_some(),
    );
}

#[cfg(target_os = "macos")]
pub(super) fn refresh_macos_app_menus_for_external_editor(cx: &mut App, configured: bool) {
    let menus = macos_app_menus(cx, configured);
    cx.set_menus(menus);
}

/// How many recent repositories the macOS "Recent Repositories" submenu lists.
///
/// The recents list itself holds many more, and the repository switcher lists
/// and searches every one of them. A native submenu has no search and is rebuilt
/// whenever the active repository changes, so it keeps to the newest few.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) const RECENT_REPOSITORIES_MENU_LIMIT: usize = 15;

/// The recent repositories the macOS "Recent Repositories" submenu lists:
/// the newest [`RECENT_REPOSITORIES_MENU_LIMIT`], most recent first.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn recent_repository_menu_paths(
    recent_repos: Vec<PathBuf>,
) -> impl Iterator<Item = PathBuf> {
    recent_repos
        .into_iter()
        .take(RECENT_REPOSITORIES_MENU_LIMIT)
}

#[cfg(target_os = "macos")]
fn recent_repo_menu_items(recent_repos: Vec<PathBuf>) -> Vec<MenuItem> {
    recent_repository_menu_paths(recent_repos)
        .map(|path| {
            MenuItem::action(
                recent_repository_label(&path),
                OpenRecentRepository {
                    storage_key: session::path_storage_key(&path),
                },
            )
        })
        .collect()
}

/// Labels the macOS "Recent Repositories" menu items. Other platforms only
/// reach the recents through the repository switcher, which builds its own rows.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn recent_repository_label(path: &Path) -> String {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.display().to_string();
    };
    let Some(parent) = path.parent() else {
        return name.to_string();
    };
    format!("{name} - {}", parent.display())
}
