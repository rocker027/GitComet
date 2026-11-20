//! B1: the checked merge / rebase aborts must not restore a commit over data Git does not
//! track. `git merge --abort` and `git rebase --abort` treat ignored files as expendable, so a
//! directory standing where the restored commit has a file would be deleted with its contents.

use super::worktree_finish::{add_branch_worktree, head_id, init_repo};
use crate::git_cli::git_stdout;
use crate::git_cli::run_git;
use gitcomet_core::services::{CancellationToken, CommandOutput, GitBackend, Result};
use gitcomet_core::worktree_finish::FinishOperation;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};

const SLOT_CONTENTS: &str = "same contents for rename detection\n";

/// Data written after the conflict paused that the restore would destroy.
enum Blocker {
    File(&'static str),
    EmptyDir(&'static str),
}

/// One blocker per rule: an ignored and an untracked file and an empty directory inside the
/// directory that becomes a file again, an ignored file on a path the restore writes, and an
/// ignored file where the restore needs a directory.
const BLOCKERS: [Blocker; 5] = [
    Blocker::File("slot/local.ignored"),
    Blocker::File("slot/notes.txt"),
    Blocker::EmptyDir("slot/empty"),
    Blocker::File("cache.ignored"),
    Blocker::File("data.ignored"),
];

impl Blocker {
    fn name(&self) -> &'static str {
        match self {
            Blocker::File(name) | Blocker::EmptyDir(name) => name,
        }
    }

    fn create(&self, site: &Path) {
        match self {
            Blocker::File(name) => fs::write(site.join(name), self.contents()).expect("write"),
            Blocker::EmptyDir(name) => fs::create_dir(site.join(name)).expect("mkdir"),
        }
    }

    fn contents(&self) -> String {
        format!("{} written after the pause\n", self.name())
    }

    fn assert_intact(&self, site: &Path) {
        match self {
            Blocker::File(name) => assert_eq!(
                fs::read_to_string(site.join(name)).expect("post-pause data preserved"),
                self.contents()
            ),
            Blocker::EmptyDir(name) => assert!(site.join(name).is_dir(), "{name} must remain"),
        }
    }

    fn remove(&self, site: &Path) {
        match self {
            Blocker::File(name) => fs::remove_file(site.join(name)).expect("remove file"),
            Blocker::EmptyDir(name) => fs::remove_dir(site.join(name)).expect("remove dir"),
        }
    }
}

/// `main` and a linked `feat` worktree that both track `slot`, `cache.ignored` and
/// `data.ignored/file.txt` (the last two force-added past `.gitignore`).
fn init_fixture(root: &Path) -> (PathBuf, PathBuf) {
    let main_repo = init_repo(root);
    fs::write(main_repo.join(".gitignore"), "*.ignored\n").expect("write gitignore");
    fs::write(main_repo.join("slot"), SLOT_CONTENTS).expect("write slot");
    fs::write(main_repo.join("cache.ignored"), "tracked cache\n").expect("write cache");
    fs::create_dir(main_repo.join("data.ignored")).expect("mkdir data");
    fs::write(main_repo.join("data.ignored/file.txt"), "tracked data\n").expect("write data");
    run_git(&main_repo, &["add", "-f", "."]);
    run_git(&main_repo, &["commit", "-m", "track restore targets"]);
    let feat_wt = root.join("wt-feat");
    add_branch_worktree(&main_repo, &feat_wt, "feat/restore");
    (main_repo, feat_wt)
}

/// Turns `slot` into `slot/owned.txt` and deletes the two force-added paths, so restoring the
/// other side has to write all three again.
fn commit_restructure(worktree: &Path) {
    fs::remove_file(worktree.join("slot")).expect("remove slot");
    fs::create_dir(worktree.join("slot")).expect("mkdir slot");
    fs::write(worktree.join("slot/owned.txt"), SLOT_CONTENTS).expect("write slot/owned.txt");
    fs::remove_file(worktree.join("cache.ignored")).expect("remove cache");
    fs::remove_dir_all(worktree.join("data.ignored")).expect("remove data");
    commit_base_edit(worktree, "restructured side\n");
}

fn commit_base_edit(worktree: &Path, contents: &str) {
    fs::write(worktree.join("base.txt"), contents).expect("edit base");
    run_git(worktree, &["add", "-A"]);
    run_git(worktree, &["commit", "-m", "edit base"]);
}

fn git_path_exists(site: &Path, name: &str) -> bool {
    let git_path = PathBuf::from(git_stdout(site, &["rev-parse", "--git-path", name]));
    if git_path.is_absolute() {
        git_path.exists()
    } else {
        site.join(git_path).exists()
    }
}

fn site_state(site: &Path, operation_marker: &str) -> (String, String, String, bool) {
    (
        git_stdout(
            site,
            &[
                "status",
                "--porcelain=v2",
                "--ignored",
                "--untracked-files=all",
            ],
        ),
        git_stdout(site, &["ls-files", "--stage"]),
        git_stdout(site, &["rev-parse", "HEAD"]),
        git_path_exists(site, operation_marker),
    )
}

fn assert_abort_refuses_each_blocker_then_restores(
    site: &Path,
    operation_marker: &str,
    abort: impl Fn() -> Result<CommandOutput>,
) {
    assert!(site.join("slot").is_dir(), "slot must be a directory here");
    assert!(git_path_exists(site, operation_marker));

    for blocker in &BLOCKERS {
        let name = blocker.name();
        blocker.create(site);
        let before = site_state(site, operation_marker);
        let err = abort().expect_err("the restore must not destroy data Git does not track");
        assert!(
            err.to_string().contains(name),
            "refusal must name the blocking entry '{name}': {err}"
        );
        blocker.assert_intact(site);
        assert_eq!(
            site_state(site, operation_marker),
            before,
            "refused abort must leave index, working tree, HEAD and operation state untouched ({name})"
        );
        assert!(
            before.0.contains("u UU"),
            "conflict must remain: {}",
            before.0
        );
        blocker.remove(site);
    }

    // Positive control: with only tracked entries in the way, the abort restores everything.
    abort().expect("provable restore");
    assert!(!git_path_exists(site, operation_marker));
    assert_eq!(
        fs::read_to_string(site.join("slot")).expect("slot restored as a file"),
        SLOT_CONTENTS
    );
    assert_eq!(
        fs::read_to_string(site.join("cache.ignored")).expect("cache restored"),
        "tracked cache\n"
    );
    assert_eq!(
        fs::read_to_string(site.join("data.ignored/file.txt")).expect("data restored"),
        "tracked data\n"
    );
    assert_eq!(git_stdout(site, &["status", "--porcelain"]), "");
}

#[test]
fn b1_abort_merge_checked_refuses_restores_over_untracked_or_ignored_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (main_repo, feat_wt) = init_fixture(dir.path());
    commit_base_edit(&main_repo, "main change\n");
    let main_head = head_id(&main_repo);
    commit_restructure(&feat_wt);
    let feat_head = head_id(&feat_wt);

    let opened = GixBackend.open(&main_repo).expect("open main");
    let step = opened
        .merge_commit_at(&feat_head, "merge feat")
        .expect("merge conflict");
    assert!(!step.succeeded);
    let snapshot = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("conflict snapshot");
    assert!(
        snapshot.has_merge_head,
        "a paused merge must report MERGE_HEAD"
    );
    assert_eq!(snapshot.operation, FinishOperation::Merge);

    assert_abort_refuses_each_blocker_then_restores(&main_repo, "MERGE_HEAD", || {
        opened.abort_merge_checked()
    });
    assert_eq!(head_id(&main_repo), main_head);
}

#[test]
fn b1_abort_rebase_checked_refuses_restores_over_untracked_or_ignored_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (main_repo, feat_wt) = init_fixture(dir.path());
    commit_restructure(&main_repo);
    let main_head = head_id(&main_repo);
    commit_base_edit(&feat_wt, "feat change\n");
    let feat_head = head_id(&feat_wt);

    // The rebase runs in the linked worktree, whose rebase state lives in its own git dir.
    let opened = GixBackend.open(&feat_wt).expect("open feat worktree");
    let step = opened.rebase_onto_at(&main_head).expect("rebase conflict");
    assert!(!step.succeeded);
    let snapshot = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("conflict snapshot");
    assert!(!snapshot.has_merge_head);
    assert_eq!(snapshot.operation, FinishOperation::Rebase);

    assert_abort_refuses_each_blocker_then_restores(&feat_wt, "rebase-merge", || {
        opened.abort_rebase_checked()
    });
    assert_eq!(head_id(&feat_wt), feat_head);
    assert_eq!(
        git_stdout(&feat_wt, &["symbolic-ref", "--short", "HEAD"]),
        "feat/restore"
    );
}

#[test]
fn b1_abort_rebase_checked_refuses_without_a_rebase_in_progress() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (main_repo, _feat_wt) = init_fixture(dir.path());
    let before = site_state(&main_repo, "rebase-merge");

    let err = GixBackend
        .open(&main_repo)
        .expect("open main")
        .abort_rebase_checked()
        .expect_err("no recorded restore target, nothing to prove");

    assert!(err.to_string().contains("Rebase abort refused"), "{err}");
    assert_eq!(site_state(&main_repo, "rebase-merge"), before);
}
