//! Project and user skill discovery for the local execution environment.
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub skill_md_path: String,
}

/// Discover project skills before user skills. Missing roots are optional.
pub fn search_skills(query: String, root_path: Option<String>) -> Result<Vec<SkillEntry>, String> {
    let home = std::env::var("USERPROFILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|value| !value.trim().is_empty())
        });
    Ok(search_skills_in(
        root_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(Path::new),
        home.as_deref().map(Path::new),
        &query,
    ))
}

fn search_skills_in(project: Option<&Path>, home: Option<&Path>, query: &str) -> Vec<SkillEntry> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(project) = project {
        roots.push(project.join(".agents/skills"));
    }
    if let Some(home) = home {
        roots.extend([
            home.join(".agents/skills"),
            home.join(".codex/skills"),
            // Preserve discovery of legacy Claude skills and plugin skills.
            home.join(".claude"),
        ]);
    }
    let mut candidates = Vec::new();
    for (priority, root) in roots.iter().enumerate() {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file() && entry.file_name() == "SKILL.md")
        {
            let path = entry.path();
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let name = extract_frontmatter_field(&content, "name")
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| {
                    path.parent()
                        .and_then(|parent| parent.file_name())
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
            let description =
                extract_frontmatter_field(&content, "description").unwrap_or_default();
            candidates.push((
                priority,
                SkillEntry {
                    name,
                    description,
                    skill_md_path: path.to_string_lossy().replace('\\', "/"),
                },
            ));
        }
    }
    // Sort before deduplication and limiting: traversal order must not determine
    // which copy wins or hide project skills behind 30 global candidates.
    candidates.sort_by(|(a_priority, a), (b_priority, b)| {
        a_priority
            .cmp(b_priority)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.skill_md_path.cmp(&b.skill_md_path))
    });
    let q = query.to_lowercase();
    let mut names = std::collections::HashSet::new();
    let mut paths = std::collections::HashSet::new();
    candidates
        .into_iter()
        .map(|(_, skill)| skill)
        .filter(|skill| {
            let path = if cfg!(windows) {
                skill.skill_md_path.to_lowercase()
            } else {
                skill.skill_md_path.clone()
            };
            paths.insert(path) && names.insert(skill.name.to_lowercase())
        })
        .filter(|skill| {
            q.is_empty()
                || skill.name.to_lowercase().contains(&q)
                || skill.description.to_lowercase().contains(&q)
        })
        .take(30)
        .collect()
}

fn extract_frontmatter_field(content: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    content
        .lines()
        .skip_while(|l| *l != "---")
        .skip(1)
        .take_while(|l| *l != "---")
        .find(|l| l.trim_start().starts_with(&prefix))
        .map(|l| {
            l.trim_start()
                .trim_start_matches(&prefix)
                .trim()
                .trim_matches('"')
                .to_string()
        })
}

#[cfg(test)]
#[path = "skill_discovery_tests.rs"]
mod skill_discovery_tests;
