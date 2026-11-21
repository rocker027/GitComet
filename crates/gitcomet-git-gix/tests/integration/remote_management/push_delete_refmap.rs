//! The tracking-ref refresh that follows a remote branch deletion fetches explicit
//! `refs/remotes/*` refspecs only. A configured `remote.<name>.fetch` refspec that writes a
//! local branch must not ride along, or deleting a remote branch could move a branch that a
//! Worktree Finish run (or the user) expects to stay put.

use super::*;

const LIVE_REFSPEC_INTO_LOCAL_BRANCH: &str = "+refs/heads/live:refs/heads/release";

fn rev_parse(repo: &Path, revision: &str) -> String {
    run_git_capture(repo, &["rev-parse", "--verify", revision])
        .trim()
        .to_string()
}

fn commit_file(repo: &Path, name: &str, contents: &str, message: &str) {
    fs::write(repo.join(name), contents).expect("write file");
    run_git(repo, &["add", name]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
}

fn init_bare(path: &Path) {
    fs::create_dir_all(path).expect("create bare remote dir");
    run_git(path, &["init", "--bare", "-b", "main"]);
}

/// A work repo on `main` whose local `release` stays at the base commit while `live` is one
/// commit ahead. Returns `(base, live_tip)`.
fn init_work_repo_with_release_and_live(work_repo: &Path) -> (String, String) {
    fs::create_dir_all(work_repo).expect("create work repo dir");
    run_git(work_repo, &["init", "-b", "main"]);
    configure_repo_with_user(work_repo);
    commit_file(work_repo, "base.txt", "base\n", "base");
    let base = rev_parse(work_repo, "HEAD");
    run_git(work_repo, &["branch", "release", "main"]);
    run_git(work_repo, &["checkout", "-b", "live", "main"]);
    commit_file(work_repo, "live.txt", "live\n", "live advances");
    let live_tip = rev_parse(work_repo, "HEAD");
    run_git(work_repo, &["checkout", "main"]);
    (base, live_tip)
}

#[test]
fn failed_batch_delete_refresh_ignores_configured_refspecs_into_local_branches() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().expect("create tempdir");
    let remote_repo = dir.path().join("remote.git");
    let work_repo = dir.path().join("work");
    init_bare(&remote_repo);
    let (base, live_tip) = init_work_repo_with_release_and_live(&work_repo);
    let remote_url = git_remote_url(&remote_repo);
    run_git(&work_repo, &["remote", "add", "origin", &remote_url]);
    run_git(&work_repo, &["push", "origin", "main", "live"]);
    // A stale tracking ref proves the refresh really fetched the live branch.
    run_git(
        &work_repo,
        &["update-ref", "refs/remotes/origin/live", &base],
    );
    run_git(
        &work_repo,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            LIVE_REFSPEC_INTO_LOCAL_BRANCH,
        ],
    );

    let opened = GixBackend.open(&work_repo).expect("open work repo");
    // `gone` never existed on the remote, so Git rejects the whole batch.
    let result = opened
        .delete_remote_branches_with_output("origin", &["gone".to_string(), "live".to_string()]);

    assert!(
        result.is_err(),
        "a missing refspec must surface as an error"
    );
    assert_eq!(
        rev_parse(&remote_repo, "refs/heads/live"),
        live_tip,
        "the rejected batch must leave live on the remote"
    );
    assert_eq!(
        rev_parse(&work_repo, "refs/remotes/origin/live"),
        live_tip,
        "the surviving branch's tracking ref must still be refreshed"
    );
    assert_eq!(
        rev_parse(&work_repo, "refs/heads/release"),
        base,
        "the refresh must not apply the configured refspec that writes refs/heads/release"
    );
}

#[test]
fn pushurl_delete_refresh_ignores_configured_refspecs_into_local_branches() {
    let _guard = remote_management_test_lock();
    let dir = tempfile::tempdir().expect("create tempdir");
    let fetch_remote = dir.path().join("fetch.git");
    let push_remote = dir.path().join("push.git");
    let work_repo = dir.path().join("work");
    init_bare(&fetch_remote);
    init_bare(&push_remote);
    let (base, live_tip) = init_work_repo_with_release_and_live(&work_repo);
    let fetch_url = git_remote_url(&fetch_remote);
    let push_url = git_remote_url(&push_remote);
    run_git(&work_repo, &["remote", "add", "origin", &fetch_url]);
    run_git(&work_repo, &["push", "origin", "main", "live"]);
    run_git(&work_repo, &["push", &push_url, "main", "live"]);
    run_git(&work_repo, &["config", "remote.origin.pushurl", &push_url]);
    run_git(
        &work_repo,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            LIVE_REFSPEC_INTO_LOCAL_BRANCH,
        ],
    );

    let opened = GixBackend.open(&work_repo).expect("open work repo");
    opened
        .delete_remote_branch_with_output("origin", "live")
        .expect("delete live from the push endpoint");

    assert!(
        !run_git_status(
            &push_remote,
            &["show-ref", "--verify", "--quiet", "refs/heads/live"],
        )
        .success(),
        "live should be deleted from the push endpoint"
    );
    assert_eq!(
        rev_parse(&work_repo, "refs/remotes/origin/live"),
        live_tip,
        "the fetch endpoint still has live, so its tracking ref must be restored"
    );
    assert_eq!(
        rev_parse(&work_repo, "refs/heads/release"),
        base,
        "the refresh must not apply the configured refspec that writes refs/heads/release"
    );
}
