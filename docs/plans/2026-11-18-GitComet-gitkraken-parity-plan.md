# GitComet 操作對標 GitKraken：差距分析與改善計畫

## 結論

GitComet 的 Git 功能面已大致齊全，部分功能比 GitKraken 更強，例如內建三方衝突解決器（GitKraken 要付費）與 git-annex。手感差距集中在三個 GitKraken 招牌互動：

1. **分支標籤拖放**：把分支拖到另一個分支上，就能合併、rebase 或推送。GitComet 的歷史圖與側欄都沒有拖放。
2. **Git 操作的 Undo/Redo**：可以復原 checkout、commit、reset、刪除分支等操作。GitComet 只有檔案管理與文字輸入的復原，命令面板裡 `undo`／`redo` 還是 TODO。
3. **「就地」的入口**：GitKraken 雙擊歷史圖上的分支就簽出、工具列有 Pop、左欄有 Tags 區段，GitComet 都沒有。這些操作的後端多半已存在，只缺入口。

P1 有一部分是 GitComet 自己的便利性提案，GitKraken 文件沒有對應入口，例如 commit 右鍵「改訊息」與「Undo commit」。這類項目在表中標為「便利性提案」，不算對標缺口。

建議先做「沿用既有後端、只補入口」的批次（P1），同時做側欄拖放（G1）。Git 操作 Undo/Redo（G3）是高風險邊界（L3），要先決定範圍再動工。

本文件只有分析與計畫，沒有修改程式碼。實機操作比對為 **NOT RUN**，沒有啟動 GitComet 或 GitKraken。

## 目標與方案

- **目標**：讓熟悉 GitKraken 的使用者，可以用相同的手勢與入口完成常用 Git 操作。範圍是互動模型，不搬 GitKraken 的雲端或付費功能。
- **GitKraken 基準來源**：官方說明中心 `https://help.gitkraken.com/gitkraken-desktop/`，以及其 `llms-full.txt`（2026-04-16 產生）。頁面標示的最後驗證日期是 2026-03／04，release notes 看到 11.10.0。下列項目官方文件沒有完整列出，標為 **UNVERIFIED**：
  - 拖放選單的完整清單
  - commit 列右鍵選單的完整清單
  - 遠端分支雙擊的行為
- **GitComet 現況來源**：原始碼盤點。路徑以 `crates/gitcomet-ui-gpui/src/view/` 為根，下文簡寫為 `view/`。每個「沒有」的判定都經過 grep 確認。主要缺項已由 owner 再抽查一次。

## 範圍與版本

| 項目 | 值 |
| --- | --- |
| repo | `GitComet`（本機） |
| 分支 | `refactor/gitkraken`，與 `dev` 無差異 |
| base commit | `c90b586f` |
| 工作區 | 乾淨，未提交差異只有本文件 |
| 本次分級 | 唯讀分析與計畫。不改行為，不建 SDLC 紀錄。各改善項目的實作分級列在下方 |

## 現況對照

| 領域 | GitKraken | GitComet 現況 | 差距 |
| --- | --- | --- | --- |
| 分支拖放 | 拖到分支上出現 Merge／Rebase／Interactive Rebase／Start a PR。拖到遠端分支上就推送。也有拖放觸發 reset | 沒有。只有 repo 分頁、互動式 rebase 列、檔案總管有拖放。歷史圖與側欄的分支列沒有（grep `on_drag\|on_drop\|drag_over\|DragMove` 無命中） | **大** |
| Undo/Redo | 工具列按鈕加上 `Cmd-Z`／`Cmd-Shift-Z`。可復原 checkout、commit、discard、刪除分支、移除 remote、reset、rebase 類操作 | 沒有 Git 層級的復原。`view/command_palette.rs:866-867` 是 TODO。只能從 reflog 手動 reset | **大** |
| 工作區列（WIP） | 圖頂的 `//WIP` 節點，點擊後切到暫存面板 | 圖頂有「Uncommitted changes」列，點擊後詳細面板顯示 Unstaged／Staged 與 commit 表單（`view/rows/history/mod.rs:1090-1318`） | 小：已對齊。差在沒有右鍵選單 |
| 雙擊 | 雙擊分支標籤就簽出，雙擊 tag 跳到 commit，雙擊 ghost branch 也會簽出 | 側欄本機分支雙擊會簽出（`view/rows/sidebar/branch_rows/branch.rs:326-349`）。**歷史圖完全沒有雙擊**。遠端分支雙擊一律跳建立對話框 | 中 |
| commit 右鍵 | 文件列出的項目：建立分支、tag、annotated tag、cherry-pick、revert、reset、squash、rebase onto、checkout、patch、與工作區比較。改訊息、Drop、上移／下移只確認存在於互動式 rebase 與 cherry-pick 內，右鍵入口 UNVERIFIED。復原 commit 靠工具列 Undo | 文件列出的項目都已具備（`view/panels/popover/context_menu/commit.rs`）。改訊息在詳細面板的鉛筆按鈕（`view/panels/layout/commit_reword.rs:392`）。Drop 與上移／下移在互動式 rebase 編輯器內，與 GitKraken 相同 | 小：已對齊。復原 commit 的缺口併入 G3 |
| 圖上的 stash 節點 | 右鍵可 Apply／Pop／Delete／Edit message | 右鍵是一般 commit 選單，沒有 stash 操作 | 中 |
| 分支右鍵 | Checkout、Rename、Delete、Merge、Push、Interactive Rebase、Hide、Solo、Pin to Left、Start a PR | 有 Checkout、Rename、Delete、Merge／Squash／Rebase（限目前分支）、Focus（等同 Solo）、Pin、upstream。**沒有 Push、Hide、Create tag** | 中 |
| 側欄區段 | Local、Remote、Worktrees、Stashes、PR、Issues、Tags、Submodules，可篩選，有 Hide／Solo 眼睛圖示 | 有 Pinned、Local、Remote、Worktrees、Submodules、Stashes，可篩選，Focus 時才出現眼睛圖示。**沒有 Tags 區段**，沒有單一分支 Hide（只能隱藏整個 remote） | 中 |
| 工具列 | Undo、Redo、Pull▾、Push、Branch、Stash、Pop | 有 Pull▾（模式與 GitKraken 對等）、Push▾、Branch、Stash、Terminal、merge／rebase 進行中提示列。**沒有 Undo、Redo、Pop** | 中 |
| Stash 建立 | WIP 節點欄位可取名（選填），可對單檔或多檔 stash | 對話框**必須**填訊息，一律含 untracked 檔，沒有 keep-index，也不能只 stash 選取的檔案 | 中 |
| commit 表單 | Summary 與 Description 兩欄。按鈕標示「Commit Changes to N Files」。有 Stage all and commit | 單一多行欄位。按鈕標籤固定。沒有 stage all and commit，`Cmd-Shift-Enter` 是 Commit & Push。`CommitOptionsMenu` 已定義但沒有任何入口 | 中 |
| 多選 commit | squash、cherry-pick N、rebase N onto、建立 patch、N 個 commit 合併 diff | 有 Shift／Cmd 點選、squash、cherry-pick N。patch 只能單筆。比較要分兩步（Mark for comparison） | 小 |
| 互動式 rebase | Pick／Reword／Squash／Drop，P／S／R／D 單鍵，有 Reset | 有 Pick／Reword／Squash／Fixup／Drop、拖放排序、▲▼。`interactive_rebase.rs` 內沒有按鍵處理，單鍵是否缺少**待確認** | 小 |
| 命令面板 | `Cmd-P`，含 Checkout、Search Commits、Undo／Redo、Tag、Stash 等 | `Cmd-P`，89 個指令。10 個 TODO：search-commits、checkout-remote-branch、delete-remote-branch、merge、delete-tag、remove-remote、edit-remote-url、remove-submodule、remove-worktree、discard-all（`view/command_palette.rs:877-886`） | 小 |
| 衝突處理 | 內建 Merge Tool（付費），Take current／incoming | 內建三方解決器加上 Ours／Theirs，也可用外部工具 | GitComet 較強 |
| PR／Issue／CI | 左欄有 PR、Issues，拖放可 Start a PR | 沒有 | 列為範圍外，見下方 |

### 已對齊或較強，改動時不可退化

- 命令面板 `Cmd-P`、歷史圖搜尋 `Cmd-F`、`Cmd-/` 快捷鍵表。
- Pull 下拉模式：fast-forward if possible、fast-forward only、rebase、Fetch all，與 GitKraken 對等。
- Hunk／行暫存（右鍵、`S`／`U` 鍵、gutter）、Split／Inline 檢視、Blame、檔案歷史。
- Amend、Commit & Push、annotated tag、reflog 檢視、worktree、submodule、LFS、annex、workspace、repo 分頁拖放排序、作者篩選、歷史模式（first-parent 等）。
- Focus 等同 GitKraken 的 Solo。Pin 的語意不同：GitComet 釘在側欄頂部，GitKraken 的 Pin to Left 是把分支線固定在圖的左側。

## 改善項目

分級依 harness「工作流程」：L2 是一般風險的標準任務，L3 是命中資料遺失、不可逆或持久化格式等高風險邊界。工作量是相對估計：S 為小、M 為中、L 為大。

### P0：定義 GitKraken 手感

| ID | 項目 | 可觀察結果 | 主要觸點 | 分級／量 |
| --- | --- | --- | --- | --- |
| G1 | 側欄分支拖放 | 把側欄分支 A 拖到分支 B 上，出現選單，選了之後沿用既有確認對話框。A 拖到目前分支會出現「Merge A into current」「Squash A into current」。目前分支拖到 B 會出現「Rebase current onto B」「Interactive rebase current onto B」。本機分支拖到遠端分支會出現「Push A to remote/B」，這一項是條件性項目：後端已有 `push_refs_with_lease_with_output`（`gitcomet-core/src/services.rs:702`，git-gix 已實作），但 UI 沒有接，語意待確認。若不能沿用，push 拆到下一輪，M 的估計不含它 | `view/rows/sidebar/branch_rows/branch.rs`（`on_drag`／`on_drop`／拖曳預覽）、新增 `context_menu/branch_drop.rs`、重用 `MergeCommitConfirm`、`RebaseOntoConfirm`、互動式 rebase 設定、push prompt。測試放 `popover/tests/branch.rs`。另需更新 `docs/` | L2／M |
| G2 | 歷史圖 ref 標籤拖放與雙擊 | 圖上的標籤也能拖放，選單同 G1。雙擊只有一個本機分支的列時簽出該分支。多個分支時開 ref 選單。雙擊工作區列時聚焦 commit 訊息欄 | 標籤是 canvas 繪製，沒有自己的事件（`view/rows/history_canvas.rs:2705-2717`、`view/history_refs_hover.rs`），要補 hit-test。另涉及 `view/rows/history/mod.rs` 點擊處理。依賴 G1 的選單模型 | L2／L |
| G3 | Git 操作 Undo/Redo | 工具列有 Undo／Redo 按鈕，非文字輸入焦點時 `Cmd-Z`／`Cmd-Shift-Z` 生效。分三階段：(a) commit、checkout、reset、建立／刪除分支這類只動 ref 的操作；(b) discard，需要在丟棄前把內容寫成 blob；(c) rebase、cherry-pick、merge。merge／rebase 進行中時停用。工作區有變更時，先詢問是否 stash | `gitcomet-state` 新增操作日誌 owner、`Msg`／`Effect`。ref 還原可評估沿用既有分支交易 `apply_branch_updates_with_output`／`recover_branch_updates_with_output`（`gitcomet-core/src/services.rs:686-701`，git-gix 實作在 `repo/history_transaction.rs`）。這組交易會比對原始值與預期值，適合當 CAS 基礎。另涉及 action bar、`command_palette.rs:866-867`、`app/bindings.rs` 的 context 範圍，以及文件 | **L3**／L |

### P1：補齊入口（後端多半已有）

分級規則全表一致：只要項目含有不可逆操作（刪除、丟棄、drop），即使沿用既有確認對話框，也標 L3。

「依據」欄說明項目的來源：「對標」表示 GitKraken 官方文件有對應入口，「便利性提案」表示 GitKraken 文件沒有對應入口或觸發位置 UNVERIFIED。

| ID | 項目 | 依據 | 可觀察結果 | 主要觸點 | 分級／量 |
| --- | --- | --- | --- | --- | --- |
| Q1 | commit 右鍵「Edit commit message…」 | 便利性提案。GitKraken 只確認 Reword 在互動式 rebase 內 | 規則與鉛筆按鈕相同。不符合規則時項目反灰，並以 tooltip 說明原因 | `context_menu/commit.rs`、`panels/layout/commit_reword.rs:19-64` | L2／S |
| Q2 | HEAD 右鍵「Undo commit」 | 便利性提案。GitKraken 用工具列 Undo 復原 commit。若 G3 先完成，本項可省略 | 執行 `reset --soft HEAD~1`，訊息回填到 commit 表單。已推送的 commit 要先警告 | `context_menu/commit.rs`、`reset_prompt.rs`、`panes/details.rs` 訊息回填 | L2／S |
| Q3 | 圖上 stash 節點右鍵 Apply／Pop／Drop | 對標（`stashing/`） | stash 節點改用 stash 選單 | `context_menu/commit.rs`、`context_menu/stash.rs`、`panes/history/cache_build.rs:175-195` | **L3**（含 Drop）／S |
| Q4 | 工具列 Pop 按鈕 | 對標（`interface/`） | Stash 按鈕旁加 Pop，沒有 stash 時停用 | `panels/action_bar.rs`、bar 預設位置 `gitcomet-extension-api/src/bars.rs` | L2／S |
| Q5 | Stash 對話框改版 | 對標（`stashing/`：名稱選填、Stash file） | 訊息改選填、加入「含 untracked」「keep index」開關，檔案右鍵加「Stash N files」 | `stash_prompt.rs`、`host/repository_prompts.rs:388-396`。`gitcomet-core/src/services.rs:1420` 的 `stash_create` 要加 pathspec 與選項，git-gix 也要實作 | L2／M |
| Q6 | commit 表單資訊 | 對標（`commits/`、`keyboard-shortcuts/`） | 按鈕顯示檔案數與目標分支。新增「Stage all and commit」指令，沒有已暫存檔時由主按鈕觸發。接上或移除沒有入口的 `CommitOptionsMenu` | `panels/layout/commit_form.rs:35-60,81-85`、`command_palette.rs` | L2／S |
| Q7a | 側欄 Tags 區段：瀏覽 | 對標（`tags/`） | 可篩選，雙擊跳到 commit。右鍵可 push、從這裡建立分支 | `branch_sidebar.rs` 新增 row 變體、`rows/sidebar/branch_rows/`、`context_menu/tag.rs`、`panes/sidebar/search.rs` | L2／M |
| Q7b | 側欄 Tags 區段：刪除 | 對標（`tags/`） | 右鍵刪除本機 tag 或遠端 tag，沿用既有確認 | `context_menu/tag.rs:170,194` | **L3**／S |
| Q8 | 遠端分支雙擊直接簽出 | 便利性提案。GitKraken help 文件未記載，只有 Learn 頁面提到 | 已有追蹤分支時直接簽出。沒有時以同名建立並簽出，不跳對話框，名稱衝突才詢問 | `rows/sidebar/branch_rows/branch.rs:351-365`、`checkout_remote_branch_prompt.rs`、`branch_exists_prompt.rs` | L2／S |
| Q9a | 命令面板 TODO：非破壞性 | 對標（`command-palette/`） | search-commits、checkout-remote-branch、merge、edit-remote-url 接上既有 popover，不新增後端 | `command_palette.rs:877-886`、`commands.rs`、`gitcomet_view_commands.rs` | L2／S |
| Q9b | 命令面板 TODO：破壞性 | 部分對標（Discard all changes） | delete-remote-branch、delete-tag、remove-remote、remove-submodule、remove-worktree、discard-all 接上既有確認 | 同上 | **L3**／M |
| Q10 | 互動式 rebase 單鍵 | 對標（`interactive-rebase/`：P／S／R／D） | 選取列時按 P／R／S／F／D 切換動作 | `panes/main/interactive_rebase.rs`。先確認其他檔案沒有已實作的按鍵 | L2／S |
| Q11 | 分支右鍵補 Push 與 Create tag | Push 是對標（`pushing-and-pulling/`）。Create tag 是便利性提案，GitKraken 只確認 tag 右鍵有 Create branch here | 非目前分支也能推送，可從分支尖端建立 tag | `context_menu/branch.rs`。推送非目前分支可評估沿用 `push_refs_with_lease_with_output`，語意待確認 | L2／S–M |

### P2：較大或需要先決策

| ID | 項目 | 說明 | 分級／量 |
| --- | --- | --- | --- |
| D1 | 單一分支 Hide，並把 Focus 對齊 Solo 的說法 | 眼睛圖示常駐，隱藏狀態要存進 `repository_preferences`，等於變更持久化格式 | **L3**／M |
| D2 | commit 訊息拆成 Summary 與 Description 兩欄 | 影響草稿保存與既有使用習慣，需要決策 | L2，若改到草稿持久化則為 L3／M |
| D3 | GitKraken 鍵位預設組 | 見下方衝突表。新增設定值屬持久化變更 | L2–L3／M |
| D4 | 多選 commit 擴充 | N 筆匯出 patch、N 筆合併 diff、選兩筆直接比較、Rebase N commits onto | L2／M |
| D5 | 右鍵單步 Drop commit、上移／下移 | 便利性提案。GitKraken 只確認這些動作存在於互動式 rebase 與 cherry-pick 內，右鍵入口 UNVERIFIED。借用互動式 rebase 後端，會改寫歷史 | **L3**／M |
| D6 | 把非目前分支 reset 到指定 commit | GitKraken 的「Reset [branch] to this commit」與拖放 reset 可作為依據，但非目前分支是否適用 UNVERIFIED。等同 `branch -f`，可能讓 commit 變成孤兒 | **L3**／S–M |

## 快捷鍵衝突（待決）

GitComet 已有使用者與 `docs/shortcuts.md` 契約。建議不要直接改預設，下表每列由使用者決定「重新指定、加別名，或維持」。

「未綁定」的判斷依據有二：`docs/shortcuts.md` 全文沒有列出該鍵，`app/bindings.rs` 與 `view/` 的字面鍵位也沒有。下表只核對 macOS。Windows／Linux 的對應鍵還沒逐一核對，例如內嵌終端機會把 `Ctrl-L` 交給 shell。

| 按鍵（macOS） | GitKraken | GitComet 現況 | 建議 |
| --- | --- | --- | --- |
| `Cmd-B` | 建立分支 | 切換側欄 | 放進鍵位預設組，不改預設 |
| `Cmd-J` | 切換左欄 | 切換終端機 | 同上 |
| `Cmd-K` | 切換 commit 詳細面板 | 在瀏覽器開啟 remote | 同上 |
| `Cmd-Shift-S` | Stage all | Stash | 同上 |
| `Cmd-Shift-Enter` | Stage all and commit | Commit & Push | 同上 |
| `Cmd-Z`／`Cmd-Shift-Z`／`Cmd-Y` | Git 操作 Undo／Redo | 只用於文字輸入與檔案總管 | 隨 G3 以 context 區分，非文字焦點時給 Git 操作 |
| `Cmd-L` | Fetch all | 未綁定 | 可直接加別名 |
| `Cmd-Shift-U` | Unstage all | 未綁定（現為 `Cmd-Opt-U`） | 可直接加別名 |
| `Cmd-1`～`9` | 切到第 N 個分頁 | 一般視窗未綁定。衝突解決器內 `Cmd-1`～`3` 用來選衝突來源 | 可新增，但衝突解決器的 context 要保留原意 |
| `Cmd-Shift-M` | 聚焦 commit 訊息欄 | 未綁定 | 可直接新增 |
| `Cmd-Opt-F` | 聚焦左欄篩選 | 未綁定 | 可直接新增。需確認 Windows 的 AltGr 衝突 |
| `Opt-T`／`Opt-O` | 開啟終端機／檔案管理器 | `Cmd-Opt-T`、`Cmd-Shift-L` | 維持。macOS 的 `Opt`+字母會在文字欄輸入特殊字元 |
| `J`／`K`／`H`／`L` | 清單上下左右移動 | 方向鍵 | 只在歷史圖與檔案清單焦點時加別名 |

## 待決問題

1. **第一批做哪些？** 建議 G1 加上 P1 中的對標非破壞性項目：Q4、Q6、Q9a、Q10。便利性提案（Q1、Q2、Q8）是否納入，由使用者決定。G3 先做設計審查再動工。
2. **快捷鍵策略**：直接改預設、新增「GitKraken 鍵位」預設組，或只加不衝突的別名。建議預設組加別名。
3. **拖放的目標不是目前分支時怎麼辦？** GitComet 後端的合併與 rebase 都以目前分支為準。可選：只允許和目前分支有關的組合，其餘反灰說明；自動先 checkout；或用暫時 worktree。建議第一種。這部分 GitKraken 的實際行為 UNVERIFIED。
4. **Undo 的範圍與壽命**：只在本次執行期間有效，還是跨重啟保存？是否涵蓋 push？GitKraken 文件沒有列出 push，可視為不支援。建議只在 session 內有效，ref 類操作先行，push 不納入。
5. **commit 訊息**：維持單欄，還是改成 Summary 與 Description 兩欄。

## 範圍外

- PR／Issue／CI 整合，以及拖放 Start a PR。需要 forge 帳號驗證，屬認證邊界（L3），也超出「操作手感」的範圍。
- Launchpad、Cloud Workspaces、GitKraken AI、Insights 等雲端或付費功能。
- git-flow 與 profile／身分切換。目前沒有相關功能（grep `git.?flow\|gitflow` 為 0 筆），若需要再另立項目。

## 狀態與證據

使用者以 `/goal 逐一進行實作` 授權依序實作。待決問題 1～3 沒有回覆，採本計畫的建議預設：只加不衝突的快捷鍵別名，拖放只允許與目前分支相關的組合。工程紀錄：SDLC hub `gitcomet` 專案的 `AISDLC-2026-1118-gitkraken-parity-batch1`（L2）。

### 進度

「互動」欄記錄驗證層級：「GPUI 互動」表示以測試環境的模擬滑鼠與按鍵走過整條路徑；「僅規則」表示只測了判斷函式，互動路徑未驗。

| ID | 狀態 | 互動 | 說明 |
| --- | --- | --- | --- |
| Q4 | 完成 | GPUI 互動 | Stash 旁加 Pop，彈出 `stash@{0}`。放在既有 Stash 工具列項目內，不改 extension API。審查 W3 修正：操作進行中停用、忽略連點 |
| Q10 | 完成 | GPUI 互動 | P／R／S／D 單鍵，選單同步顯示字母。依 GitKraken 不含 F（fixup）。審查 W2 修正：按鍵作用於剛點的列，不等 store 發布選取 |
| Q6 | 完成（`CommitOptionsMenu` 除外） | GPUI 互動 | 按鈕顯示檔案數，tooltip 標目標分支。Q6b：沒有已暫存檔時，Commit 按鈕與 `Cmd-Enter` 改為「Stage All & Commit」，commit 請求帶 `stage_paths`，同一個背景工作先 stage 再 commit，保證順序。其他 repo 動作仍可能同時執行，與一般 commit 相同，不宣稱隔離。Commit & Push 不代為暫存。amend、merge／rebase 進行中或有衝突時不提供。auth 重試只重送 commit，不重新 stage。palette 新增 Stage All and Commit，與 Commit Changes 開同一個對話框。`CommitOptionsMenu` 未處理 |
| Q9a | 完成 | GPUI 互動（Merge、Search Commits） | Merge into Current Branch、Search Commits、Edit Remote URL。Checkout Remote Branch 併入既有 Checkout Branch（選擇器本來就列遠端分支），補完整片語關鍵字。互動測試抓到並修正一個 bug：合併選擇器原本會列出目前分支 |
| G1 | 完成 | GPUI 互動 | 側欄分支拖放選單：合併、squash、rebase、互動式 rebase、推送（只推到 upstream 或首推同名分支）。合併、squash、推送與分支右鍵、工具列一樣立即執行，rebase 先確認。計畫原寫「沿用確認對話框」，審查 W1 指出不符，已改文件說明，未新增確認 |
| 快捷鍵 | 部分完成 | GPUI 綁定測試 | 新增 `Cmd/Ctrl-L` 抓取全部、`Cmd/Ctrl-Shift-U` 全部取消暫存。`Cmd-1`～`9`、`Cmd-Shift-M`、`Cmd-Opt-F` 需要新指令，尚未做 |
| Q8 | 完成 | 僅規則 | 遠端分支雙擊：有追蹤分支就簽出，名稱可用就直接建立，名稱衝突才詢問 |
| Q11 | 部分完成 | 僅規則 | 分支右鍵加 Create tag。推送非目前分支需要新的狀態層訊息與後端呼叫，延後 |
| Q5 | 部分完成 | GPUI 互動 | 訊息改選填，加「Include untracked files」開關。keep-index 與只 stash 選取檔案需要後端擴充，延後 |
| G2 | 部分完成 | 僅規則 | 只做雙擊：雙擊只有一個本機分支的歷史列會簽出它。多分支選單、工作區列雙擊、標籤拖放未做 |
| Q1 | 完成 | GPUI 互動 | commit 右鍵「Edit commit message…」：選取該 commit、展開詳細面板，詳細資料載入後開編輯器。被規則阻擋時以 toast 說明 |
| Q2 | 部分完成 | 僅規則 | HEAD 右鍵「Undo commit…」走既有 soft reset 確認。已推送警告與訊息回填未做 |
| Q7a | 完成 | 僅規則 | 側欄 Tags 區段：依名稱排序、可篩選，點擊在歷史圖中跳到該 commit。與 git-annex 區段相同，只在有 tag 時顯示，以免沒有 tag 的 repo 版面改變。tag 右鍵選單含不需確認的刪除，移到 Q7b（L3） |
| Q3 | 完成（L3，審查通過） | 選單模型 | 圖上 stash 節點右鍵改開 stash 選單，Drop 走既有確認 |
| Q7b | 完成（L3，審查通過） | GPUI 互動 | 側欄 tag 右鍵開 tag 選單。依使用者決定，所有刪除 tag 的入口（含既有右鍵）都先確認 |
| Q9b | 完成（L3，審查通過） | GPUI 互動＋選單模型 | 命令面板 Remove Worktree／Submodule／Remote、Delete Tag、Delete Remote Branch、Discard All Unstaged Changes，全部先經選擇器或確認。Discard All 排除子模組並顯示衝突提示 |
| 快捷鍵 `Cmd/Ctrl-1`～`9` | 完成 | 綁定測試＋GPUI | 切換 repo 分頁。衝突解決器開著時 `1`～`3` 保留給挑選來源 |
| G3 Undo/Redo | 第一階段已實作，L3 獨立審查判定阻擋，待修 | GPUI 互動＋狀態層 | 依使用者確認的範圍：本次執行期間、commit／建立分支／刪除分支／soft reset、不含 push。工具列按鈕、命令面板、`Cmd/Ctrl-Z`、`Cmd/Ctrl-Shift-Z`、`Cmd/Ctrl-Y`。ref 被移動或分支在其他 worktree 簽出時拒絕 |
| 其餘 | 未開始 | — | D1～D6、Q11 推送非目前分支、Q5 keep-index 與部分檔案 stash、G2 標籤拖放 |

### 使用者決定（2026-11-19）

1. 刪除 tag 一律先確認，包含既有右鍵入口。
2. Undo/Redo 第一階段範圍照建議：本次執行期間、commit／建立與刪除分支／soft reset、不含 push。
3. 下列實作者先行採用的預設維持現狀。

### 實作者先行採用的預設（已獲使用者同意）

- 快捷鍵只加不衝突的別名，不改既有預設。
- 拖放只允許與目前分支相關的組合，其餘反灰說明。
- Q1、Q2、Q8 是計畫標為「便利性提案」、留給使用者決定的項目，本輪已一併實作。
- Q5 改變既有預設：stash 訊息從必填改為選填。原本鎖定「空訊息不可 stash」的測試已改寫為新行為。
- 2026-11-19 併入 `feature/side-bar`（7b46bb17，details pane 在工作區乾淨時自動隱藏）。`refactor/gitkraken` 以 fast-forward 前進，本計畫的未提交改動套回其上。`panes/details.rs` 的訂閱 closure 衝突，兩邊都保留。另修一處語意衝突：Focus Commit Message（`Cmd-Shift-M`）會先覆蓋自動隱藏再聚焦 commit 訊息框，否則乾淨工作區按下去沒有可見效果。
- 2026-11-19 使用者要求：app 開啟時工作區乾淨就先隱藏 details pane，點了 commit 才顯示。改為自動隱藏預設啟用（`details_hide_when_clean` 初始為 true）。工作區有變更時照常顯示，因為 commit 表單在裡面。
- Q6b 改變既有預設：沒有已暫存檔時，Commit 按鈕與 `Cmd-Enter` 原本停用，現在會先暫存全部變更（含 untracked）再 commit。按鈕文字會改成「Stage All & Commit」提示這件事。
- Q8 改變既有預設：遠端分支雙擊原本一律跳對話框，現在名稱可用時直接建立並簽出。

### 獨立審查

2026-11-18 由 code-reviewer 審查（patch sha256 `85be08ce…a199d`），判定「警告」：3 項中優先、11 項低優先，無關鍵問題。W1～W3 與 L1～L8 已修正。定向複審確認修正有效，另找出 N1（工具列重繪指紋未含 `stashes_rev`，Pop 可能顯示過期狀態）並指出 L1 只修一半，兩者已再修正。複審也指出 Pop 的可用狀態原本取決於側欄是否載入 stash 清單，已改為只有確定沒有 stash 時才停用。未處理而留給使用者決定的項目：

- L9：Search Commits 關閉 diff 的情境、Q1 被阻擋時的 toast、G2／Q8／Q11／Q2 的互動路徑，仍缺 GPUI 測試。
- L10：檔案歷史的 commit 選單也會出現「Edit commit message…」與「Undo commit…」，因為兩者共用 commit 選單項目。目前保留。
- L11：分支列加上拖曳後，按住移動超過約 2px 就不算點擊。repo 分頁與檔案總管也是同樣做法，目前保留。
- 審查列為 UNVERIFIED 的觀察：歷史圖雙擊會算入被 Focus 或篩選隱藏的分支；Linux IBus 可能攔截 `Ctrl-Shift-U`。

### 證據

- GitKraken 基準來自官方 help 頁與 `llms-full.txt`。
- `cargo test -p gitcomet-ui-gpui --lib`：4275 passed、0 failed（第一批加 Q8 與快捷鍵別名）。
- `cargo clippy --locked -p gitcomet-ui-gpui --tests -- -D warnings`：通過。`cargo fmt --all --check`：通過。

### 未驗

- 實機操作（啟動 App 拖放、按鍵）為 NOT RUN。UI 行為以 GPUI 測試環境的模擬滑鼠與按鍵驗證。
- GitKraken 拖放選單與 commit 右鍵完整清單 UNVERIFIED。
- 快捷鍵表的 Windows／Linux 對應鍵未逐一核對，只經 GPUI 綁定測試。
