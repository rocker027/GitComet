# Worktree Finish

Worktree Finish lets you integrate one or more feature/issue worktrees into a single local target branch from RevWeave, with explicit preview, conflict handling (manual or via Orca CLI), Git state verification, and conservative batch-end folder cleanup.

## Entry points

- **Worktree row context menu**: **Finish into…** (`收尾到…`) pre-selects that worktree.
- **Worktrees section menu**: **Finish worktrees…** (`收尾工作樹…`) opens the batch setup dialog with all worktrees listed.
- **Command palette**: `Finish Worktrees…` (`finish-worktrees` / `收尾工作樹…`).
- **Sidebar branch drag-and-drop**: dropping a local branch checked out in another worktree onto another local branch offers cross-worktree **Finish … into …** and **Rebase … onto …** entries.

## Semantics

| Mode | Squash unchecked (default) | Squash checked |
| --- | --- | --- |
| **Merge into** | Runs `git merge --ff --no-edit -m <message> -- <locked-source-sha>` in the target host worktree (fast-forward, merge commit, or nothing new). | Runs `git merge --squash --no-commit -- <locked-source-sha>` in the target host worktree and commits one single-parent commit through RevWeave. Git does not consider the source branch merged. |
| **Rebase onto** | Runs `git rebase --no-rebase-merges --no-update-refs --no-autosquash --no-autostash -- <locked-target-sha>` in each source worktree. Rewrites source history; the target branch **does not move** and is not merged into yet. | Rebases onto `<locked-target-sha>` first to flatten history, then compresses `<locked-target-sha>..HEAD` into a single commit on top of target via `squash_commits_with_output`. Target branch still does not move. |

Both per-source checkboxes (**Squash** and **Remove folder after finish**) default to unchecked.

## Target host & temporary worktrees

- **Merge into** runs in the open tab if the target branch is checked out there, otherwise in the linked worktree where it is checked out.
- If the target branch is not checked out anywhere, a dedicated temporary worktree is created under RevWeave's per-user state directory (`worktree-finish-temp/`) **only after explicit confirmation**. If conflicts or uncommitted changes remain, the temporary worktree is kept and its exact path is shown.
- **Rebase onto** operates directly inside each source worktree and never checks out the target branch.

## Conflicts, Orca dispatch, and Git verification

- The queue pauses at the first conflict while preserving earlier completed items.
- **Squash merge conflicts** have no `MERGE_HEAD`; RevWeave captures the conflict baseline, restores only squash-owned paths plus `SQUASH_MSG` cleanup on Skip/Abort so post-conflict staged or working-tree user files are preserved (refusing destructive abort if owned files were edited or cannot be verified), and guides continuation from the queue panel.
- **Merge and Rebase conflicts** (Squash unchecked) run `git merge --abort` / `git rebase --abort` on Skip/Abort only after the same proof: when the restore would turn a directory back into a file, or write through a path a file now occupies, everything in the way (including Git-ignored and untracked files, and empty directories) must be tracked by Git. Otherwise RevWeave does not run the abort: the index, working files, conflict state and your data stay as they are, and the item is marked **Failed** with the blocking path instead of **Skipped**.
- Users may resolve conflicts manually in a tab or dispatch a dynamically discovered Orca agent (`orca agent installed` / `orca agent launch` / `orca agent list`, resolved via `ORCA_CLI_COMMAND` or `PATH` with argument arrays and never shell concatenation) in the exact conflict site worktree (target host for Merge, source worktree for Rebase). Launches are non-reentrant while in flight, and uncertain/missing launch receipts block duplicate launches.
- **Agent completion is never treated as Git completion.** RevWeave always re-snapshots the conflict worktree and reconciles against the pre-step HEAD snapshot:
  - Conflicts cleared, operation still active → **Awaiting review** (never auto-committed on `rerere.autoUpdate`).
  - External or agent commit matching expected shape → **Adopt commit** or **Rollback** (never double-commits).
  - External abort back to pre-step HEAD → **Aborted** (requeueable, never misclassified as "nothing new").
  - Source or target HEAD moved → **Refresh preview** re-runs preview and requires a fresh confirmation with the new fingerprint before any write executes.
  - Unexpected HEAD, branch switch (even at the same SHA), or dirty state → **Paused** with cleanup cancelled for that source.

## Batch-end cleanup

- Cleanup runs only at the end of the batch, only for verified completed items where **Remove folder** was checked, and **never** passes `--force` or deletes local branches.
- Before removing any folder, RevWeave re-verifies (including inside the repository mutation gate): `common_dir` identity, not primary/bare, HEAD matches completion snapshot, branch unchanged and not detached, completely clean (no staged/unstaged/untracked changes), no in-progress Git operation, not locked, no initialized submodules, Git-ignored paths and contents match the preview SHA-256 fingerprint (keeping the folder whenever ignored files changed or cannot be reliably fingerprinted), no open RevWeave terminal in that worktree (updated live across terminal open/close), and no active or unverifiable known Orca agent session.
- Any failed check keeps the folder intact and records the exact reason. Runs live in memory only and never auto-resume or auto-clean after an application restart.

## Known limitations

- **Finish protection only applies inside the window that started the run.** A running finish blocks conflicting operations (for example renaming a protected branch, or a fetch that would move it) in the RevWeave window where it was started. Each window keeps its own run state, so the same operations from another RevWeave window are not blocked, and neither are a terminal or other Git clients. Avoid changing the repository from elsewhere until the run finishes.
