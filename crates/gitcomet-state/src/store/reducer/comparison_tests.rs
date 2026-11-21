use super::*;
use crate::model::{AppState, ComparisonSource, Loadable, RepoState};
use crate::msg::{CommitSelectMode, Effect};
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::{
    Commit, CommitFileChange, CommitId, Diff, DiffLine, DiffLineKind, DiffTarget, FileStatusKind,
    LogPage, RepoSpec,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::process::{GitExecutableAvailability, GitExecutablePreference, GitRuntimeState};
use std::sync::atomic::AtomicU64;

fn commit(id: &str, parent: &str) -> Commit {
    Commit {
        id: CommitId(id.into()),
        parent_ids: smallvec::smallvec![CommitId(parent.into())],
        summary: id.into(),
        author: "Tester".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

/// A repo whose loaded log is newest-first `c3, c2, c1` (so `c1` is oldest).
/// Every commit has a parent — including the oldest, whose parent `c0` is
/// simply older than the loaded page — so the merged-diff base is a real
/// parent rather than the root-commit fallback.
fn state_with_log(repo_id: RepoId) -> AppState {
    let mut state = AppState {
        git_runtime: GitRuntimeState {
            preference: GitExecutablePreference::SystemPath,
            availability: GitExecutableAvailability::Available {
                version_output: "git version 2.0.0".to_string(),
            },
        },
        ..Default::default()
    };
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.history_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![commit("c3", "c2"), commit("c2", "c1"), commit("c1", "c0")],
        next_cursor: None,
    }));
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);
    state
}

fn dispatch_effects(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(99);
    reduce(&mut repos, &id_alloc, state, msg)
}

fn repo(state: &AppState, repo_id: RepoId) -> &RepoState {
    state.repos.iter().find(|r| r.id == repo_id).unwrap()
}

fn select(state: &mut AppState, repo_id: RepoId, id: &str, mode: CommitSelectMode) -> Vec<Effect> {
    dispatch_effects(
        state,
        Msg::SelectCommitMulti {
            request_id: None,
            repo_id,
            commit_id: CommitId(id.into()),
            mode,
            clicked_index: None,
            visible_order: None,
        },
    )
}

#[test]
fn selecting_two_commits_enters_ordered_range_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let c1 = CommitId("c1".into());
    let c3 = CommitId("c3".into());

    select(&mut state, repo_id, "c2", CommitSelectMode::Single);
    let effects = select(&mut state, repo_id, "c3", CommitSelectMode::Toggle);

    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("two selected commits should start a comparison");
    // The base is the *parent* of the oldest selected commit, regardless of
    // click order, so the merged diff includes that commit's own changes.
    assert_eq!(range.from, c1);
    assert_eq!(range.to, Some(c3.clone()));
    assert_eq!(range.source, ComparisonSource::MergedSelection);

    // The diff pane stays empty: the comparison presents the file
    // side-selection first, and the user opens a file to view its diff.
    assert_eq!(repo(&state, repo_id).diff_state.diff_target, None);
    assert!(matches!(
        repo(&state, repo_id).history_state.range_files,
        Loadable::Loading
    ));
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadRangeFiles { from, to, .. } if *from == c1 && *to == Some(c3.clone())
        )),
        "a LoadRangeFiles effect for c1->c3 should be issued"
    );
}

fn toggle_adjacent(repo_id: RepoId, from: &str, to: &str) -> Msg {
    Msg::ToggleAdjacentComparison {
        repo_id,
        from: CommitId(from.into()),
        to: CommitId(to.into()),
    }
}

#[test]
fn adjacent_comparison_switches_revisions_without_moving_selection() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    let initial = select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    let selected = repo(&state, repo_id).history_state.multi_selection.clone();
    let focused = repo(&state, repo_id).history_state.selected_commit.clone();

    let request = |state: &AppState, effects: Vec<Effect>, expected_from, expected_source| {
        let repo = repo(state, repo_id);
        let range = repo.history_state.range_selection.as_ref().unwrap();
        assert_eq!(range.from.as_ref(), expected_from);
        assert_eq!(range.to.as_ref().unwrap().as_ref(), "c3");
        assert_eq!(range.source, expected_source);
        assert_eq!(repo.history_state.multi_selection, selected);
        assert!(matches!(repo.history_state.range_files, Loadable::Loading));
        assert_eq!(repo.diff_state.diff_target, None);
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadRangeFiles { from, to, load, .. }
                    if from.as_ref() == expected_from
                        && to.as_ref().is_some_and(|id| id.as_ref() == "c3") =>
                {
                    Some(load.request)
                }
                _ => None,
            })
            .unwrap()
    };
    let cumulative_request = request(&state, initial, "c1", ComparisonSource::MergedSelection);
    let previous_diff = gitcomet_core::domain::DiffTarget::commit_range(
        CommitId("c1".into()),
        Some(CommitId("c3".into())),
        Some("old-file".into()),
    );
    state.repos[0].diff_state.diff_target = Some(previous_diff);
    state.repos[0].set_range_files(Loadable::Ready(Arc::new(vec![])));
    let endpoints = dispatch_effects(&mut state, toggle_adjacent(repo_id, "c1", "c3"));
    let endpoint_request = request(&state, endpoints, "c2", ComparisonSource::AdjacentPair);
    assert_ne!(endpoint_request, cumulative_request);
    assert_eq!(repo(&state, repo_id).history_state.selected_commit, focused);

    // A context click on either selected commit only moves focus.
    let effects = select(
        &mut state,
        repo_id,
        "c3",
        CommitSelectMode::PreserveIfSelected,
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. }))
    );
    assert_eq!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .as_ref()
            .unwrap()
            .source,
        ComparisonSource::AdjacentPair,
    );
    let cumulative = dispatch_effects(&mut state, toggle_adjacent(repo_id, "c2", "c3"));
    let new_request = request(&state, cumulative, "c1", ComparisonSource::MergedSelection);
    assert_ne!(new_request, cumulative_request);

    // A prior request for these same cumulative endpoints cannot set the list.
    for id in [cumulative_request, new_request] {
        dispatch_effects(
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
                repo_id,
                from: CommitId("c1".into()),
                to: Some(CommitId("c3".into())),
                request: id,
                result: Ok(gitcomet_core::services::Comparison::new(
                    CommitId("c1".into()),
                    vec![],
                )),
            }),
        );
        assert_eq!(
            matches!(
                repo(&state, repo_id).history_state.range_files,
                Loadable::Ready(_)
            ),
            id == new_request,
        );
    }
}

#[test]
fn adjacent_comparison_ignores_a_stale_click_or_an_ineligible_selection() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    let other_repo = RepoId(2);
    state.repos.extend(state_with_log(other_repo).repos);
    state.active_repo = Some(other_repo);
    let effects = dispatch_effects(&mut state, toggle_adjacent(repo_id, "c1", "c3"));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. }))
    );

    state.active_repo = Some(repo_id);
    let effects = dispatch_effects(&mut state, toggle_adjacent(repo_id, "c2", "c3"));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. }))
    );

    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);
    let effects = dispatch_effects(&mut state, toggle_adjacent(repo_id, "c1", "c3"));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. }))
    );
    assert_eq!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .as_ref()
            .unwrap()
            .source,
        ComparisonSource::SelectedPair,
    );
}

#[test]
fn range_files_loaded_populates_only_the_current_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    let effects = select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);
    let request = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadRangeFiles { load, .. } => Some(load.request),
            _ => None,
        })
        .expect("a range-file load should be issued");

    let files = vec![
        CommitFileChange::new(std::path::PathBuf::from("a.rs"), FileStatusKind::Modified)
            .with_line_stats((Some(1), Some(0)).into()),
    ];

    // A stale result (wrong `from`) is dropped.
    dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: CommitId("c9".into()),
            to: Some(CommitId("c3".into())),
            request,
            result: Ok(gitcomet_core::services::Comparison::new(
                CommitId("c9".into()),
                files.clone(),
            )),
        }),
    );
    assert!(matches!(
        repo(&state, repo_id).history_state.range_files,
        Loadable::Loading
    ));

    // A reply from an *overtaken* load for the very same endpoints is
    // dropped too. This is the case `(from, to)` cannot catch: a
    // commit↔working-tree comparison keeps its pair across every refresh, so
    // only the request id distinguishes a current reply from a late one.
    dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: CommitId("c0".into()),
            to: Some(CommitId("c3".into())),
            request: request.wrapping_sub(1),
            result: Ok(gitcomet_core::services::Comparison::new(
                CommitId("c0".into()),
                files.clone(),
            )),
        }),
    );
    assert!(matches!(
        repo(&state, repo_id).history_state.range_files,
        Loadable::Loading
    ));

    // The matching result populates the list.
    dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: CommitId("c0".into()),
            to: Some(CommitId("c3".into())),
            request,
            result: Ok(gitcomet_core::services::Comparison::new(
                CommitId("c0".into()),
                files.clone(),
            )),
        }),
    );
    match &repo(&state, repo_id).history_state.range_files {
        Loadable::Ready(loaded) => assert_eq!(loaded.as_ref(), &files),
        other => panic!("expected loaded range files, got {other:?}"),
    }
}

#[test]
fn single_selection_clears_an_active_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_some()
    );

    select(&mut state, repo_id, "c2", CommitSelectMode::Single);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_none(),
        "collapsing to a single commit ends the comparison"
    );
}

fn loaded_details(id: &str) -> gitcomet_core::domain::CommitDetails {
    gitcomet_core::domain::CommitDetails {
        id: CommitId(id.into()),
        message: format!("{id} message"),
        author_name: "Tester".into(),
        author_email: "t@example.com".into(),
        authored_at_unix: 0,
        committed_at: String::new(),
        committed_at_unix: 0,
        parent_ids: Vec::new(),
        files: Vec::new(),
    }
}

/// Entering a comparison moves `selected_commit` to the focused commit
/// without loading its details — the comparison view owns the pane, so that
/// load would be wasted. Leaving the comparison is therefore the moment the
/// details pane has to be put back in sync, or it keeps rendering whichever
/// commit's details were loaded last under a different commit's selection.
#[test]
fn closing_a_comparison_reloads_the_focused_commits_details() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);

    // c3 selected, its details loaded.
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    state.repos[0].history_state.commit_details = Loadable::Ready(Arc::new(loaded_details("c3")));

    // Ctrl-click c2: comparison mode, focus moves to c2, details stay c3's.
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_some()
    );
    assert_eq!(
        repo(&state, repo_id).history_state.selected_commit,
        Some(CommitId("c2".into()))
    );

    let effects = dispatch_effects(&mut state, Msg::ClearComparison { repo_id });

    let r = repo(&state, repo_id);
    assert!(
        !matches!(&r.history_state.commit_details, Loadable::Ready(d) if d.id == CommitId("c3".into())),
        "c3's details must not stay on screen under c2's selection"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadCommitDetails { commit_id, .. } if *commit_id == CommitId("c2".into())
        )),
        "closing the comparison should load the still-selected commit's details"
    );
}

/// Every plain history click leaves a commit in `multi_selection`, so a
/// comparison started from a context menu finds a stale one sitting there.
/// It describes a different comparison, so it must not survive to name this
/// one or supply its preview cards.
#[test]
fn an_explicit_comparison_drops_a_stale_multi_selection() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .multi_selection
            .is_multi(),
        "precondition: a multi-selection comparison is active"
    );

    dispatch_effects(
        &mut state,
        Msg::CompareWithWorkingTree {
            repo_id,
            from: CommitId("c2".into()),
            from_label: "main".into(),
        },
    );

    let r = repo(&state, repo_id);
    assert!(
        r.history_state.multi_selection.commits.is_empty(),
        "the previous selection is not part of this comparison"
    );
    let range = r
        .history_state
        .range_selection
        .clone()
        .expect("the explicit comparison replaces the previous one");
    assert_eq!(range.from, CommitId("c2".into()));
    assert_eq!(range.to, None);
}

/// A multi-selection comparison keeps its selection: there, the selection
/// *is* what is being compared, and the UI names the comparison after it.
#[test]
fn a_multi_selection_comparison_keeps_its_selection() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);

    let r = repo(&state, repo_id);
    assert!(r.history_state.range_selection.is_some());
    assert!(r.history_state.multi_selection.is_multi());
}

#[test]
fn clear_comparison_dismisses_selection_and_diff() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_some()
    );

    dispatch_effects(&mut state, Msg::ClearComparison { repo_id });
    let r = repo(&state, repo_id);
    assert!(r.history_state.range_selection.is_none());
    assert!(!r.history_state.multi_selection.is_multi());
    assert_eq!(r.diff_state.diff_target, None);
}

#[test]
fn mark_then_compare_with_marked_builds_the_range() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let c1 = CommitId("c1".into());
    let c3 = CommitId("c3".into());

    // Nothing marked yet: comparing is a no-op.
    let effects = dispatch_effects(
        &mut state,
        Msg::CompareWithMarked {
            repo_id,
            commit_id: c3.clone(),
            label: "c3".into(),
        },
    );
    assert!(effects.is_empty());
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_none()
    );

    // Mark c1 (base), then compare c3 against it.
    dispatch_effects(
        &mut state,
        Msg::MarkForComparison {
            repo_id,
            commit_id: c1.clone(),
            label: "main".into(),
        },
    );
    dispatch_effects(
        &mut state,
        Msg::CompareWithMarked {
            repo_id,
            commit_id: c3.clone(),
            label: "feature".into(),
        },
    );
    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("compare with marked should start a comparison");
    assert_eq!(range.from, c1);
    assert_eq!(range.to, Some(c3.clone()));
    assert_eq!(range.from_label, "main");
    assert_eq!(range.to_label, "feature");
    assert_eq!(range.source, ComparisonSource::Explicit);
}

#[test]
fn compare_commit_range_message_orders_via_labels() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let effects = dispatch_effects(
        &mut state,
        Msg::CompareCommitRange {
            repo_id,
            from: CommitId("c1".into()),
            to: CommitId("c3".into()),
            from_label: "main".into(),
            to_label: "feature".into(),
        },
    );
    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("explicit compare should set a comparison");
    assert_eq!(range.from_label, "main");
    assert_eq!(range.to_label, "feature");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. }))
    );
}

#[test]
fn compare_with_working_tree_starts_a_worktree_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let from = CommitId("c2".into());

    let effects = dispatch_effects(
        &mut state,
        Msg::CompareWithWorkingTree {
            repo_id,
            from: from.clone(),
            from_label: "main".into(),
        },
    );

    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("compare with working tree should start a comparison");
    assert_eq!(range.from, from);
    // The tip is the working tree, not a commit.
    assert_eq!(range.to, None);
    assert_eq!(range.to_label, "Working tree");
    // A worktree-tip file list load is issued, and the diff pane is cleared.
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadRangeFiles { from: f, to: None, .. } if *f == from
    )));
    assert_eq!(repo(&state, repo_id).diff_state.diff_target, None);
}

/// A refresh means two full-tree `git diff` calls, so changes arriving while
/// one is running must fold into it rather than each starting their own —
/// and the fold must still end with a run that sees the final state.
#[test]
fn external_worktree_changes_refresh_a_worktree_comparison_one_at_a_time() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let from = CommitId("c2".into());
    let effects = dispatch_effects(
        &mut state,
        Msg::CompareWithWorkingTree {
            repo_id,
            from: from.clone(),
            from_label: "main".into(),
        },
    );
    let load_request = |effects: &[Effect]| {
        effects.iter().find_map(|e| match e {
            Effect::LoadRangeFiles {
                from: f,
                to: None,
                load,
                ..
            } if *f == from => Some(load.request),
            _ => None,
        })
    };
    let first = load_request(&effects).expect("the comparison issues a file-list load");

    // Two changes land while that load is still running: neither starts its
    // own, they collapse into the one already in flight.
    for _ in 0..2 {
        let effects = dispatch_effects(
            &mut state,
            Msg::RepoExternallyChanged {
                repo_id,
                change: crate::msg::RepoExternalChange::Worktree,
            },
        );
        assert!(
            load_request(&effects).is_none(),
            "a refresh must not stack on top of one already in flight"
        );
    }

    // When it lands, the folded changes are honoured by exactly one re-run,
    // so the list ends up describing the worktree as it is now.
    let effects = dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: from.clone(),
            to: None,
            request: first,
            result: Ok(gitcomet_core::services::Comparison::new(
                from.clone(),
                Vec::new(),
            )),
        }),
    );
    let second = load_request(&effects).expect("the folded refresh runs once the load lands");
    assert_ne!(first, second, "the re-run is a new request, not a replay");

    // Nothing further is queued, so a quiet worktree stops the chain.
    let effects = dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: from.clone(),
            to: None,
            request: second,
            result: Ok(gitcomet_core::services::Comparison::new(
                from.clone(),
                Vec::new(),
            )),
        }),
    );
    assert!(load_request(&effects).is_none());

    // And with nothing in flight, the next change refreshes immediately.
    let effects = dispatch_effects(
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        load_request(&effects).is_some(),
        "expected the worktree comparison file list to refresh"
    );
}

#[test]
fn external_change_does_not_refresh_a_commit_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    // Two-commit (immutable) comparison.
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_some()
    );

    let effects = dispatch_effects(
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. })),
        "a commit↔commit comparison is immutable and must not refresh"
    );
}

/// Two commits with a gap between them compare directly, `c1..c3`, the way
/// `git diff c1 c3` does, rather than claiming to be their merged diff.
#[test]
fn selecting_two_commits_with_a_gap_compares_them_directly() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let c1 = CommitId("c1".into());
    let c3 = CommitId("c3".into());

    select(&mut state, repo_id, "c1", CommitSelectMode::Single);
    let effects = select(&mut state, repo_id, "c3", CommitSelectMode::Toggle);

    let history = &repo(&state, repo_id).history_state;
    assert!(history.multi_selection.is_multi());
    let range = history
        .range_selection
        .as_ref()
        .expect("two selected commits start a comparison");
    assert_eq!(range.from, c1);
    assert_eq!(range.to, Some(c3.clone()));
    assert_eq!(range.source, ComparisonSource::SelectedPair);
    assert_eq!(range.from_label, "c1");
    assert_eq!(range.to_label, "c3");
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadRangeFiles { from, to, .. } if *from == c1 && *to == Some(c3.clone())
        )),
        "a LoadRangeFiles effect for c1->c3 should be issued"
    );
}

/// Two branch tips are the usual reason to pick two commits: the lower one in
/// the log is the base, whichever was clicked first.
#[test]
fn selecting_two_branch_tips_compares_them_directly() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    state.repos[0].history_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![
            commit("a1", "base"),
            commit("b1", "base"),
            commit("base", "c0"),
        ],
        next_cursor: None,
    }));

    for [first, second] in [["a1", "b1"], ["b1", "a1"]] {
        select(&mut state, repo_id, first, CommitSelectMode::Single);
        select(&mut state, repo_id, second, CommitSelectMode::Toggle);

        let range = repo(&state, repo_id)
            .history_state
            .range_selection
            .clone()
            .expect("two branch tips start a comparison");
        assert_eq!(range.from, CommitId("b1".into()), "{first} then {second}");
        assert_eq!(
            range.to,
            Some(CommitId("a1".into())),
            "{first} then {second}"
        );
        assert_eq!(range.source, ComparisonSource::SelectedPair);
    }
}

/// Three or more commits with a gap have no single diff holding exactly their
/// changes, so they stay a plain list until the gap closes.
#[test]
fn selecting_three_commits_with_a_gap_shows_no_diff_until_the_gap_closes() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    state.repos[0].history_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![
            commit("c4", "c3"),
            commit("c3", "c2"),
            commit("c2", "c1"),
            commit("c1", "c0"),
        ],
        next_cursor: None,
    }));

    select(&mut state, repo_id, "c4", CommitSelectMode::Single);
    select(&mut state, repo_id, "c3", CommitSelectMode::Toggle);
    let effects = select(&mut state, repo_id, "c1", CommitSelectMode::Toggle);

    let history = &repo(&state, repo_id).history_state;
    assert_eq!(history.multi_selection.commits.len(), 3);
    assert_eq!(history.range_selection, None);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. })),
        "a selection with a gap must not load a diff"
    );

    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("closing the gap starts the comparison");
    assert_eq!(range.from, CommitId("c0".into()));
    assert_eq!(range.to, Some(CommitId("c4".into())));
    assert_eq!(range.source, ComparisonSource::MergedSelection);
}

/// The list landing again can change the selected file's change: deleted on
/// disk while a working-tree comparison shows it, say. The diff then loads
/// the one side it has left instead of a patch against nothing.
#[test]
fn comparison_list_refresh_replans_a_file_that_lost_a_side() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    let c1 = CommitId("c1".into());
    let c3 = CommitId("c3".into());
    select(&mut state, repo_id, "c2", CommitSelectMode::Single);
    select(&mut state, repo_id, "c3", CommitSelectMode::Toggle);
    let land = |state: &mut AppState, kind: FileStatusKind| {
        let request = state.repos[0].begin_range_files_load().request;
        dispatch_effects(
            state,
            Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
                repo_id,
                from: CommitId("c1".into()),
                to: Some(CommitId("c3".into())),
                request,
                result: Ok(gitcomet_core::services::Comparison::new(
                    CommitId("c1".into()),
                    vec![CommitFileChange::new(
                        std::path::PathBuf::from("a.rs"),
                        kind,
                    )],
                )),
            }),
        )
    };
    land(&mut state, FileStatusKind::Modified);
    dispatch_effects(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: gitcomet_core::domain::DiffTarget::commit_range(
                c1,
                Some(c3),
                Some(std::path::PathBuf::from("a.rs")),
            ),
        },
    );
    assert_eq!(repo(&state, repo_id).diff_state.one_sided_side, None);

    let effects = land(&mut state, FileStatusKind::Deleted);
    let diff_state = &repo(&state, repo_id).diff_state;
    assert_eq!(
        diff_state.one_sided_side,
        Some(gitcomet_core::domain::DiffPreviewTextSide::Old)
    );
    assert!(matches!(
        diff_state.diff_preview_text_file,
        Loadable::Loading
    ));
    assert!(matches!(diff_state.diff, Loadable::NotLoaded));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadDiffPreviewTextFile { .. })),
        "the old side should load: {effects:?}"
    );

    // The same list again changes nothing, so nothing reloads.
    let effects = land(&mut state, FileStatusKind::Deleted);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadDiffPreviewTextFile { .. })),
        "{effects:?}"
    );
}

/// A comparison of `c3`/`c2` with one of its files open.
fn state_with_comparison_file_open(repo_id: RepoId) -> AppState {
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    select(&mut state, repo_id, "c2", CommitSelectMode::Toggle);
    dispatch_effects(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: gitcomet_core::domain::DiffTarget::commit_range(
                CommitId("c1".into()),
                Some(CommitId("c3".into())),
                Some("a.txt".into()),
            ),
        },
    );
    assert!(repo(&state, repo_id).diff_state.diff_target.is_some());
    state
}

/// A file diff opened from the comparison belongs to it, so leaving the
/// comparison for a single commit closes it.
#[test]
fn collapsing_to_one_commit_closes_the_comparisons_file_diff() {
    let repo_id = RepoId(1);
    let mut state = state_with_comparison_file_open(repo_id);
    select(&mut state, repo_id, "c2", CommitSelectMode::Single);
    assert_eq!(repo(&state, repo_id).diff_state.diff_target, None);
}

#[test]
fn clearing_the_commit_selection_closes_the_comparisons_file_diff() {
    let repo_id = RepoId(1);
    let mut state = state_with_comparison_file_open(repo_id);
    dispatch_effects(
        &mut state,
        Msg::ClearCommitSelection {
            request_id: None,
            repo_id,
        },
    );
    assert_eq!(repo(&state, repo_id).history_state.range_selection, None);
    assert_eq!(repo(&state, repo_id).diff_state.diff_target, None);
}

/// A right-click keeps the selection, so the same pair must not reload its list.
#[test]
fn refocusing_within_a_pair_keeps_its_loaded_comparison() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c1", CommitSelectMode::Single);
    select(&mut state, repo_id, "c3", CommitSelectMode::Toggle);
    state.repos[0].history_state.range_files = Loadable::Ready(Arc::new(Vec::new()));
    let rev = state.repos[0].history_state.range_files_rev;

    let effects = select(
        &mut state,
        repo_id,
        "c1",
        CommitSelectMode::PreserveIfSelected,
    );

    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. })),
        "the unchanged pair must not reload"
    );
    let history = &repo(&state, repo_id).history_state;
    assert!(matches!(history.range_files, Loadable::Ready(_)));
    assert_eq!(history.range_files_rev, rev);
    assert_eq!(history.selected_commit, Some(CommitId("c1".into())));
}

/// Commit-time order can list a parent above its child (date skew in a
/// shallow clone). The pair is still a chain, newest being the child.
#[test]
fn a_parent_listed_above_its_child_still_gets_the_merged_diff() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    state.repos[0].history_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![commit("parent", "c0"), commit("child", "parent")],
        next_cursor: None,
    }));

    select(&mut state, repo_id, "parent", CommitSelectMode::Single);
    select(&mut state, repo_id, "child", CommitSelectMode::Toggle);

    let range = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .expect("a parent and its child compare");
    assert_eq!(range.from, CommitId("c0".into()));
    assert_eq!(range.to, Some(CommitId("child".into())));
    assert_eq!(range.source, ComparisonSource::MergedSelection);
}

/// An editor opened from the comparison's diff returns to it on exit; once the
/// comparison has ended, exiting must not reopen that dead diff.
#[test]
fn leaving_the_editor_after_the_comparison_ended_does_not_reopen_its_diff() {
    let repo_id = RepoId(1);
    let mut state = state_with_comparison_file_open(repo_id);
    dispatch_effects(
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: "a.txt".into(),
        },
    );
    assert!(repo(&state, repo_id).diff_state.edit_mode);
    select(&mut state, repo_id, "c2", CommitSelectMode::Single);

    dispatch_effects(&mut state, Msg::ExitDiffEditMode { repo_id });

    assert!(!matches!(
        repo(&state, repo_id).diff_state.diff_target,
        Some(gitcomet_core::domain::DiffTarget::CommitRange { .. })
    ));
}

fn open_range_file(state: &mut AppState, repo_id: RepoId, path: &str) {
    dispatch_effects(
        state,
        Msg::SelectDiff {
            repo_id,
            target: gitcomet_core::domain::DiffTarget::commit_range(
                CommitId("c1".into()),
                Some(CommitId("c3".into())),
                Some(path.into()),
            ),
        },
    );
}

/// Back/Forward between two files of a live pair keeps the pair.
#[test]
fn stepping_back_between_a_pairs_files_keeps_the_pair() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c1", CommitSelectMode::Single);
    select(&mut state, repo_id, "c3", CommitSelectMode::Toggle);
    open_range_file(&mut state, repo_id, "a.txt");
    open_range_file(&mut state, repo_id, "b.txt");

    let effects = dispatch_effects(&mut state, Msg::GlobalNavBack { repo_id });

    let history = &repo(&state, repo_id).history_state;
    assert_eq!(history.multi_selection.commits.len(), 2);
    assert_eq!(
        history.range_selection.as_ref().map(|range| range.source),
        Some(ComparisonSource::SelectedPair)
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. })),
        "the unchanged pair must not reload its list"
    );
    assert_eq!(
        repo(&state, repo_id)
            .diff_state
            .diff_target
            .as_ref()
            .and_then(|target| target.file_path()),
        Some(std::path::Path::new("a.txt"))
    );
}

/// A pair restored by Back comes back as an explicit comparison; the history
/// entry it was restored from must still describe the view.
#[test]
fn a_restored_pair_matches_its_history_entry() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c1", CommitSelectMode::Single);
    select(&mut state, repo_id, "c3", CommitSelectMode::Toggle);
    open_range_file(&mut state, repo_id, "a.txt");
    dispatch_effects(
        &mut state,
        Msg::CompareWithWorkingTree {
            repo_id,
            from: CommitId("c2".into()),
            from_label: "c2".into(),
        },
    );

    dispatch_effects(&mut state, Msg::GlobalNavBack { repo_id });

    let repo = repo(&state, repo_id);
    let nav = &repo.navigation.main_history;
    assert!(repo.main_view_snapshot_matches(&nav.entries[nav.cursor]));
    assert!(nav.cursor + 1 < nav.entries.len(), "Forward is still there");
}

fn range_load_cancellation(effects: &[Effect]) -> gitcomet_core::services::CancellationToken {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadRangeFiles { load, .. } => Some(load.cancellation.clone()),
            _ => None,
        })
        .expect("a range-file load")
}

/// A load nobody will read is stopped rather than left to run to the end.
#[test]
fn a_superseded_or_abandoned_range_load_is_cancelled() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c1", CommitSelectMode::Single);
    let first =
        range_load_cancellation(&select(&mut state, repo_id, "c3", CommitSelectMode::Toggle));
    let second =
        range_load_cancellation(&select(&mut state, repo_id, "c2", CommitSelectMode::Toggle));
    assert!(
        first.is_cancelled(),
        "a newer comparison supersedes the pair's load"
    );
    assert!(!second.is_cancelled());

    select(&mut state, repo_id, "c2", CommitSelectMode::Single);
    assert!(
        second.is_cancelled(),
        "leaving the comparison abandons its load"
    );
}

fn untracked_comparison_fixture() -> (AppState, crate::model::RangeSelection, Vec<Effect>) {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    select(&mut state, repo_id, "c3", CommitSelectMode::Single);
    let effects = dispatch_effects(
        &mut state,
        Msg::CompareWithOptions {
            repo_id,
            from: CommitId("c2".into()),
            to: None,
            options: gitcomet_core::services::ComparisonOptions::merge_base(),
            from_label: "topic".into(),
            to_label: "Working tree".into(),
        },
    );
    let expected = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .unwrap();
    (state, expected, effects)
}

fn live_comparison_reply(
    state: &mut AppState,
    request: u64,
    files: std::result::Result<Vec<CommitFileChange>, Error>,
) -> Vec<Effect> {
    dispatch_effects(
        state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id: RepoId(1),
            from: CommitId("c2".into()),
            to: None,
            request,
            result: files.map(|files| {
                gitcomet_core::services::Comparison::new(CommitId("c1".into()), files)
            }),
        }),
    )
}

fn select_live_comparison_file(state: &mut AppState, kind: FileStatusKind) -> DiffTarget {
    let repo_id = RepoId(1);
    let expected = repo(state, repo_id)
        .history_state
        .range_selection
        .clone()
        .unwrap();
    dispatch_effects(
        state,
        Msg::ToggleWorkingTreeComparisonUntracked { repo_id, expected },
    );
    let request = repo(state, repo_id).history_state.range_files_request;
    live_comparison_reply(
        state,
        request,
        Ok(vec![CommitFileChange::new("selected.txt".into(), kind)]),
    );
    let target = DiffTarget::commit_range(CommitId("c1".into()), None, Some("selected.txt".into()));
    dispatch_effects(
        state,
        Msg::SelectDiff {
            repo_id,
            target: target.clone(),
        },
    );
    assert_eq!(
        repo(state, repo_id).diff_state.diff_target.as_ref(),
        Some(&target)
    );
    target
}

fn notify_live_comparison_change(state: &mut AppState) -> Vec<Effect> {
    dispatch_effects(
        state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::Worktree,
        },
    )
}

#[test]
fn live_comparison_defers_diff_reads_and_keeps_the_loaded_patch_until_confirmation() {
    let repo_id = RepoId(1);
    let (mut state, _, _) = untracked_comparison_fixture();
    let target = select_live_comparison_file(&mut state, FileStatusKind::Modified);
    let load_patch = |state: &mut AppState, result| {
        dispatch_effects(
            state,
            Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                repo_id,
                target: target.clone(),
                result,
            }),
        )
    };
    let shown = Diff {
        target: target.clone(),
        lines: vec![DiffLine {
            kind: DiffLineKind::Add,
            text: "before the refresh".into(),
        }],
    };
    load_patch(&mut state, Ok(shown.clone()));
    let effects = notify_live_comparison_change(&mut state);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRangeFiles { .. }))
    );
    assert!(!effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiff { .. } | Effect::LoadDiffFile { .. } | Effect::LoadSelectedDiff { .. }
    )));
    for stale in [
        Ok(Diff {
            target: target.clone(),
            lines: Vec::new(),
        }),
        Err(Error::new(ErrorKind::Backend("early error".into()))),
    ] {
        load_patch(&mut state, stale);
        let actual = repo(&state, repo_id).diff_state.diff.ready().unwrap();
        assert_eq!(actual.as_ref(), &shown);
    }

    let request = repo(&state, repo_id).history_state.range_files_request;
    let effects = live_comparison_reply(
        &mut state,
        request,
        Ok(vec![CommitFileChange::new(
            "selected.txt".into(),
            FileStatusKind::Modified,
        )]),
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(
                e,
                Effect::LoadSelectedDiff {
                    load_patch_diff: true,
                    ..
                }
            ))
            .count(),
        1,
        "an affected surviving file must reload once",
    );
    let refreshed = Diff {
        target: target.clone(),
        lines: vec![DiffLine {
            kind: DiffLineKind::Add,
            text: "after the refresh".into(),
        }],
    };
    load_patch(&mut state, Ok(refreshed.clone()));
    assert_eq!(
        repo(&state, repo_id)
            .diff_state
            .diff
            .ready()
            .unwrap()
            .as_ref(),
        &refreshed
    );
}

#[test]
fn live_comparison_does_not_reload_an_unaffected_or_replaced_selection() {
    let repo_id = RepoId(1);
    let (mut state, _, _) = untracked_comparison_fixture();
    let target = select_live_comparison_file(&mut state, FileStatusKind::Modified);
    let notify = |state: &mut AppState, path: &str| {
        dispatch_effects(
            state,
            Msg::RepoExternallyChanged {
                repo_id,
                change: crate::msg::RepoExternalChange {
                    paths: crate::msg::ChangedPaths::known(vec![path.into()]),
                    ..crate::msg::RepoExternalChange::worktree()
                },
            },
        )
    };
    let files = vec![
        CommitFileChange::new("selected.txt".into(), FileStatusKind::Modified),
        CommitFileChange::new("other.txt".into(), FileStatusKind::Modified),
    ];
    notify(&mut state, "other.txt");
    let request = repo(&state, repo_id).history_state.range_files_request;
    assert!(live_comparison_reply(&mut state, request, Ok(files.clone())).is_empty());
    assert_eq!(
        repo(&state, repo_id).diff_state.diff_target.as_ref(),
        Some(&target)
    );

    let _ = notify(&mut state, "selected.txt");
    let request = repo(&state, repo_id).history_state.range_files_request;
    let other = DiffTarget::commit_range(CommitId("c1".into()), None, Some("other.txt".into()));
    dispatch_effects(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: other.clone(),
        },
    );
    assert!(live_comparison_reply(&mut state, request, Ok(files)).is_empty());
    assert_eq!(
        repo(&state, repo_id).diff_state.diff_target.as_ref(),
        Some(&other)
    );
}

#[test]
fn live_comparison_refresh_rebuilds_the_target_if_the_base_or_rename_changes() {
    let repo_id = RepoId(1);
    let (mut state, _, _) = untracked_comparison_fixture();
    select_live_comparison_file(&mut state, FileStatusKind::Modified);
    notify_live_comparison_change(&mut state);
    let request = repo(&state, repo_id).history_state.range_files_request;
    let base = CommitId("new-base".into());
    let file = CommitFileChange::new("selected.txt".into(), FileStatusKind::Renamed)
        .with_old_path(Some("old-name.txt".into()));
    let effects = dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
            repo_id,
            from: CommitId("c2".into()),
            to: None,
            request,
            result: Ok(gitcomet_core::services::Comparison::new(
                base.clone(),
                vec![file],
            )),
        }),
    );
    let current = repo(&state, repo_id)
        .diff_state
        .diff_target
        .as_ref()
        .unwrap();
    let expected = DiffTarget::commit_range(base, None, Some("selected.txt".into()));
    assert_eq!(current, &expected);
    let old_path = Some(std::path::Path::new("old-name.txt"));
    assert_eq!(current.old_file_path(), old_path);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(
                e,
                Effect::LoadSelectedDiff {
                    load_patch_diff: true,
                    ..
                }
            ))
            .count(),
        1
    );
}

#[test]
fn live_comparison_waits_when_the_newly_selected_file_is_also_affected() {
    let repo_id = RepoId(1);
    let (mut state, _, _) = untracked_comparison_fixture();
    let current_request = repo(&state, repo_id).history_state.range_files_request;
    live_comparison_reply(
        &mut state,
        current_request,
        Ok(vec![
            CommitFileChange::new("first.txt".into(), FileStatusKind::Modified),
            CommitFileChange::new("second.txt".into(), FileStatusKind::Modified),
        ]),
    );
    let select_path = |state: &mut AppState, path: &str| {
        dispatch_effects(
            state,
            Msg::SelectDiff {
                repo_id,
                target: DiffTarget::commit_range(CommitId("c1".into()), None, Some(path.into())),
            },
        )
    };
    assert!(!select_path(&mut state, "first.txt").is_empty());
    notify_live_comparison_change(&mut state);
    assert!(select_path(&mut state, "second.txt").is_empty());
    let selected = DiffTarget::commit_range(CommitId("c1".into()), None, Some("second.txt".into()));
    assert_eq!(
        repo(&state, repo_id).diff_state.diff_target.as_ref(),
        Some(&selected)
    );
    dispatch_effects(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id,
            result: Ok(Diff {
                target: selected.clone(),
                lines: Vec::new(),
            }),
            target: selected,
        }),
    );
    assert!(matches!(
        repo(&state, repo_id).diff_state.diff,
        Loadable::Loading
    ));
    let request = repo(&state, repo_id).history_state.range_files_request;
    live_comparison_reply(&mut state, request, Ok(Vec::new()));
    assert!(repo(&state, repo_id).diff_state.diff_target.is_none());
}

#[test]
fn live_comparison_failure_finishes_a_loading_diff_but_preserves_loaded_content() {
    let repo_id = RepoId(1);
    for already_read in [false, true] {
        let (mut state, _, _) = untracked_comparison_fixture();
        let target = select_live_comparison_file(&mut state, FileStatusKind::Modified);
        let old_patch = Diff {
            target: target.clone(),
            lines: vec![DiffLine {
                kind: DiffLineKind::Add,
                text: "existing hunk".into(),
            }],
        };
        if already_read {
            dispatch_effects(
                &mut state,
                Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                    repo_id,
                    target: target.clone(),
                    result: Ok(old_patch.clone()),
                }),
            );
        }
        notify_live_comparison_change(&mut state);
        let request = repo(&state, repo_id).history_state.range_files_request;
        let effects = live_comparison_reply(
            &mut state,
            request,
            Err(Error::new(ErrorKind::Backend("temporary error".into()))),
        );
        assert!(effects.is_empty());
        let r = repo(&state, repo_id);
        assert_eq!(r.diff_state.diff_target.as_ref(), Some(&target));
        assert!(
            matches!(&r.history_state.range_files, Loadable::Error(msg) if msg.contains("temporary error"))
        );
        if already_read {
            assert_eq!(r.diff_state.diff.ready().unwrap().as_ref(), &old_patch);
        } else {
            assert!(
                matches!(&r.diff_state.diff, Loadable::Error(msg) if msg.contains("temporary error"))
            );
        }
    }
}

#[test]
fn live_comparison_probes_an_extensionless_file_after_its_deferred_selection() {
    let repo_id = RepoId(1);
    let (mut state, _, _) = untracked_comparison_fixture();
    let file = CommitFileChange::new("no-extension".into(), FileStatusKind::Modified);
    let request = repo(&state, repo_id).history_state.range_files_request;
    live_comparison_reply(&mut state, request, Ok(vec![file.clone()]));
    notify_live_comparison_change(&mut state);
    let target = DiffTarget::commit_range(CommitId("c1".into()), None, Some(file.path.clone()));
    let effects = dispatch_effects(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: target.clone(),
        },
    );
    assert!(effects.is_empty());
    let request = repo(&state, repo_id).history_state.range_files_request;
    let effects = live_comparison_reply(&mut state, request, Ok(vec![file]));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiffFileImage { target: t, .. } if t == &target))
    );
}

#[test]
fn live_comparison_closes_a_file_missing_from_the_latest_list() {
    let repo_id = RepoId(1);
    for kind in [FileStatusKind::Untracked, FileStatusKind::Modified] {
        let (mut state, _, _) = untracked_comparison_fixture();
        let target = select_live_comparison_file(&mut state, kind);
        let history = &repo(&state, repo_id).history_state;
        let range = history.range_selection.clone();
        let selected = history.selected_commit.clone();

        notify_live_comparison_change(&mut state);
        let request = repo(&state, repo_id).history_state.range_files_request;
        let other = CommitFileChange::new("other.txt".into(), FileStatusKind::Modified);
        let effects = live_comparison_reply(&mut state, request, Ok(vec![other.clone()]));
        let r = repo(&state, repo_id);
        assert_eq!(r.diff_state.diff_target, None, "removed {kind:?}");
        assert_eq!(r.history_state.range_selection, range);
        assert_eq!(r.history_state.selected_commit, selected);
        assert_eq!(
            r.history_state.range_files.ready().unwrap().as_slice(),
            &[other]
        );
        assert!(
            effects.is_empty(),
            "no diff should reload for a removed row"
        );
        let nav = &r.navigation.main_history;
        assert!(r.main_view_snapshot_matches(&nav.entries[nav.cursor]));

        dispatch_effects(
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                repo_id,
                target,
                result: Err(Error::new(ErrorKind::Backend(
                    "late load of a vanished path".into(),
                ))),
            }),
        );
        let r = repo(&state, repo_id);
        assert_eq!(r.diff_state.diff_target, None);
        assert!(matches!(r.diff_state.diff, Loadable::NotLoaded));
    }
}

#[test]
fn live_comparison_waits_for_a_current_success_and_keeps_a_surviving_file() {
    let repo_id = RepoId(1);
    let (mut state, _, _) = untracked_comparison_fixture();
    let target = select_live_comparison_file(&mut state, FileStatusKind::Modified);

    notify_live_comparison_change(&mut state);
    let first = repo(&state, repo_id).history_state.range_files_request;
    notify_live_comparison_change(&mut state);
    let effects = live_comparison_reply(&mut state, first, Ok(Vec::new()));
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadRangeFiles { .. }]
    ));
    let r = repo(&state, repo_id);
    assert_eq!(r.diff_state.diff_target.as_ref(), Some(&target));
    assert_eq!(r.history_state.range_files.ready().unwrap().len(), 1);
    let latest = r.history_state.range_files_request;
    assert_ne!(latest, first);

    live_comparison_reply(&mut state, first, Ok(Vec::new()));
    live_comparison_reply(
        &mut state,
        latest,
        Err(Error::new(ErrorKind::Backend(
            "comparison could not finish".into(),
        ))),
    );
    assert_eq!(
        repo(&state, repo_id).diff_state.diff_target.as_ref(),
        Some(&target)
    );
    notify_live_comparison_change(&mut state);
    let request = repo(&state, repo_id).history_state.range_files_request;
    live_comparison_reply(
        &mut state,
        request,
        Ok(vec![
            CommitFileChange::new("other.txt".into(), FileStatusKind::Added),
            CommitFileChange::new("selected.txt".into(), FileStatusKind::Deleted),
        ]),
    );
    let r = repo(&state, repo_id);
    assert_eq!(r.diff_state.diff_target.as_ref(), Some(&target));
    assert_eq!(
        r.diff_state.one_sided_side,
        Some(gitcomet_core::domain::DiffPreviewTextSide::Old),
    );
}

#[test]
fn working_tree_untracked_toggle_reverses_without_moving_selection_or_navigation() {
    let repo_id = RepoId(1);
    let (mut state, expected, first_effects) = untracked_comparison_fixture();
    let first_cancellation = range_load_cancellation(&first_effects);
    let first_request = repo(&state, repo_id).history_state.range_files_request;
    // A loaded merge base is not part of the gesture identity, and must be
    // resolved again rather than shown as a pending result for the next load.
    state.repos[0]
        .history_state
        .range_selection
        .as_mut()
        .unwrap()
        .base = Some(CommitId("resolved".into()));
    dispatch_effects(
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: gitcomet_core::domain::DiffTarget::commit_range(
                CommitId("resolved".into()),
                None,
                Some("tracked.txt".into()),
            ),
        },
    );
    let selected = repo(&state, repo_id).history_state.selected_commit.clone();
    let multi = repo(&state, repo_id).history_state.multi_selection.clone();
    let nav_len = repo(&state, repo_id).navigation.main_history.entries.len();
    let nav_cursor = repo(&state, repo_id).navigation.main_history.cursor;
    let earlier_steps =
        repo(&state, repo_id).navigation.main_history.entries[..nav_cursor].to_vec();
    let effects = dispatch_effects(
        &mut state,
        Msg::ToggleWorkingTreeComparisonUntracked { repo_id, expected },
    );
    let current = repo(&state, repo_id)
        .history_state
        .range_selection
        .as_ref()
        .unwrap();
    assert_eq!(current.from, CommitId("c2".into()));
    assert_eq!(current.to, None);
    assert_eq!(current.from_label, "topic");
    assert_eq!(current.to_label, "Working tree");
    assert_eq!(
        current.options.base,
        gitcomet_core::services::ComparisonBase::MergeBase
    );
    assert!(current.options.include_untracked);
    assert_eq!(current.base, None);
    assert!(effects.iter().any(|effect| matches!(effect, Effect::LoadRangeFiles {from, to: None, options, ..} if *from == current.from && options.include_untracked && options.base == current.options.base)));
    assert!(first_cancellation.is_cancelled());
    assert_ne!(
        repo(&state, repo_id).history_state.range_files_request,
        first_request
    );
    assert_eq!(repo(&state, repo_id).diff_state.diff_target, None);
    assert_eq!(
        repo(&state, repo_id).history_state.selected_commit,
        selected
    );
    assert_eq!(repo(&state, repo_id).history_state.multi_selection, multi);
    assert_eq!(
        repo(&state, repo_id).navigation.main_history.entries.len(),
        nav_len
    );
    assert_eq!(
        repo(&state, repo_id).navigation.main_history.cursor,
        nav_cursor
    );

    let expected = current.clone();
    let current_cancellation = range_load_cancellation(&effects);
    dispatch_effects(
        &mut state,
        Msg::ToggleWorkingTreeComparisonUntracked { repo_id, expected },
    );
    let current = repo(&state, repo_id)
        .history_state
        .range_selection
        .as_ref()
        .unwrap();
    assert!(!current.options.include_untracked);
    assert_eq!(
        current.options.base,
        gitcomet_core::services::ComparisonBase::MergeBase
    );
    assert!(current_cancellation.is_cancelled());
    assert_eq!(
        repo(&state, repo_id).history_state.selected_commit,
        selected
    );
    // Returning to the previous comparison with its file diff cleared makes
    // the tail identical to that earlier step. Keep only one copy.
    let navigation = &repo(&state, repo_id).navigation.main_history;
    assert_eq!(navigation.entries, earlier_steps);
    assert_eq!(navigation.entries.len(), nav_len - 1);
    assert_eq!(navigation.cursor, nav_cursor - 1);
    assert!(
        navigation.can_back(),
        "the earlier single-commit step remains reachable"
    );
    dispatch_effects(&mut state, Msg::GlobalNavBack { repo_id });
    assert_eq!(repo(&state, repo_id).navigation.main_history.cursor, 0);
    assert!(repo(&state, repo_id).main_view_snapshot_matches(&earlier_steps[0]));
    assert_eq!(
        repo(&state, repo_id).history_state.selected_commit,
        selected
    );
    assert!(
        repo(&state, repo_id)
            .history_state
            .range_selection
            .is_none()
    );
}

#[test]
fn working_tree_untracked_toggle_ignores_same_sha_other_labels_options_and_external_repo() {
    let repo_id = RepoId(1);
    for mismatch in [
        "from_label",
        "to_label",
        "options",
        "external_repo",
        "missing",
    ] {
        let (mut state, expected, _) = untracked_comparison_fixture();
        match mismatch {
            "from_label" => {
                state.repos[0]
                    .history_state
                    .range_selection
                    .as_mut()
                    .unwrap()
                    .from_label = "tag-at-same-sha".into()
            }
            "to_label" => {
                state.repos[0]
                    .history_state
                    .range_selection
                    .as_mut()
                    .unwrap()
                    .to_label = "another worktree".into()
            }
            "options" => {
                state.repos[0]
                    .history_state
                    .range_selection
                    .as_mut()
                    .unwrap()
                    .options
                    .include_untracked = true
            }
            "external_repo" => {
                let mut other = state.repos[0].clone();
                other.id = RepoId(2);
                state.repos.push(other);
                state.active_repo = Some(RepoId(2));
            }
            _ => state.repos[0].history_state.range_selection = None,
        }
        let before = repo(&state, repo_id).history_state.range_selection.clone();
        let request = repo(&state, repo_id).history_state.range_files_request;
        assert!(
            dispatch_effects(
                &mut state,
                Msg::ToggleWorkingTreeComparisonUntracked { repo_id, expected }
            )
            .is_empty(),
            "{mismatch}"
        );
        assert_eq!(
            repo(&state, repo_id).history_state.range_selection,
            before,
            "{mismatch}"
        );
        assert_eq!(
            repo(&state, repo_id).history_state.range_files_request,
            request,
            "{mismatch}"
        );
    }
}

#[test]
fn working_tree_untracked_toggle_rejects_commit_pairs_and_requires_working_tree_git() {
    let repo_id = RepoId(1);
    let mut state = state_with_log(repo_id);
    dispatch_effects(
        &mut state,
        Msg::CompareCommitRange {
            repo_id,
            from: CommitId("c1".into()),
            to: CommitId("c3".into()),
            from_label: "older".into(),
            to_label: "newer".into(),
        },
    );
    let expected = repo(&state, repo_id)
        .history_state
        .range_selection
        .clone()
        .unwrap();
    let request = repo(&state, repo_id).history_state.range_files_request;
    let message = Msg::ToggleWorkingTreeComparisonUntracked {
        repo_id,
        expected: expected.clone(),
    };
    assert_eq!(
        message.repository_requirement(),
        Some((
            repo_id,
            gitcomet_core::domain::RepositoryRequirement::WorkingTree
        ))
    );
    assert!(msg_requires_available_git(&message));
    assert!(dispatch_effects(&mut state, message).is_empty());
    assert_eq!(
        repo(&state, repo_id).history_state.range_selection,
        Some(expected)
    );
    assert_eq!(
        repo(&state, repo_id).history_state.range_files_request,
        request
    );
}

#[test]
fn working_tree_untracked_toggle_rejects_overtaken_file_list_replies() {
    let repo_id = RepoId(1);
    let (mut state, expected, _) = untracked_comparison_fixture();
    let old_request = repo(&state, repo_id).history_state.range_files_request;
    dispatch_effects(
        &mut state,
        Msg::ToggleWorkingTreeComparisonUntracked { repo_id, expected },
    );
    let current_request = repo(&state, repo_id).history_state.range_files_request;
    for (request, name) in [(old_request, "stale.txt"), (current_request, "fresh.txt")] {
        dispatch_effects(
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
                repo_id,
                from: CommitId("c2".into()),
                to: None,
                request,
                result: Ok(gitcomet_core::services::Comparison::new(
                    CommitId("base".into()),
                    vec![CommitFileChange::new(
                        std::path::PathBuf::from(name),
                        FileStatusKind::Added,
                    )],
                )),
            }),
        );
        if request == old_request {
            assert!(matches!(
                repo(&state, repo_id).history_state.range_files,
                Loadable::Loading
            ));
            assert_eq!(
                repo(&state, repo_id)
                    .history_state
                    .range_selection
                    .as_ref()
                    .unwrap()
                    .base,
                None
            );
        } else {
            let files = repo(&state, repo_id)
                .history_state
                .range_files
                .ready()
                .unwrap();
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].path, std::path::PathBuf::from("fresh.txt"));
        }
    }
}
