//! F4': a scoped squash abort must not restore an owned file over a directory that still
//! holds data the squash does not own.

use super::worktree_finish::{add_branch_worktree, head_id, init_repo};
use crate::git_cli::git_stdout;
use crate::git_cli::run_git;
use gitcomet_core::services::{CancellationToken, GitBackend};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};

fn squash_abort_site_state(repo: &Path) -> (String, String, bool) {
    (
        git_stdout(
            repo,
            &[
                "status",
                "--porcelain=v2",
                "--ignored",
                "--untracked-files=all",
            ],
        ),
        git_stdout(repo, &["ls-files", "--stage"]),
        repo.join(".git/SQUASH_MSG").is_file(),
    )
}

#[test]
fn f4_abort_squash_merge_paths_refuses_directory_to_file_restore_with_unowned_descendants() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = init_repo(dir.path());
    fs::write(main_repo.join(".gitignore"), "*.ignored\n").expect("write gitignore");
    fs::write(
        main_repo.join("slot"),
        "same contents for rename detection\n",
    )
    .expect("slot");
    run_git(&main_repo, &["add", ".gitignore", "slot"]);
    run_git(&main_repo, &["commit", "-m", "track slot file"]);
    let feat_wt = dir.path().join("wt-feat");
    add_branch_worktree(&main_repo, &feat_wt, "feat/slot-dir");

    fs::write(main_repo.join("base.txt"), "main change\n").expect("main change");
    run_git(&main_repo, &["add", "base.txt"]);
    run_git(&main_repo, &["commit", "-m", "main edits base"]);
    let main_head = head_id(&main_repo);

    fs::remove_file(feat_wt.join("slot")).expect("remove slot file");
    fs::create_dir(feat_wt.join("slot")).expect("mkdir slot");
    fs::write(
        feat_wt.join("slot/owned.txt"),
        "same contents for rename detection\n",
    )
    .expect("write slot/owned.txt");
    fs::write(feat_wt.join("base.txt"), "feat change\n").expect("feat change");
    run_git(&feat_wt, &["add", "-A"]);
    run_git(&feat_wt, &["commit", "-m", "move slot under its old path"]);
    let feat_head = head_id(&feat_wt);

    let opened = GixBackend.open(&main_repo).expect("open main");
    let step = opened.squash_merge_at(&feat_head).expect("squash conflict");
    assert!(!step.succeeded);
    let baseline = opened
        .finish_snapshot(&CancellationToken::new())
        .expect("conflict snap")
        .dirty_entries
        .expect("baseline dirty entries");
    // Same ownership the state layer derives: every baseline path plus rename origins.
    let mut owned_paths: Vec<PathBuf> = baseline
        .iter()
        .flat_map(|entry| std::iter::once(entry.path.clone()).chain(entry.orig_path.clone()))
        .collect();
    owned_paths.sort();
    owned_paths.dedup();
    assert!(
        owned_paths.contains(&PathBuf::from("slot")),
        "{owned_paths:?}"
    );
    assert!(
        owned_paths.contains(&PathBuf::from("slot/owned.txt")),
        "{owned_paths:?}"
    );
    assert!(main_repo.join("slot").is_dir());

    for (name, contents) in [
        (
            "slot/local.ignored",
            "ignored data written after the pause\n",
        ),
        ("slot/notes.txt", "untracked note written after the pause\n"),
    ] {
        let extra = main_repo.join(name);
        fs::write(&extra, contents).expect("write post-pause data");
        let before = squash_abort_site_state(&main_repo);
        let err = opened
            .abort_squash_merge_paths(&owned_paths)
            .expect_err("restoring slot as a file must not delete unowned descendants");
        assert!(
            err.to_string().contains(name),
            "refusal must name the blocking entry '{name}': {err}"
        );
        assert_eq!(
            fs::read_to_string(&extra).expect("post-pause data preserved"),
            contents
        );
        assert_eq!(
            squash_abort_site_state(&main_repo),
            before,
            "refused abort must leave index, worktree, conflict state and SQUASH_MSG untouched"
        );
        assert!(before.2, "SQUASH_MSG must remain for manual recovery");
        assert!(
            before.0.contains("u UU") && before.0.contains("R100 slot/owned.txt\tslot"),
            "conflict entries must remain: {}",
            before.0
        );
        fs::remove_file(&extra).expect("remove post-pause data");
    }

    // Positive control: once the directory holds only squash-owned entries, the restore runs.
    opened
        .abort_squash_merge_paths(&owned_paths)
        .expect("provable directory-to-file restore");
    assert_eq!(head_id(&main_repo), main_head);
    assert!(main_repo.join("slot").is_file());
    assert_eq!(
        fs::read_to_string(main_repo.join("base.txt")).expect("read base"),
        "main change\n"
    );
    assert_eq!(git_stdout(&main_repo, &["status", "--porcelain"]), "");
}
