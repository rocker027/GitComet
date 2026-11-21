//! First-parent navigation over the displayed commits, independent of list rows.

use crate::view::caches::HistoryVisibleIndices;
use gitcomet_core::domain::{Commit, CommitId};
use gitcomet_core::history_index::{HistoryProjection, MISSING_PARENT};
use gitcomet_core::services::{CancellationToken, Result};
use rustc_hash::FxHashMap;
use std::cell::OnceCell;
use std::sync::Arc;

/// Follow the first parent through known hidden commits. Memoizing hidden
/// chains keeps a newer-child scan from walking the same chain for every row.
fn first_visible_parent(
    from: usize,
    visible_at: &impl Fn(usize) -> Option<usize>,
    parent: &impl Fn(usize) -> Option<usize>,
    hidden_parents: &mut FxHashMap<usize, Option<usize>>,
) -> Option<usize> {
    first_visible_parent_checked(from, visible_at, parent, hidden_parents, &mut || Ok(()))
        .expect("infallible branch traversal")
}

fn first_visible_parent_checked(
    from: usize,
    visible_at: &impl Fn(usize) -> Option<usize>,
    parent: &impl Fn(usize) -> Option<usize>,
    hidden_parents: &mut FxHashMap<usize, Option<usize>>,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Option<usize>> {
    let mut next = parent(from);
    let mut path = Vec::new();
    let result = loop {
        check()?;
        let Some(raw) = next else {
            break None;
        };
        if let Some(visible) = visible_at(raw) {
            break Some(visible);
        }
        if let Some(cached) = hidden_parents.get(&raw) {
            break *cached;
        }
        // An unfinished entry also terminates a malformed hidden-parent cycle.
        hidden_parents.insert(raw, None);
        path.push(raw);
        next = parent(raw);
    };
    for raw in path {
        check()?;
        hidden_parents.insert(raw, result);
    }
    Ok(result)
}

fn indexed_first_parent(projection: &HistoryProjection, raw: usize) -> Option<usize> {
    // Projection::parents filters hidden edges and could promote a merge's
    // second parent. Read the raw first edge, stopping at missing parents.
    projection
        .index
        .parents(raw)
        .first()
        .copied()
        .filter(|&row| row != MISSING_PARENT)
        .map(|row| row as usize)
}

/// One default newer neighbor for each visible row. Built with the projection
/// in the background; key handlers must only read this packed table.
#[derive(Clone, Debug)]
pub(super) struct IndexedBranchChildren {
    children: Arc<[u32]>,
}

impl IndexedBranchChildren {
    pub(super) fn build(
        projection: &HistoryProjection,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        cancellation.check_cancelled()?;
        let mut children = vec![MISSING_PARENT; projection.len()];
        let mut hidden_parents = FxHashMap::default();
        let all_visible = projection.len() == projection.index.len();
        let visible_at = |raw| projection.visible_position(raw);
        let mut steps = 0usize;
        let mut check = || {
            steps = steps.wrapping_add(1);
            if steps.is_multiple_of(1024) {
                cancellation.check_cancelled()?;
            }
            Ok(())
        };
        for child in 0..projection.len() {
            if child.is_multiple_of(1024) {
                cancellation.check_cancelled()?;
            }
            let parent = if all_visible {
                // Visible and raw positions coincide, so there are no hidden
                // parent chains to walk on this path.
                indexed_first_parent(projection, child)
            } else {
                let Some(raw) = projection.raw_position(child) else {
                    continue;
                };
                first_visible_parent_checked(
                    raw,
                    &visible_at,
                    &|raw| indexed_first_parent(projection, raw),
                    &mut hidden_parents,
                    &mut check,
                )?
            };
            let Some(parent) = parent else {
                continue;
            };
            if child == parent {
                continue;
            }
            let current = children[parent];
            // Children are visited in display order. The last preceding child
            // is closest; only the first following child is the fallback.
            if current == MISSING_PARENT || child < parent {
                children[parent] = u32::try_from(child).expect("indexed row fits u32");
            }
        }
        cancellation.check_cancelled()?;
        Ok(Self {
            children: children.into(),
        })
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn bytes(&self) -> usize {
        std::mem::size_of_val(self.children.as_ref())
    }

    #[cfg(test)]
    pub(super) fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.children, &other.children)
    }

    fn child(&self, parent: usize) -> Option<usize> {
        self.children
            .get(parent)
            .copied()
            .filter(|&row| row != MISSING_PARENT)
            .map(|row| row as usize)
    }
}

#[allow(clippy::too_many_arguments)]
fn branch_neighbor(
    from_raw: usize,
    from_visible: usize,
    direction: i8,
    preferred_child: Option<usize>,
    visible_len: usize,
    raw_at: impl Fn(usize) -> Option<usize>,
    visible_at: impl Fn(usize) -> Option<usize>,
    parent: impl Fn(usize) -> Option<usize>,
) -> Option<usize> {
    if direction == 0 {
        return None;
    }
    let mut hidden_parents = FxHashMap::default();
    if direction > 0 {
        return first_visible_parent(from_raw, &visible_at, &parent, &mut hidden_parents)
            .filter(|&row| row != from_visible);
    }
    let mut is_child = |row| {
        row != from_visible
            && raw_at(row).is_some_and(|raw| {
                first_visible_parent(raw, &visible_at, &parent, &mut hidden_parents)
                    == Some(from_visible)
            })
    };
    if let Some(preferred) = preferred_child.filter(|&row| is_child(row)) {
        return Some(preferred);
    }
    // Topological/date ordering normally places children before the parent.
    // Prefer the closest preceding row, then scan later rows for date-order exceptions.
    (0..from_visible)
        .rev()
        .chain(from_visible + 1..visible_len)
        .find(|&row| is_child(row))
}

pub(super) fn paged_branch_neighbor(
    commits: &[Commit],
    visible: &HistoryVisibleIndices,
    visible_ix_by_commit: &FxHashMap<CommitId, usize>,
    from: &CommitId,
    direction: i8,
    preferred_child: Option<&CommitId>,
) -> Option<usize> {
    let from_visible = *visible_ix_by_commit.get(from)?;
    let from_raw = visible.get(from_visible)?;
    if &commits.get(from_raw)?.id != from {
        return None;
    }
    // Visible parents use the existing cache. Only build a temporary hidden-ID
    // lookup when a first-parent edge actually reaches a hidden/missing commit.
    let hidden_raw_by_commit = OnceCell::<FxHashMap<&CommitId, usize>>::new();
    let parent = |raw: usize| {
        let id = commits.get(raw)?.parent_ids.first()?;
        visible_ix_by_commit
            .get(id)
            .and_then(|&row| visible.get(row))
            .or_else(|| {
                if visible.len() == commits.len() {
                    return None;
                }
                hidden_raw_by_commit
                    .get_or_init(|| {
                        commits
                            .iter()
                            .enumerate()
                            .filter(|(_, commit)| !visible_ix_by_commit.contains_key(&commit.id))
                            .map(|(raw, commit)| (&commit.id, raw))
                            .collect()
                    })
                    .get(id)
                    .copied()
            })
    };
    branch_neighbor(
        from_raw,
        from_visible,
        direction,
        preferred_child.and_then(|id| visible_ix_by_commit.get(id).copied()),
        visible.len(),
        |row| visible.get(row),
        |raw| visible.visible_ix(raw),
        parent,
    )
}

pub(super) fn indexed_branch_neighbor(
    projection: &HistoryProjection,
    children: &IndexedBranchChildren,
    from: &CommitId,
    direction: i8,
    preferred_child: Option<&CommitId>,
) -> Option<usize> {
    let from_visible = projection.position(from.as_ref())?;
    if direction == 0 {
        return None;
    }
    let mut hidden_parents = FxHashMap::default();
    let parent_of = |raw, memo: &mut FxHashMap<usize, Option<usize>>| {
        first_visible_parent(
            raw,
            &|raw| projection.visible_position(raw),
            &|raw| indexed_first_parent(projection, raw),
            memo,
        )
    };
    if direction > 0 {
        let from_raw = projection.raw_position(from_visible)?;
        return parent_of(from_raw, &mut hidden_parents).filter(|&row| row != from_visible);
    }
    if let Some(preferred) = preferred_child.and_then(|id| projection.position(id.as_ref()))
        && preferred != from_visible
        && projection
            .raw_position(preferred)
            .is_some_and(|raw| parent_of(raw, &mut hidden_parents) == Some(from_visible))
    {
        return Some(preferred);
    }
    children.child(from_visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::HistoryMode;
    use gitcomet_core::history_index::HistoryIndexBuilder;
    use gitcomet_core::services::HistorySnapshot;

    fn id(value: u8) -> CommitId {
        CommitId(format!("{value:040x}").into())
    }

    fn commit(value: u8, parents: &[u8]) -> Commit {
        Commit {
            id: id(value),
            parent_ids: parents.iter().copied().map(id).collect(),
            summary: "commit".into(),
            author: "a".into(),
            time: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    fn projection(commits: &[Commit], hidden: &[usize]) -> HistoryProjection {
        let raw_id = |id: &CommitId| {
            let mut bytes = [0u8; 20];
            bytes[19] = u8::from_str_radix(&id.as_ref()[38..], 16).unwrap();
            bytes
        };
        let mut builder = HistoryIndexBuilder::new(
            HistorySnapshot("branch-navigation".into()),
            HistoryMode::FullReachable,
            20,
        )
        .unwrap();
        for commit in commits {
            let parents: Vec<_> = commit.parent_ids.iter().map(raw_id).collect();
            builder
                .push(
                    &raw_id(&commit.id),
                    parents.iter().map(|id| id.as_slice()),
                    false,
                )
                .unwrap();
        }
        HistoryProjection::new(
            builder.finish(&CancellationToken::new()).unwrap(),
            hidden.iter().map(|&raw| raw as u32).collect(),
        )
    }

    fn assert_both(
        commits: &[Commit],
        hidden: &[usize],
        from: u8,
        direction: i8,
        preferred: Option<u8>,
        expected: Option<usize>,
    ) {
        let visible = HistoryVisibleIndices::Filtered(Arc::from(
            (0..commits.len())
                .filter(|raw| !hidden.contains(raw))
                .collect::<Vec<_>>(),
        ));
        let visible_map = visible
            .iter()
            .enumerate()
            .map(|(row, raw)| (commits[raw].id.clone(), row))
            .collect();
        let preferred = preferred.map(id);
        assert_eq!(
            paged_branch_neighbor(
                commits,
                &visible,
                &visible_map,
                &id(from),
                direction,
                preferred.as_ref()
            ),
            expected,
            "paged"
        );
        let projection = projection(commits, hidden);
        let children =
            IndexedBranchChildren::build(&projection, &CancellationToken::new()).unwrap();
        assert_eq!(
            indexed_branch_neighbor(
                &projection,
                &children,
                &id(from),
                direction,
                preferred.as_ref()
            ),
            expected,
            "indexed"
        );
    }

    #[test]
    fn shared_ancestor_prefers_the_return_path_then_the_closest_preceding_child() {
        let commits = [
            commit(1, &[3]),
            commit(2, &[3]),
            commit(3, &[4]),
            commit(4, &[]),
        ];
        assert_both(&commits, &[], 1, 1, None, Some(2));
        assert_both(&commits, &[], 3, -1, Some(1), Some(0));
        assert_both(&commits, &[], 3, -1, None, Some(1));
        assert_both(&commits, &[], 3, -1, Some(4), Some(1));
    }

    #[test]
    fn merge_navigation_never_promotes_a_second_parent() {
        let commits = [
            commit(1, &[2, 3]),
            commit(2, &[4]),
            commit(3, &[4]),
            commit(4, &[]),
        ];
        assert_both(&commits, &[], 1, 1, None, Some(1));
        assert_both(&commits, &[], 3, -1, Some(1), None);
        assert_both(&commits, &[1], 1, 1, None, Some(2));
        assert_both(&commits, &[1], 3, -1, Some(1), None);
        let missing = [commit(1, &[255, 2]), commit(2, &[])];
        assert_both(&missing, &[], 1, 1, None, None);
        assert_both(&missing, &[], 2, -1, Some(1), None);
    }

    #[test]
    fn hidden_chains_are_skipped_and_hidden_preferred_children_are_rejected() {
        let commits = [
            commit(1, &[2]),
            commit(2, &[3]),
            commit(3, &[4]),
            commit(4, &[]),
        ];
        assert_both(&commits, &[1, 2], 1, 1, None, Some(1));
        assert_both(&commits, &[1, 2], 4, -1, Some(2), Some(0));
        assert_both(&commits, &[1, 2], 2, 1, None, None);
        assert_both(&commits, &[1, 2], 4, 1, None, None);
        assert_both(&commits, &[1, 2], 1, -1, None, None);
        assert_both(&commits, &[1, 2], 1, 0, None, None);
    }

    #[test]
    fn a_child_sorted_after_its_parent_is_a_fallback_only() {
        let commits = [commit(1, &[2]), commit(2, &[]), commit(3, &[2])];
        assert_both(&commits, &[], 2, -1, None, Some(0));
        assert_both(&commits, &[], 2, -1, Some(3), Some(2));
        assert_both(&commits, &[0], 2, -1, None, Some(1));
    }

    #[test]
    fn packed_children_match_the_paged_scan_with_four_bytes_per_visible_row() {
        let commits = [
            commit(1, &[4, 5]),
            commit(2, &[4]),
            commit(3, &[2]),
            commit(4, &[6]),
            commit(5, &[6]),
            commit(6, &[]),
        ];
        for hidden in [
            &[][..],
            &[1][..],
            &[3][..],
            &[1, 3][..],
            &[0, 1, 3, 4][..],
            &[0, 1, 2, 3, 4, 5][..],
        ] {
            let visible = HistoryVisibleIndices::Filtered(Arc::from(
                (0..commits.len())
                    .filter(|raw| !hidden.contains(raw))
                    .collect::<Vec<_>>(),
            ));
            let visible_map = visible
                .iter()
                .enumerate()
                .map(|(row, raw)| (commits[raw].id.clone(), row))
                .collect();
            let projection = projection(&commits, hidden);
            let children =
                IndexedBranchChildren::build(&projection, &CancellationToken::new()).unwrap();
            assert!(children.shares_storage_with(&children.clone()));
            assert_eq!(children.bytes(), projection.len() * 4);
            assert_eq!(
                std::mem::size_of::<IndexedBranchChildren>(),
                std::mem::size_of::<Arc<[u32]>>()
            );
            for from in commits.iter().map(|commit| &commit.id) {
                for preferred in
                    std::iter::once(None).chain(commits.iter().map(|commit| Some(&commit.id)))
                {
                    for direction in [-1, 0, 1] {
                        assert_eq!(
                            indexed_branch_neighbor(
                                &projection,
                                &children,
                                from,
                                direction,
                                preferred
                            ),
                            paged_branch_neighbor(
                                &commits,
                                &visible,
                                &visible_map,
                                from,
                                direction,
                                preferred
                            ),
                            "from={from:?} direction={direction} preferred={preferred:?} hidden={hidden:?}",
                        );
                    }
                }
            }
        }
        let projection = projection(&[], &[]);
        let children =
            IndexedBranchChildren::build(&projection, &CancellationToken::new()).unwrap();
        assert_eq!(children.bytes(), 0);
    }

    #[test]
    fn indexed_children_build_rejects_cancellation_and_checks_hidden_chain_work() {
        let projection = projection(&[commit(1, &[2]), commit(2, &[])], &[]);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(IndexedBranchChildren::build(&projection, &cancellation).is_err());

        let cancellation = CancellationToken::new();
        let mut checks = 0;
        let mut check = || {
            checks += 1;
            if checks == 3 {
                cancellation.cancel();
            }
            cancellation.check_cancelled()
        };
        assert!(
            first_visible_parent_checked(
                0,
                &|raw| (raw == 20).then_some(0),
                &|raw| Some(raw + 1),
                &mut FxHashMap::default(),
                &mut check,
            )
            .is_err()
        );
        assert_eq!(checks, 3);
    }
}
