use crate::repo::oid_to_arc_str;
use gitcomet_core::domain::{Branch, CommitId};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub(crate) fn reflog_lines_rev(
    platform: &mut gix::refs::file::log::iter::Platform<'_, '_>,
    context: &str,
    limit: Option<usize>,
) -> Result<Vec<gix::refs::log::Line>> {
    if limit == Some(0) {
        return Ok(Vec::new());
    }

    let read_error = |e: std::io::Error| {
        Error::new(ErrorKind::Backend(format!("gix reflog {context}: {e}")))
            .with_source(e)
            .with_context("read Git reflog", None)
    };
    let line_error = |e| Error::new(ErrorKind::Backend(format!("gix reflog {context}: {e}")));
    let Some(iter) = platform.rev().map_err(read_error)? else {
        return Ok(Vec::new());
    };

    let limit = limit.unwrap_or(usize::MAX);
    match iter.take(limit).collect::<std::result::Result<Vec<_>, _>>() {
        Ok(lines) => return Ok(lines),
        // gix's reverse reader uses a fixed buffer and reports overflow as an untyped I/O error.
        Err(e)
            if e.iter_errors()
                .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
                .any(|io| {
                    io.kind() == std::io::ErrorKind::Other
                        && io
                            .to_string()
                            .starts_with("buffer too small for line size,")
                }) => {}
        Err(e) => return Err(line_error(e)),
    }

    // The forward reader loads the complete file into the platform's buffer.
    // Decode only the requested lines, in the same order as the reverse reader.
    if platform.all().map_err(read_error)?.is_none() {
        return Ok(Vec::new());
    }
    platform
        .buf
        .lines_with_terminator()
        .rev()
        .take(limit)
        .map(|line| {
            gix::refs::file::log::LineRef::from_bytes(line)
                .map(Into::into)
                .map_err(line_error)
        })
        .collect()
}

pub(crate) fn try_collect_loose_local_branches_fast(
    repo: &gix::Repository,
) -> Result<Option<Vec<Branch>>> {
    if crate::refs::backend(repo)? == crate::refs::RefBackend::Reftable {
        return Ok(None);
    }

    if repo.common_dir().join("packed-refs").exists() {
        return Ok(None);
    }

    let root = repo.common_dir().join("refs").join("heads");
    if !root.exists() {
        return Ok(Some(Vec::new()));
    }

    let mut branches = Vec::new();
    let mut scratch = Vec::new();
    let mut target_ids = FxHashMap::default();
    let mut last_target = None;
    if !collect_loose_local_branches_fast(
        &root,
        &root,
        &mut scratch,
        &mut target_ids,
        &mut last_target,
        &mut branches,
    )? {
        return Ok(None);
    }
    branches.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(Some(branches))
}

fn collect_loose_local_branches_fast(
    root: &Path,
    dir: &Path,
    scratch: &mut Vec<u8>,
    target_ids: &mut FxHashMap<gix::ObjectId, CommitId>,
    last_target: &mut Option<(gix::ObjectId, CommitId)>,
    branches: &mut Vec<Branch>,
) -> Result<bool> {
    for entry in std::fs::read_dir(dir).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "read refs dir {}: {e}",
            dir.display()
        )))
        .with_source(e)
        .with_context("read refs directory", Some(dir))
    })? {
        let entry = entry.map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "read refs dir entry {}: {e}",
                dir.display()
            )))
            .with_source(e)
            .with_context("read refs directory entry", Some(dir))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "read refs file type {}: {e}",
                path.display()
            )))
            .with_source(e)
            .with_context("read ref file type", Some(&path))
        })?;

        if file_type.is_dir() {
            if !collect_loose_local_branches_fast(
                root,
                &path,
                scratch,
                target_ids,
                last_target,
                branches,
            )? {
                return Ok(false);
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        scratch.clear();
        File::open(&path)
            .and_then(|mut file| file.read_to_end(scratch))
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "read branch ref {}: {e}",
                    path.display()
                )))
                .with_source(e)
                .with_context("read branch ref", Some(&path))
            })?;

        let Some(target_id) = parse_loose_ref_target_id(scratch) else {
            return Ok(false);
        };

        let relative = path.strip_prefix(root).unwrap_or(path.as_path());
        let name = path_to_git_ref_name(relative);
        let target = cached_commit_id(target_ids, last_target, target_id);
        branches.push(Branch {
            name,
            target,
            upstream: None,
            divergence: None,
        });
    }
    Ok(true)
}

pub(crate) fn parse_loose_ref_target_id(buf: &[u8]) -> Option<gix::ObjectId> {
    let trimmed = buf.strip_suffix(b"\n").unwrap_or(buf);
    let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
    if trimmed.starts_with(b"ref: ") {
        return None;
    }
    gix::ObjectId::from_hex(trimmed).ok()
}

pub(crate) fn path_to_git_ref_name(path: &Path) -> String {
    let mut name = String::new();
    for component in path.components() {
        if !name.is_empty() {
            name.push('/');
        }
        name.push_str(component.as_os_str().to_string_lossy().as_ref());
    }
    name
}

pub(crate) fn cached_commit_id(
    cache: &mut FxHashMap<gix::ObjectId, CommitId>,
    last_target: &mut Option<(gix::ObjectId, CommitId)>,
    target_id: gix::ObjectId,
) -> CommitId {
    if let Some((cached_oid, commit_id)) = last_target.as_ref()
        && *cached_oid == target_id
    {
        return commit_id.clone();
    }

    if let Some(commit_id) = cache.get(&target_id) {
        let commit_id = commit_id.clone();
        *last_target = Some((target_id, commit_id.clone()));
        return commit_id;
    }

    let commit_id = CommitId(oid_to_arc_str(&target_id));
    cache.insert(target_id, commit_id.clone());
    *last_target = Some((target_id, commit_id.clone()));
    commit_id
}
