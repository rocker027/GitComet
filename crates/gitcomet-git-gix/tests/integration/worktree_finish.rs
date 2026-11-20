use crate::git_cli::{git_output, git_stdout, run_git};
use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{CancellationToken, GitBackend, WorktreeCheckout};
use gitcomet_core::worktree_finish::{FinishOperation, default_merge_branch_message};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn init_repo(root: &Path) -> PathBuf {
    let repo = root.join("main");
    fs::create_dir_all(&repo).expect("create repo dir");
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Tester"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["config", "core.autocrlf", "false"]);
    run_git(&repo, &["config", "core.eol", "lf"]);
    fs::write(repo.join("base.txt"), "line1\nline2\n").expect("write base");
    run_git(&repo, &["add", "base.txt"]);
    run_git(&repo, &["commit", "-m", "initial"]);
    repo
}

pub(crate) fn head_id(path: &Path) -> CommitId {
    CommitId(git_stdout(path, &["rev-parse", "HEAD"]).into())
}

pub(crate) fn add_branch_worktree(main_repo: &Path, wt_path: &Path, branch: &str) {
    let backend = GixBackend;
    let repo = backend.open(main_repo).expect("open main");
    repo.add_worktree_with_output(
        wt_path,
        &WorktreeCheckout::NewBranch {
            name: branch.to_string(),
            start: Some("HEAD".to_string()),
        },
    )
    .expect("add branch worktree");
}

#[test]
fn t_g1_squash_merge_conflict_state_and_abort_restores_without_squash_msg() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    let feat_wt = dir.path().join("wt-feat");
    add_branch_worktree(&main_repo, &feat_wt, "feat/conflict");

    // Diverging edits to base.txt
    fs::write(main_repo.join("base.txt"), "main-line\nline2\n").expect("write main");
    run_git(&main_repo, &["add", "base.txt"]);
    run_git(&main_repo, &["commit", "-m", "main edit"]);
    let main_before = head_id(&main_repo);

    fs::write(feat_wt.join("base.txt"), "feat-line\nline2\n").expect("write feat");
    run_git(&feat_wt, &["add", "base.txt"]);
    run_git(&feat_wt, &["commit", "-m", "feat edit"]);
    let feat_head = head_id(&feat_wt);

    let backend = GixBackend;
    let opened = backend.open(&main_repo).expect("open main");
    let step = opened
        .squash_merge_at(&feat_head)
        .expect("squash merge step");
    assert!(
        !step.succeeded,
        "conflicting squash merge must exit non-zero"
    );

    let snap = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snapshot during squash conflict");
    assert!(
        !snap.has_merge_head,
        "squash merge must not create MERGE_HEAD"
    );
    assert_eq!(snap.operation, FinishOperation::SquashMergeConflict);
    assert_eq!(snap.unmerged, vec![PathBuf::from("base.txt")]);
    assert!(
        snap.conflict_marker_paths
            .contains(&PathBuf::from("base.txt"))
    );
    assert_eq!(snap.head, Some(main_before.clone()));

    // Verify git merge --abort fails for squash merge conflicts.
    let abort_out = git_output(&main_repo, &["merge", "--abort"]);
    assert!(
        !abort_out.status.success(),
        "git merge --abort should fail without MERGE_HEAD"
    );

    // Our abort_squash_merge must restore HEAD, index, and remove SQUASH_MSG.
    opened
        .abort_squash_merge()
        .expect("abort_squash_merge should succeed");
    let after = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snapshot after abort");
    assert_eq!(after.head, Some(main_before));
    assert_eq!(after.operation, FinishOperation::None);
    assert!(!after.has_squash_msg, "SQUASH_MSG must be cleaned up");
    assert!(after.unmerged.is_empty());
    assert!(after.is_completely_clean());
}

#[test]
fn t_g2_integration_preview_clean_conflict_non_ascii_and_no_diff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    let clean_wt = dir.path().join("wt-clean");
    let conflict_wt = dir.path().join("wt-conflict");
    add_branch_worktree(&main_repo, &clean_wt, "feat/clean");
    add_branch_worktree(&main_repo, &conflict_wt, "feat/conflict");

    // Clean branch adds a file
    fs::write(clean_wt.join("new file.txt"), "hello\n").expect("write clean");
    run_git(&clean_wt, &["add", "new file.txt"]);
    run_git(&clean_wt, &["commit", "-m", "add new file"]);
    let clean_head = head_id(&clean_wt);

    // Main + conflict branch both edit a file with spaces and non-ASCII characters
    let special_name = "資料夾 測試/檔 名.txt";
    fs::create_dir_all(main_repo.join("資料夾 測試")).expect("mkdir main");
    fs::write(main_repo.join(special_name), "main version\n").expect("write special main");
    run_git(&main_repo, &["add", special_name]);
    run_git(&main_repo, &["commit", "-m", "add special on main"]);
    let main_head = head_id(&main_repo);

    fs::create_dir_all(conflict_wt.join("資料夾 測試")).expect("mkdir conflict");
    fs::write(conflict_wt.join(special_name), "branch version\n").expect("write special branch");
    run_git(&conflict_wt, &["add", special_name]);
    run_git(&conflict_wt, &["commit", "-m", "add special on branch"]);
    let conflict_head = head_id(&conflict_wt);

    let backend = GixBackend;
    let opened = backend.open(&main_repo).expect("open main");

    // Clean preview
    let clean_preview = opened
        .integration_preview(&main_head, &clean_head, &CancellationToken::new())
        .expect("clean preview");
    assert_eq!(clean_preview.ahead, 1);
    assert_eq!(clean_preview.behind, 1);
    assert!(clean_preview.merged_tree.is_some());
    assert!(clean_preview.conflict_paths.is_empty());
    assert!(clean_preview.linear);
    assert_eq!(clean_preview.commit_subjects, vec!["add new file"]);
    assert!(!clean_preview.likely_previously_squashed);

    // Conflict preview with spaces & non-ASCII
    let conflict_preview = opened
        .integration_preview(&main_head, &conflict_head, &CancellationToken::new())
        .expect("conflict preview");
    assert!(conflict_preview.merged_tree.is_none());
    assert_eq!(
        conflict_preview.conflict_paths,
        vec![PathBuf::from(special_name)]
    );

    // Simulate previously squash-landed branch (ahead > 0, same content as target)
    let squashed_wt = dir.path().join("wt-squashed");
    add_branch_worktree(&main_repo, &squashed_wt, "feat/squashed");
    fs::write(squashed_wt.join("same.txt"), "same content\n").expect("write same wt");
    run_git(&squashed_wt, &["add", "same.txt"]);
    run_git(&squashed_wt, &["commit", "-m", "same on branch"]);
    let squashed_head = head_id(&squashed_wt);

    fs::write(main_repo.join("same.txt"), "same content\n").expect("write same main");
    run_git(&main_repo, &["add", "same.txt"]);
    run_git(&main_repo, &["commit", "-m", "squash same onto main"]);
    let main_head2 = head_id(&main_repo);

    let no_diff_preview = opened
        .integration_preview(&main_head2, &squashed_head, &CancellationToken::new())
        .expect("no-diff preview");
    assert!(no_diff_preview.ahead > 0);
    assert!(!no_diff_preview.source_is_ancestor_of_target);
    assert_eq!(no_diff_preview.merged_tree, no_diff_preview.target_tree);
    assert!(no_diff_preview.likely_previously_squashed);
}

#[test]
fn t_g3_checked_worktree_remove_guards_dirty_untracked_locked_head_and_allows_clean_or_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    fs::write(main_repo.join(".gitignore"), "*.ignored\n").expect("write gitignore");
    run_git(&main_repo, &["add", ".gitignore"]);
    run_git(&main_repo, &["commit", "-m", "add gitignore"]);

    let backend = GixBackend;
    let opened = backend.open(&main_repo).expect("open main");

    // 1. Dirty tracked -> refused
    let dirty_wt = dir.path().join("wt-dirty");
    add_branch_worktree(&main_repo, &dirty_wt, "feat/dirty");
    let dirty_head = head_id(&dirty_wt);
    fs::write(dirty_wt.join("base.txt"), "modified\n").expect("modify");
    assert!(
        opened
            .remove_worktree_checked(&dirty_wt, &dirty_head)
            .is_err()
    );
    assert!(dirty_wt.exists());

    // 2. Untracked -> refused
    let untracked_wt = dir.path().join("wt-untracked");
    add_branch_worktree(&main_repo, &untracked_wt, "feat/untracked");
    let untracked_head = head_id(&untracked_wt);
    fs::write(untracked_wt.join("notes.tmp"), "untracked\n").expect("untracked");
    assert!(
        opened
            .remove_worktree_checked(&untracked_wt, &untracked_head)
            .is_err()
    );
    assert!(untracked_wt.exists());

    // 3. Locked -> refused
    let locked_wt = dir.path().join("wt-locked");
    add_branch_worktree(&main_repo, &locked_wt, "feat/locked");
    let locked_head = head_id(&locked_wt);
    run_git(
        &main_repo,
        &["worktree", "lock", locked_wt.to_str().unwrap()],
    );
    let locked_repo = backend.open(&locked_wt).expect("open locked");
    let locked_snap = locked_repo
        .finish_snapshot(&CancellationToken::new())
        .expect("locked snap");
    assert!(locked_snap.locked);
    assert!(
        opened
            .remove_worktree_checked(&locked_wt, &locked_head)
            .is_err()
    );
    assert!(locked_wt.exists());

    // 4. HEAD changed -> refused
    let moved_wt = dir.path().join("wt-moved");
    add_branch_worktree(&main_repo, &moved_wt, "feat/moved");
    let old_head = head_id(&moved_wt);
    fs::write(moved_wt.join("extra.txt"), "new\n").expect("extra");
    run_git(&moved_wt, &["add", "extra.txt"]);
    run_git(&moved_wt, &["commit", "-m", "new commit"]);
    assert!(
        opened
            .remove_worktree_checked(&moved_wt, &old_head)
            .is_err()
    );
    assert!(moved_wt.exists());

    // 5. Ignored-only file: snapshot discloses ignored_count == 1, non-force git worktree remove removes it and keeps branch ref.
    let ignored_wt = dir.path().join("wt-ignored");
    add_branch_worktree(&main_repo, &ignored_wt, "feat/ignored");
    let ignored_head = head_id(&ignored_wt);
    fs::write(ignored_wt.join("local.ignored"), "build artifact\n").expect("ignored");
    let ignored_repo = backend.open(&ignored_wt).expect("open ignored");
    let ignored_snap = ignored_repo
        .finish_snapshot(&CancellationToken::new())
        .expect("ignored snap");
    assert_eq!(ignored_snap.ignored_count, 1);
    assert!(ignored_snap.is_completely_clean());

    opened
        .remove_worktree_checked(&ignored_wt, &ignored_head)
        .expect("clean worktree with only ignored file should remove without --force");
    assert!(!ignored_wt.exists());
    // Branch ref must still exist!
    let branch_still_exists = opened
        .resolve_ref_commit("refs/heads/feat/ignored")
        .expect("resolve ref");
    assert_eq!(branch_still_exists, Some(ignored_head));
}

#[test]
fn t_g4_t_r4_t_r7_linked_worktree_rebase_keeps_target_and_ignores_rebase_merges_and_update_refs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    let feat_wt = dir.path().join("wt-feat");
    add_branch_worktree(&main_repo, &feat_wt, "feat/rebase");

    // Advance main
    fs::write(main_repo.join("main_only.txt"), "main\n").expect("main file");
    run_git(&main_repo, &["add", "main_only.txt"]);
    run_git(&main_repo, &["commit", "-m", "main advances"]);
    let main_head = head_id(&main_repo);

    // On feat_wt: create a commit, create sibling ref pointing to it, create a merge commit,
    // and enable rebase.rebaseMerges=true and rebase.updateRefs=true.
    fs::write(feat_wt.join("a.txt"), "a\n").expect("a");
    run_git(&feat_wt, &["add", "a.txt"]);
    run_git(&feat_wt, &["commit", "-m", "feat a"]);
    let sibling_before = head_id(&feat_wt);
    run_git(
        &feat_wt,
        &["branch", "feat/sibling", sibling_before.as_ref()],
    );

    run_git(&feat_wt, &["checkout", "-b", "feat/side", "HEAD~1"]);
    fs::write(feat_wt.join("b.txt"), "b\n").expect("b");
    run_git(&feat_wt, &["add", "b.txt"]);
    run_git(&feat_wt, &["commit", "-m", "feat b"]);
    run_git(&feat_wt, &["checkout", "feat/rebase"]);
    run_git(
        &feat_wt,
        &["merge", "--no-ff", "feat/side", "-m", "merge side"],
    );

    run_git(&feat_wt, &["config", "rebase.rebaseMerges", "true"]);
    run_git(&feat_wt, &["config", "rebase.updateRefs", "true"]);

    let backend = GixBackend;
    let feat_repo = backend.open(&feat_wt).expect("open feat");
    let step = feat_repo
        .rebase_onto_at(&main_head)
        .expect("rebase_onto_at");
    assert!(step.succeeded, "rebase should succeed cleanly");

    // Target branch must NOT have moved.
    assert_eq!(head_id(&main_repo), main_head);

    // Sibling branch must NOT have been moved by rebase.updateRefs=true.
    let sibling_after = feat_repo
        .resolve_ref_commit("refs/heads/feat/sibling")
        .expect("resolve sibling");
    assert_eq!(sibling_after, Some(sibling_before));

    // Rebased history must be linear (merge commit flattened despite rebase.rebaseMerges=true),
    // allowing squash_commits_with_output to compress it to 1 commit on top of main_head.
    let new_head = head_id(&feat_wt);
    let (oldest, count) = feat_repo
        .linear_range_after(&main_head, &new_head)
        .expect("linear range")
        .expect("range exists");
    assert_eq!(count, 2);
    feat_repo
        .squash_commits_with_output(&oldest, &new_head, "squashed feat/rebase")
        .expect("squash commits after flattened rebase");

    let squashed_head = head_id(&feat_wt);
    let snap = feat_repo
        .finish_snapshot(&CancellationToken::new())
        .expect("snap after squash");
    assert_eq!(snap.head, Some(squashed_head));
    assert_eq!(snap.head_parents, vec![main_head.clone()]);
    assert_eq!(head_id(&main_repo), main_head);
}

#[test]
fn t_g5_t_m7_snapshot_classifications_and_explicit_merge_branch_message() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    let clean_wt = dir.path().join("wt-m7");
    add_branch_worktree(&main_repo, &clean_wt, "feat/m7");

    fs::write(main_repo.join("m.txt"), "m\n").expect("m");
    run_git(&main_repo, &["add", "m.txt"]);
    run_git(&main_repo, &["commit", "-m", "main commit"]);

    fs::write(clean_wt.join("f.txt"), "f\n").expect("f");
    run_git(&clean_wt, &["add", "f.txt"]);
    run_git(&clean_wt, &["commit", "-m", "feat commit"]);
    let feat_head = head_id(&clean_wt);

    let backend = GixBackend;
    let opened = backend.open(&main_repo).expect("open main");
    let msg = default_merge_branch_message("feat/m7", "main");
    let step = opened
        .merge_commit_at(&feat_head, &msg)
        .expect("merge_commit_at");
    assert!(step.succeeded);

    let subject = git_stdout(&main_repo, &["log", "-1", "--format=%s"]);
    assert_eq!(subject, "Merge branch 'feat/m7' into main");

    let snap = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snap");
    assert_eq!(snap.operation, FinishOperation::None);
    assert_eq!(snap.head_parents.len(), 2);
    assert_eq!(snap.head_parents[1], feat_head);
}

#[test]
fn t_s6_squash_merge_with_rerere_autoupdate_exits_nonzero_with_no_unmerged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    run_git(&main_repo, &["config", "rerere.enabled", "true"]);
    run_git(&main_repo, &["config", "rerere.autoUpdate", "true"]);

    let feat_wt = dir.path().join("wt-rerere");
    add_branch_worktree(&main_repo, &feat_wt, "feat/rerere");

    fs::write(main_repo.join("base.txt"), "main-val\nline2\n").expect("main val");
    run_git(&main_repo, &["add", "base.txt"]);
    run_git(&main_repo, &["commit", "-m", "main change"]);

    fs::write(feat_wt.join("base.txt"), "feat-val\nline2\n").expect("feat val");
    run_git(&feat_wt, &["add", "base.txt"]);
    run_git(&feat_wt, &["commit", "-m", "feat change"]);
    let feat_head = head_id(&feat_wt);

    let backend = GixBackend;
    let opened = backend.open(&main_repo).expect("open main");

    // First conflict: resolve and record in rerere, then reset.
    let first = opened.squash_merge_at(&feat_head).expect("first conflict");
    assert!(!first.succeeded);
    fs::write(main_repo.join("base.txt"), "resolved-both\nline2\n").expect("resolve");
    run_git(&main_repo, &["add", "base.txt"]);
    run_git(&main_repo, &["rerere"]);
    opened
        .abort_squash_merge()
        .expect("reset before second attempt");

    // Second squash merge: rerere autoUpdate stages resolution, exit code is STILL non-zero!
    let second = opened
        .squash_merge_at(&feat_head)
        .expect("second squash merge");
    assert!(
        !second.succeeded,
        "rerere.autoUpdate squash merge must still report non-zero exit code"
    );
    let snap = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snap after rerere");
    assert!(
        snap.unmerged.is_empty(),
        "rerere autoUpdate cleared unmerged entries"
    );
    assert!(
        snap.staged_count > 0,
        "rerere autoUpdate staged the resolution"
    );
}

#[test]
fn f2_ignored_fingerprint_detects_content_edit_rename_and_subdir_change() {
    use gitcomet_core::worktree_finish::IgnoredFilesFingerprint;

    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    fs::write(main_repo.join(".gitignore"), "*.ignored\nignored_dir/\n").expect("gitignore");
    run_git(&main_repo, &["add", ".gitignore"]);
    run_git(&main_repo, &["commit", "-m", "update gitignore"]);

    let wt = dir.path().join("wt-ignored-fp");
    add_branch_worktree(&main_repo, &wt, "feat/ignored-fp");

    let backend = GixBackend;
    let opened = backend.open(&wt).expect("open wt");

    let snap_empty = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snap empty");
    assert_eq!(snap_empty.ignored_count, 0);
    assert_eq!(
        snap_empty.ignored_fingerprint,
        Some(IgnoredFilesFingerprint::empty_verified())
    );

    // 1. Same count (1), content edited -> digest MUST change!
    fs::write(wt.join("local.ignored"), "preview data\n").expect("write v1");
    let snap_v1 = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snap v1");
    assert_eq!(snap_v1.ignored_count, 1);
    let fp_v1 = snap_v1.ignored_fingerprint.clone().expect("fp v1");
    assert!(matches!(
        fp_v1,
        IgnoredFilesFingerprint::Verified { entry_count: 1, .. }
    ));

    fs::write(wt.join("local.ignored"), "NEW USER DATA AFTER PREVIEW\n").expect("write v2");
    let snap_v2 = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snap v2");
    assert_eq!(snap_v2.ignored_count, 1);
    let fp_v2 = snap_v2.ignored_fingerprint.clone().expect("fp v2");
    assert_ne!(
        fp_v1, fp_v2,
        "editing ignored file content with same count must change ignored_fingerprint"
    );

    // 2. Same count (1), renamed -> digest MUST change!
    fs::rename(wt.join("local.ignored"), wt.join("renamed.ignored")).expect("rename");
    let snap_v3 = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snap v3");
    assert_eq!(snap_v3.ignored_count, 1);
    let fp_v3 = snap_v3.ignored_fingerprint.clone().expect("fp v3");
    assert_ne!(
        fp_v2, fp_v3,
        "renaming ignored file with same count must change ignored_fingerprint"
    );

    // 3. File changed inside an ignored directory -> digest MUST change!
    fs::create_dir_all(wt.join("ignored_dir")).expect("mkdir");
    fs::write(wt.join("ignored_dir/inner.txt"), "before").expect("write inner v1");
    let snap_dir_v1 = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snap dir v1");
    fs::write(wt.join("ignored_dir/inner.txt"), "after edit").expect("write inner v2");
    let snap_dir_v2 = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("snap dir v2");
    assert_eq!(snap_dir_v1.ignored_count, snap_dir_v2.ignored_count);
    assert_ne!(
        snap_dir_v1.ignored_fingerprint, snap_dir_v2.ignored_fingerprint,
        "modifying a file inside an ignored directory must change ignored_fingerprint"
    );
}

#[test]
fn f4_abort_squash_merge_paths_preserves_unrelated_staged_and_working_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    let feat_wt = dir.path().join("wt-feat");
    add_branch_worktree(&main_repo, &feat_wt, "feat/squash-abort");

    fs::write(main_repo.join("base.txt"), "main change\n").expect("main change");
    run_git(&main_repo, &["add", "base.txt"]);
    run_git(&main_repo, &["commit", "-m", "main edits base"]);

    fs::write(feat_wt.join("base.txt"), "feat change\n").expect("feat change");
    fs::write(feat_wt.join("squash_added.txt"), "added by squash\n").expect("squash added");
    run_git(&feat_wt, &["add", "base.txt", "squash_added.txt"]);
    run_git(&feat_wt, &["commit", "-m", "feat edits base and adds file"]);
    let feat_head = head_id(&feat_wt);

    let backend = GixBackend;
    let opened = backend.open(&main_repo).expect("open main");

    let step = opened
        .squash_merge_at(&feat_head)
        .expect("squash merge conflict");
    assert!(!step.succeeded);
    let snap_conflict = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("conflict snap");
    assert_eq!(
        snap_conflict.operation,
        FinishOperation::SquashMergeConflict
    );
    let baseline = snap_conflict
        .dirty_entries
        .clone()
        .expect("baseline dirty entries captured");
    let owned_paths: Vec<PathBuf> = baseline.iter().map(|e| e.path.clone()).collect();
    assert!(owned_paths.contains(&PathBuf::from("base.txt")));
    assert!(owned_paths.contains(&PathBuf::from("squash_added.txt")));

    // User creates and stages an unrelated file AFTER the conflict!
    fs::write(
        main_repo.join("new-user-data.txt"),
        "NEW DATA AFTER CONFLICT",
    )
    .expect("write user data");
    run_git(&main_repo, &["add", "new-user-data.txt"]);
    fs::write(main_repo.join("notes.txt"), "untracked note").expect("write note");

    opened
        .abort_squash_merge_paths(&owned_paths)
        .expect("scoped squash abort");

    let snap_after = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("after abort snap");
    assert_eq!(snap_after.operation, FinishOperation::None);
    assert!(snap_after.unmerged.is_empty());
    assert!(!snap_after.has_squash_msg);
    assert_eq!(
        fs::read_to_string(main_repo.join("base.txt")).expect("read base"),
        "main change\n"
    );
    assert!(
        !main_repo.join("squash_added.txt").exists(),
        "squash-owned added file should be removed"
    );
    assert_eq!(
        fs::read_to_string(main_repo.join("new-user-data.txt")).expect("user file preserved"),
        "NEW DATA AFTER CONFLICT"
    );
    let status = git_stdout(&main_repo, &["status", "--porcelain"]);
    assert!(
        status.contains("A  new-user-data.txt"),
        "unrelated staged file must remain staged in index; got status:\n{status}"
    );
    assert_eq!(
        fs::read_to_string(main_repo.join("notes.txt")).expect("untracked preserved"),
        "untracked note"
    );
}

#[test]
fn f4_abort_squash_merge_paths_uses_literal_pathspecs_for_magic_filenames_and_refuses_directory_expansion()
 {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    let feat_wt = dir.path().join("wt-feat");
    add_branch_worktree(&main_repo, &feat_wt, "feat/literal-pathspec");

    // Tracked user file that MUST survive scoped squash abort even when conflict paths are pathspec magic.
    fs::write(main_repo.join("user.txt"), "original user content\n").expect("write user.txt");
    run_git(&main_repo, &["add", "user.txt"]);
    run_git(&main_repo, &["commit", "-m", "add user.txt"]);

    #[cfg(unix)]
    let conflict_names: &[&str] = &[
        "*",
        "?",
        "[a]",
        ":(glob)*",
        ":(icase)user.txt",
        ":!user.txt",
    ];
    #[cfg(not(unix))]
    let conflict_names: &[&str] = &["[a]"];

    #[cfg(unix)]
    let added_names: &[&str] = &["added_*", "added_:(glob)*"];
    #[cfg(not(unix))]
    let added_names: &[&str] = &["added_[b]"];

    for name in conflict_names {
        fs::write(main_repo.join(name), format!("main-{name}\n")).expect("write main magic");
    }
    let mut add_main = vec!["--literal-pathspecs", "add", "--"];
    add_main.extend_from_slice(conflict_names);
    run_git(&main_repo, &add_main);
    run_git(&main_repo, &["commit", "-m", "main magic files"]);

    for name in conflict_names {
        fs::write(feat_wt.join(name), format!("feat-{name}\n")).expect("write feat magic");
    }
    for name in added_names {
        fs::write(feat_wt.join(name), format!("feat-added-{name}\n")).expect("write feat added");
    }
    let mut add_feat = vec!["--literal-pathspecs", "add", "--"];
    add_feat.extend_from_slice(conflict_names);
    add_feat.extend_from_slice(added_names);
    run_git(&feat_wt, &add_feat);
    run_git(&feat_wt, &["commit", "-m", "feat magic files"]);
    let feat_head = head_id(&feat_wt);

    let backend = GixBackend;
    let opened = backend.open(&main_repo).expect("open main");
    let step = opened
        .squash_merge_at(&feat_head)
        .expect("squash merge conflict");
    assert!(!step.succeeded);

    let snap_conflict = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("conflict snap");
    assert_eq!(
        snap_conflict.operation,
        FinishOperation::SquashMergeConflict
    );
    let owned_paths: Vec<PathBuf> = snap_conflict
        .dirty_entries
        .clone()
        .expect("dirty entries captured")
        .into_iter()
        .map(|e| e.path)
        .collect();
    for name in conflict_names.iter().chain(added_names.iter()) {
        assert!(
            owned_paths.contains(&PathBuf::from(name)),
            "expected owned path '{name}' in baseline: {owned_paths:?}"
        );
    }

    // Post-conflict user modification + staging on tracked `user.txt`.
    fs::write(main_repo.join("user.txt"), "NEW USER STAGED DATA\n").expect("edit user.txt");
    run_git(&main_repo, &["add", "user.txt"]);

    opened
        .abort_squash_merge_paths(&owned_paths)
        .expect("literal-pathspec scoped squash abort should succeed");

    let snap_after = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("after abort snap");
    assert_eq!(snap_after.operation, FinishOperation::None);
    assert!(snap_after.unmerged.is_empty());
    assert!(!snap_after.has_squash_msg);

    for name in conflict_names {
        assert_eq!(
            fs::read_to_string(main_repo.join(name)).expect("read restored magic file"),
            format!("main-{name}\n"),
            "literal file '{name}' must be restored to HEAD"
        );
    }
    for name in added_names {
        assert!(
            !main_repo.join(name).exists(),
            "squash-added literal file '{name}' must be removed"
        );
    }
    assert_eq!(
        fs::read_to_string(main_repo.join("user.txt")).expect("read user.txt"),
        "NEW USER STAGED DATA\n",
        "F4: unrelated tracked user file content MUST be preserved"
    );
    let status = git_stdout(&main_repo, &["status", "--porcelain"]);
    assert_eq!(
        status.trim(),
        "M  user.txt",
        "F4: unrelated staged user change must remain staged in index"
    );

    // Uncertain ownership / directory-expansion refusal: passing a directory name ("sub") or
    // unsafe path ("..") must refuse before touching index, working tree, or SQUASH_MSG.
    fs::create_dir_all(main_repo.join("sub")).expect("mkdir sub");
    fs::write(main_repo.join("sub/keep.txt"), "initial sub\n").expect("write sub/keep");
    run_git(&main_repo, &["add", "sub/keep.txt"]);
    run_git(&main_repo, &["commit", "-m", "add sub/keep.txt"]);
    fs::write(main_repo.join("sub/keep.txt"), "EDITED SUB STAGED\n").expect("edit sub/keep");
    run_git(&main_repo, &["add", "sub/keep.txt"]);
    let squash_msg_path = main_repo.join(".git/SQUASH_MSG");
    fs::write(&squash_msg_path, "sentinel squash msg\n").expect("write sentinel SQUASH_MSG");

    assert!(
        opened.abort_squash_merge_paths(&[]).is_err(),
        "F4: empty owned path list must be refused"
    );
    assert!(
        opened
            .abort_squash_merge_paths(&[PathBuf::from("sub")])
            .is_err(),
        "F4: directory path expanding to unowned child file must be refused"
    );
    assert!(
        opened
            .abort_squash_merge_paths(&[PathBuf::from("..")])
            .is_err(),
        "F4: parent traversal path must be refused"
    );
    let untracked_dir = main_repo.join("untracked_dir");
    fs::create_dir_all(&untracked_dir).expect("mkdir untracked_dir");
    fs::write(untracked_dir.join("note.txt"), "UNTRACKED DIR NOTE\n").expect("write note.txt");
    assert!(
        opened
            .abort_squash_merge_paths(&[PathBuf::from("untracked_dir")])
            .is_err(),
        "F4: untracked directory path must be refused"
    );
    assert_eq!(
        fs::read_to_string(untracked_dir.join("note.txt")).expect("read untracked note"),
        "UNTRACKED DIR NOTE\n",
        "F4: refused untracked directory abort must preserve directory contents"
    );
    assert_eq!(
        fs::read_to_string(main_repo.join("sub/keep.txt")).expect("read sub/keep"),
        "EDITED SUB STAGED\n",
        "F4: refused abort must preserve working file"
    );
    assert!(
        squash_msg_path.is_file(),
        "F4: refused abort must not delete SQUASH_MSG"
    );
    assert!(
        git_stdout(&main_repo, &["status", "--porcelain"]).contains("M  sub/keep.txt"),
        "F4: refused abort must preserve staged index entry"
    );
}

#[test]
fn cleanup_refuses_primary_detached_and_in_progress_merge_worktrees() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    let main_head = head_id(&main_repo);
    let backend = GixBackend;
    let opened = backend.open(&main_repo).expect("open main");

    // 1. Primary worktree must always be refused
    assert!(
        opened
            .remove_worktree_checked(&main_repo, &main_head)
            .is_err()
    );
    assert!(main_repo.exists());

    // 2. Detached linked worktree must be refused
    let detached_wt = dir.path().join("wt-detached");
    add_branch_worktree(&main_repo, &detached_wt, "feat/detached-check");
    run_git(&detached_wt, &["checkout", "--detach", "HEAD"]);
    let detached_head = head_id(&detached_wt);
    assert!(
        opened
            .remove_worktree_checked(&detached_wt, &detached_head)
            .is_err()
    );
    assert!(detached_wt.exists());

    // 3. Linked worktree with in-progress merge must be refused
    let merge_wt = dir.path().join("wt-in-merge");
    add_branch_worktree(&main_repo, &merge_wt, "feat/in-merge");
    fs::write(main_repo.join("base.txt"), "main\n").expect("main");
    run_git(&main_repo, &["add", "base.txt"]);
    run_git(&main_repo, &["commit", "-m", "main conflict"]);
    fs::write(merge_wt.join("base.txt"), "branch\n").expect("branch");
    run_git(&merge_wt, &["add", "base.txt"]);
    run_git(&merge_wt, &["commit", "-m", "branch conflict"]);
    let merge_head = head_id(&merge_wt);
    let _ = backend
        .open(&merge_wt)
        .expect("open merge_wt")
        .merge_commit_at(&head_id(&main_repo), "conflict");
    assert!(
        opened
            .remove_worktree_checked(&merge_wt, &merge_head)
            .is_err()
    );
    assert!(merge_wt.exists());
}
