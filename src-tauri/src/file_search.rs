//! Project file discovery and deterministic relevance ranking for @ references.
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

const RESULT_LIMIT: usize = 50;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    pub rel_path: String,
    pub abs_path: String,
}

#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct Rank {
    quality: u8,
    gaps: usize,
    depth: usize,
    length: usize,
    folded_path: String,
    path: String,
}

pub fn search(root_path: String, query: String) -> Result<Vec<FileEntry>, String> {
    let root = Path::new(&root_path);
    if !root.is_dir() {
        return Err(format!("Not a directory: {root_path}"));
    }
    let normalized = query.replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");
    // Queries are relative to this project, including when explicitly looking
    // inside an ignored directory. Never turn a query into an outside root.
    if Path::new(normalized)
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Ok(Vec::new());
    }
    let scope = explicit_directory(root, normalized);
    let explicit = scope != root;
    let mut walker = WalkBuilder::new(&scope);
    walker
        .hidden(false)
        .ignore(false)
        .require_git(false)
        .parents(!explicit)
        .git_global(!explicit)
        .git_exclude(!explicit)
        .follow_links(false)
        .filter_entry(|entry| {
            // Prune directories before descent. The selected search root itself
            // is always allowed, even for a project or explicit scope named build.
            entry.depth() == 0
                || !entry.file_type().is_some_and(|kind| kind.is_dir())
                || !noise_directory(&entry.file_name().to_string_lossy())
        });

    let q = normalized.to_lowercase();
    // Inspect every eligible file, retaining only the best 50 in memory. Directory
    // enumeration order can never crowd a better match out of the final results.
    let mut best = BTreeMap::new();
    for entry in walker.build().filter_map(Result::ok) {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let folded = relative.to_lowercase();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let Some((quality, gaps)) = match_quality(&name, &folded, &q) else {
            continue;
        };
        let rank = Rank {
            quality,
            gaps,
            depth: relative.matches('/').count(),
            length: relative.chars().count(),
            folded_path: folded,
            path: relative.clone(),
        };
        best.insert(
            rank,
            FileEntry {
                rel_path: relative,
                abs_path: path.to_string_lossy().replace('\\', "/"),
            },
        );
        if best.len() > RESULT_LIMIT {
            best.pop_last();
        }
    }
    Ok(best.into_values().collect())
}

fn noise_directory(name: &str) -> bool {
    [
        ".scratch",
        ".git",
        "node_modules",
        "target",
        "__pycache__",
        ".venv",
        "dist",
        "build",
    ]
    .iter()
    .any(|noise| name.eq_ignore_ascii_case(noise))
}

/// An existing directory prefix explicitly scopes the search, including ignored
/// paths such as .scratch/probe/. Ancestor ignore rules are relaxed only for that
/// subtree; ignore files inside it and nested noise directories still apply.
fn explicit_directory(root: &Path, query: &str) -> PathBuf {
    let Some((prefix, _)) = query.rsplit_once('/') else {
        return root.to_path_buf();
    };
    let mut scope = root.to_path_buf();
    for part in Path::new(prefix).components() {
        let Component::Normal(name) = part else {
            return root.to_path_buf();
        };
        let next = scope.join(name);
        let Ok(metadata) = std::fs::symlink_metadata(&next) else {
            break;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            break;
        }
        scope = next;
    }
    scope
}

fn match_quality(name: &str, path: &str, query: &str) -> Option<(u8, usize)> {
    if query.is_empty() || name == query || path == query {
        Some((0, 0))
    } else if name.starts_with(query) {
        Some((1, 0))
    } else if name.contains(query) {
        Some((2, 0))
    } else if path.contains(query) {
        Some((3, 0))
    } else if let Some(gaps) = subsequence_gaps(name, query) {
        Some((4, gaps))
    } else {
        subsequence_gaps(path, query).map(|gaps| (5, gaps))
    }
}

fn subsequence_gaps(text: &str, query: &str) -> Option<usize> {
    let mut remaining = query.chars();
    let mut wanted = remaining.next()?;
    let mut matched = 0;
    for (position, ch) in text.chars().enumerate() {
        if ch == wanted {
            matched += 1;
            match remaining.next() {
                Some(next) => wanted = next,
                None => return Some(position + 1 - matched),
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "file_search_tests.rs"]
mod file_search_tests;
