//! B1: a non-squash Merge into / Rebase onto conflict that the user skips or aborts must not
//! run `git merge --abort` / `git rebase --abort` when restoring a tracked file would delete a
//! directory holding data Git does not track (ignored or untracked files written after the
//! pause). The item fails for manual recovery and the conflict site stays untouched.

use super::worktree_finish::{
    FakeAgentDispatcher, git_stdout, open_real_store_and_preview, setup_real_finish_fixture,
    wait_real_run,
};
use super::*;
use crate::model::{FinishMode, ItemPhase, RunPhase, SkipReason, WorktreeFinishRun};
use crate::msg::{Msg, WorktreeFinishMsg};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SLOT_CONTENTS: &str = "same contents for rename detection\n";
const IGNORED_CONTENTS: &str = "ignored data written after the pause\n";
const STATUS_ARGS: [&str; 4] = [
    "status",
    "--porcelain=v2",
    "--ignored",
    "--untracked-files=all",
];

#[derive(Clone, Copy, Debug)]
enum UserAction {
    Skip,
    Abort,
}

struct PausedDirectoryFileConflict {
    _dir: tempfile::TempDir,
    store: AppStore,
    run: WorktreeFinishRun,
    main: PathBuf,
    /// Worktree holding the conflict: the target host for Merge, the source for Rebase.
    site: PathBuf,
}

/// Both worktrees track the file `slot`; the side whose tree the conflict site checks out
/// replaces it with `slot/owned.txt`, so aborting must turn the directory back into a file.
fn pause_on_directory_file_conflict(mode: FinishMode) -> PausedDirectoryFileConflict {
    let (dir, main, src) = setup_real_finish_fixture(true);
    for worktree in [&main, &src] {
        std::fs::write(worktree.join("slot"), SLOT_CONTENTS).expect("write slot");
        run_git(worktree, &["add", "slot"]);
        run_git(worktree, &["commit", "-m", "add slot"]);
    }
    let (swapped, site) = match mode {
        FinishMode::MergeInto => (&src, main.clone()),
        FinishMode::RebaseOnto => (&main, src.clone()),
    };
    std::fs::remove_file(swapped.join("slot")).expect("remove slot");
    std::fs::create_dir(swapped.join("slot")).expect("mkdir slot");
    std::fs::write(swapped.join("slot/owned.txt"), SLOT_CONTENTS).expect("write slot/owned.txt");
    run_git(swapped, &["add", "-A"]);
    run_git(swapped, &["commit", "-m", "move slot under its old path"]);

    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        mode,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    assert!(
        site.join("slot").is_dir(),
        "{mode:?}: fixture must leave slot as a directory at the conflict site"
    );
    PausedDirectoryFileConflict {
        _dir: dir,
        store,
        run,
        main,
        site,
    }
}

fn dispatch_user_action(fixture: &PausedDirectoryFileConflict, action: UserAction) {
    let run_id = fixture.run.id;
    fixture.store.dispatch(Msg::WorktreeFinish(match action {
        UserAction::Skip => WorktreeFinishMsg::SkipItem { run_id },
        UserAction::Abort => WorktreeFinishMsg::AbortRemaining {
            run_id,
            keep_conflict_state: false,
        },
    }));
}

/// Everything a refused abort must leave byte-identical at the conflict site.
fn conflict_site_state(main: &Path, site: &Path) -> [String; 5] {
    [
        git_stdout(site, &STATUS_ARGS),
        git_stdout(site, &["ls-files", "--stage"]),
        git_stdout(site, &["rev-parse", "HEAD"]),
        git_stdout(main, &["show-ref"]),
        in_progress_operation(site),
    ]
}

fn in_progress_operation(site: &Path) -> String {
    [
        "MERGE_HEAD",
        "rebase-merge/orig-head",
        "rebase-apply/orig-head",
    ]
    .into_iter()
    .filter_map(|name| {
        let git_path = PathBuf::from(git_stdout(site, &["rev-parse", "--git-path", name]));
        let absolute = if git_path.is_absolute() {
            git_path
        } else {
            site.join(git_path)
        };
        std::fs::read_to_string(absolute)
            .ok()
            .map(|contents| format!("{name}={}", contents.trim()))
    })
    .collect::<Vec<_>>()
    .join(";")
}

fn assert_refuses_and_preserves_ignored_descendant(mode: FinishMode, action: UserAction) {
    let fixture = pause_on_directory_file_conflict(mode);
    let ignored = fixture.site.join("slot/local.ignored");
    std::fs::write(&ignored, IGNORED_CONTENTS).expect("write ignored");
    let before = conflict_site_state(&fixture.main, &fixture.site);
    assert!(
        before[0].contains("! slot/local.ignored") && before[0].contains("u UU"),
        "{mode:?}: expected the paused conflict with the ignored file: {}",
        before[0]
    );
    assert!(
        !before[4].is_empty(),
        "{mode:?}: expected an in-progress Git operation at the conflict site"
    );

    dispatch_user_action(&fixture, action);
    let done = wait_real_run(&fixture.store, |r| {
        !r.is_active() || matches!(r.sources[0].phase, ItemPhase::Failed { .. })
    });

    let ItemPhase::Failed { message, .. } = &done.sources[0].phase else {
        panic!(
            "B1 {mode:?}/{action:?}: an unprovable restore must fail for manual recovery, not report {:?}",
            done.sources[0].phase
        );
    };
    assert!(
        message.contains("slot/local.ignored"),
        "B1 {mode:?}/{action:?}: the failure must name the blocking entry: {message}"
    );
    assert_eq!(
        std::fs::read_to_string(&ignored).expect("ignored data preserved"),
        IGNORED_CONTENTS,
        "B1 {mode:?}/{action:?}"
    );
    assert!(fixture.site.join("slot").is_dir());
    assert_eq!(
        conflict_site_state(&fixture.main, &fixture.site),
        before,
        "B1 {mode:?}/{action:?}: a refused abort must leave index, working tree, conflict state and refs untouched"
    );
}

/// Positive control: the same directory/file conflict without unowned data still restores.
fn assert_restores_provable_directory_to_file(mode: FinishMode, action: UserAction) {
    let fixture = pause_on_directory_file_conflict(mode);
    let head_before_pause = match mode {
        FinishMode::MergeInto => git_stdout(&fixture.site, &["rev-parse", "HEAD"]),
        FinishMode::RebaseOnto => git_stdout(&fixture.site, &["rev-parse", "refs/heads/feat"]),
    };

    dispatch_user_action(&fixture, action);
    let done = wait_real_run(&fixture.store, |r| !r.is_active());

    assert!(
        matches!(
            done.sources[0].phase,
            ItemPhase::Skipped(SkipReason::UserSkipped | SkipReason::AbortedByUser)
        ),
        "{mode:?}/{action:?}: a provable restore must still skip: {:?}",
        done.sources[0].phase
    );
    assert!(fixture.site.join("slot").is_file());
    assert_eq!(
        std::fs::read_to_string(fixture.site.join("slot")).expect("read slot"),
        SLOT_CONTENTS
    );
    assert_eq!(git_stdout(&fixture.site, &STATUS_ARGS), "");
    assert_eq!(
        git_stdout(&fixture.site, &["rev-parse", "HEAD"]),
        head_before_pause
    );
    assert_eq!(in_progress_operation(&fixture.site), "");
}

#[test]
fn b1_skip_merge_conflict_refuses_directory_to_file_restore_over_ignored_descendant() {
    assert_refuses_and_preserves_ignored_descendant(FinishMode::MergeInto, UserAction::Skip);
}

#[test]
fn b1_abort_merge_conflict_refuses_directory_to_file_restore_over_ignored_descendant() {
    assert_refuses_and_preserves_ignored_descendant(FinishMode::MergeInto, UserAction::Abort);
}

#[test]
fn b1_skip_rebase_conflict_refuses_directory_to_file_restore_over_ignored_descendant() {
    assert_refuses_and_preserves_ignored_descendant(FinishMode::RebaseOnto, UserAction::Skip);
}

#[test]
fn b1_abort_rebase_conflict_refuses_directory_to_file_restore_over_ignored_descendant() {
    assert_refuses_and_preserves_ignored_descendant(FinishMode::RebaseOnto, UserAction::Abort);
}

#[test]
fn b1_merge_and_rebase_still_restore_a_provable_directory_to_file_conflict() {
    for mode in [FinishMode::MergeInto, FinishMode::RebaseOnto] {
        for action in [UserAction::Skip, UserAction::Abort] {
            assert_restores_provable_directory_to_file(mode, action);
        }
    }
}
