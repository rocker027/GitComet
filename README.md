# <img alt="RevWeave 圖示" src="assets/revweave.svg" width="32" /> RevWeave

**開源桌面 Git 客戶端**

RevWeave 以提交歷史、分支與差異檢視為核心，由 rocker027 維護，基於 AutoExplore Oy 開發的 [GitComet](https://github.com/Auto-Explore/GitComet)。修改內容與日期記錄於 [NOTICE](NOTICE)。

支援 macOS、Windows 與 Linux。Linux 可使用 Wayland、X11 或 WSLg。

### Download

在 [RevWeave 的 GitHub Releases](https://github.com/rocker027/GitComet/releases) 取得安裝包。每一版的 `revweave-v<版本>-source.tar.gz` 包含該版對應的原始碼與建置腳本，可在同一頁免費下載。

RevWeave 使用獨立的設定與工作階段目錄。原 GitComet 的設定仍在原本的位置。命令列與 Git diff／merge 工具繼續使用 `gitcomet` 指令。

### Requirements

Git 2.53 以上可使用完整功能。使用較舊的 Git 時，依賴新版 Git 的功能會停用。

### Features

- Full local-first desktop workflow
- Git remotes, [branch-scoped Pull](docs/pull.md), push, staging, commits
- Configurable automatic background fetch across open workspaces ([docs](docs/automatic-fetch.md))
- Worktrees, branching, and full history, without agent checkpoints and other tool refs ([docs](docs/hidden-refs.md))
- Bare repository browsing and conversion ([docs](docs/bare-repositories.md))
- Multi-repository workspaces and windows
- Inline and side-by-side diffs, line-level staging, and [Difftastic](docs/difftastic.md)
- 2-way and 3-way merge tools
- Interactive rebase, cherry-pick, blame, stashes, and submodules
- Built-in editor, embedded terminal, and custom themes
- Git LFS: file status, content downloads, locks and tracking ([docs](docs/git-lfs.md))
- git-annex: content get/drop/copy/move, special remotes, adjusted branches ([docs](docs/git-annex.md))

### Build from source

On Linux, install Clang and mold first; see [developer setup](CONTRIBUTING.md#getting-started).

```bash
cargo build -p gitcomet --features ui-gpui,gix
cargo run -p gitcomet --features ui-gpui,gix -- /path/to/repo
```

### Contributing

Developer setup, workspace layout, testing, and coverage docs live in `CONTRIBUTING.md`.

### Using as a Git difftool / mergetool

GitComet can be used as a standalone diff and merge tool invoked by `git difftool` and `git mergetool`. It supports both headless (algorithm-only) and GUI (interactive GPUI window) modes.

#### Setup / uninstall (recommended)

```bash
# Configure Git globally to use RevWeave for both difftool + mergetool
gitcomet setup

# Remove RevWeave integration safely
gitcomet uninstall
```

- Use `--local` to target only the current repository instead of global config.
- Use `--dry-run` to print the commands before applying changes.

This setup registers both headless and GUI variants with `guiDefault=auto`, so Git chooses GUI when display is available and falls back to headless otherwise.
`setup`/`uninstall` are designed to be idempotent.

<details>
<summary>Show detailed setup/uninstall behavior and manual commands</summary>

Built-in `setup` writes these Git config entries:

```bash
GITCOMET_BIN="/absolute/path/to/gitcomet"

# Headless tool: algorithm-only merge/diff for CI, scripts, and no-display environments
git config --global merge.tool gitcomet
git config --global mergetool.gitcomet.cmd \
  "'$GITCOMET_BIN' mergetool --base \"\$BASE\" --local \"\$LOCAL\" --remote \"\$REMOTE\" --merged \"\$MERGED\""
git config --global mergetool.trustExitCode true
git config --global mergetool.gitcomet.trustExitCode true
git config --global mergetool.prompt false

git config --global diff.tool gitcomet
git config --global difftool.gitcomet.cmd \
  "'$GITCOMET_BIN' difftool --local \"\$LOCAL\" --remote \"\$REMOTE\" --path \"\$MERGED\""
git config --global difftool.trustExitCode true
git config --global difftool.gitcomet.trustExitCode true
git config --global difftool.prompt false

# GUI tool: opens focused GPUI windows for interactive diff/merge
git config --global merge.guitool gitcomet-gui
git config --global mergetool.gitcomet-gui.cmd \
  "'$GITCOMET_BIN' mergetool --gui --base \"\$BASE\" --local \"\$LOCAL\" --remote \"\$REMOTE\" --merged \"\$MERGED\""
git config --global mergetool.gitcomet-gui.trustExitCode true

git config --global diff.guitool gitcomet-gui
git config --global difftool.gitcomet-gui.cmd \
  "'$GITCOMET_BIN' difftool --gui --local \"\$LOCAL\" --remote \"\$REMOTE\" --path \"\$MERGED\""
git config --global difftool.gitcomet-gui.trustExitCode true

# Auto-select GUI tool when DISPLAY is available, headless otherwise
git config --global mergetool.guiDefault auto
git config --global difftool.guiDefault auto
```

Built-in `setup` stores previous user values for shared generic keys under `gitcomet.backup.*` (when needed).  
Built-in `uninstall` restores those backups only when the key still has the setup-managed value. If the user changed a setting after setup, uninstall preserves that user-edited value and then removes GitComet-specific keys.

</details>

#### CLI modes

**Browser (default):**

```bash
gitcomet [path] [--foreground]
```

On Unix, release builds relaunch the browser in a separate session when standard input is a terminal. Debug builds stay attached by default. For a detached launch, the command returns success once its first native window opens (including a first-run dialog), or an existing browser accepts the request. Startup diagnostics remain visible. Failure before that acknowledgement returns exit code 2; startup times out after 60 seconds and stops the launched process group. If the detached copy cannot be started at all (for example, no writable socket directory), the browser runs in the foreground instead. The command does not wait for repository loading, onboarding completion, or eventual window closure.

Pass `--foreground` (or set `GITCOMET_FOREGROUND=1`) to keep a newly started browser attached for continuous diagnostics or to wait for completion in a script. A script launched from a terminal can inherit terminal input and detach too; use `gitcomet --foreground "$repo"` to wait for a browser that the script starts. Requests forwarded to an existing browser still return after it accepts the request. Finder, `open`, and launches without terminal input do not use automatic terminal detachment. Focused difftool and mergetool modes continue to wait and report their final exit status.

`GITCOMET_FOREGROUND` accepts `1`/`true` and `0`/`false` (case-insensitive). An empty or unset value uses the build default: attached with debug assertions enabled, automatic detachment otherwise. Set `GITCOMET_FOREGROUND=0` to opt into detachment in a debug build. Other values are errors; `--foreground` overrides the environment.

Products built on the library opt into terminal detachment with `AppLaunch::run_cli()` or `App::run()`. Existing `AppLaunch::run()` and `run_with_args()` calls do not automatically detach. Use `run_cli()` at a process entry point whose configuration can be recreated when its executable restarts.

CLI requests to an existing browser are forwarded before preparation hooks or extension registration. A new browser performs that preparation once in its own process. Startup diagnostics use the caller's standard error; a helper started before readiness can retain that stream after the launcher returns.

**Difftool:**

```bash
gitcomet difftool --local <path> --remote <path> [--path <display_name>] [--label-left <label>] [--label-right <label>]
```

Also reads `LOCAL`/`REMOTE` from environment as a fallback when invoked by Git.

**Mergetool:**

```bash
gitcomet mergetool --local <path> --remote <path> --merged <path> [--base <path>] [--label-local <label>] [--label-remote <label>] [--label-base <label>]
```

Also reads `LOCAL`/`REMOTE`/`MERGED`/`BASE` from environment. Base is optional for add/add conflicts.

#### Compatibility

KDiff3 and Meld invocation forms are supported (`--L1/--L2/--L3`, `-o/--output/--out`, `--base`, positional arguments), so GitComet can be a drop-in replacement.

### Commit signature verification

Verification is disabled by default because it can slow down the Git log. Enable **Settings → Git log → Verify commit signatures** to show verification badges.

GitComet shows verification badges on signed commits. Git performs the check, using `gpg` for GPG and X.509 signatures and `ssh-keygen` for SSH signatures, so badges need those programs and the signers' public keys. **Settings → Executables** shows which programs GitComet found.

To install GnuPG, trust GitHub's signing key, and configure SSH allowed signers, see the [commit signature guide](docs/commit-signatures.md).

### Difftastic

Choose **Settings → Diff → Diff Engine** to make Difftastic the default, or use the diff viewer's cog for a temporary override. Both Split and Inline layouts support it. See [Difftastic](docs/difftastic.md) for partial staging and fallbacks.

### Themes

GitComet supports built-in themes and user-provided custom themes.

Built-in themes are embedded in the RevWeave binary. Custom themes are loaded from JSON bundle files in your per-user themes directory, which RevWeave creates on startup.

The full theme guide, including file locations, schema details, example bundles, and override behavior, now lives in [THEMES.md](docs/themes.md).

### Crash logs

RevWeave 將當機紀錄寫在以下路徑：

- Linux: `$XDG_STATE_HOME/revweave/crashes/`（預設 `~/.local/state/revweave/crashes/`）
- macOS: `~/Library/Logs/revweave/crashes/`
- Windows: `%LOCALAPPDATA%\revweave\crashes`（備用 `%APPDATA%\revweave\crashes`）

發生異常結束時，RevWeave 會在下次啟動顯示復原報告，並在終端機印出記錄路徑。報告包含 App 版本、平台、錯誤資訊及精簡的堆疊追蹤。

### Prior work and ideas inspired by:

SourceTree, GitKraken, Zed, GPUI, KDiff3, Meld, Github Desktop, Git, Gix, Rust, Smol, and many more.

This project has been created with the help of AI tools, including OpenAI Codex and Claude Code. The [AI policy](AI_POLICY.md) sets how contributors use them.

### License

RevWeave 依 [GNU Affero General Public License v3.0 only](LICENSE-AGPL-3.0) 發行。

上游著作權為 Copyright (C) 2026 AutoExplore Oy，完整聲明見 [NOTICE](NOTICE)。各版的對應原始碼見 [Releases](https://github.com/rocker027/GitComet/releases)。
