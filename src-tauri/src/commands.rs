use crate::models::{Project, Task, TaskPriority};
use chrono::Utc;
use rfd;
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::State;
use uuid::Uuid;
use walkdir::WalkDir;

pub struct DbState(pub Arc<Mutex<Connection>>);

// ── Projects ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_projects(state: State<DbState>) -> Result<Vec<Project>, String> {
    crate::projects::visible_list(&*state.0.lock().map_err(|e| e.to_string())?)
}

// ── Tasks ─────────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_tasks(state: State<DbState>, project_id: String) -> Result<Vec<Task>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    crate::tasks::list(&conn, &project_id)
}

#[tauri::command]
pub async fn task_overview(
    state: State<'_, DbState>,
    project_ids: Vec<String>,
    day_start: String,
    day_end: String,
) -> Result<Vec<crate::task_overview::ProjectCounts>, String> {
    let db = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        crate::task_overview::read(&conn, &project_ids, &day_start, &day_end)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn create_task(
    titles: State<crate::task_titles::TaskTitles>,
    project_id: String,
    name: Option<String>,
    priority: TaskPriority,
    prompt_json: Option<String>,
    draft_id: Option<String>,
    locale: Option<String>,
) -> Result<Task, String> {
    titles.create(crate::task_titles::NewTask {
        project_id,
        name,
        priority,
        prompt_json: prompt_json.unwrap_or_else(|| "{}".into()),
        draft_id,
        locale: locale.unwrap_or_else(|| "zh-CN".into()),
    })
}

#[tauri::command]
pub fn update_task(
    titles: State<crate::task_titles::TaskTitles>,
    id: String,
    name: Option<String>,
    prompt_json: String,
    priority: TaskPriority,
    draft_id: Option<String>,
) -> Result<(), String> {
    titles.update(
        &id,
        name.as_deref(),
        &prompt_json,
        priority,
        draft_id.as_deref(),
    )
}

#[tauri::command]
pub async fn update_task_status(
    state: State<'_, DbState>,
    app: State<'_, Arc<crate::workflow::Workflow>>,
    terminals: State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    id: String,
    status: String,
) -> Result<crate::lifecycle::Outcome, String> {
    if status == "done" {
        let app = app.inner().clone();
        terminals.close_task(&id);
        return tauri::async_runtime::spawn_blocking(move || {
            app.complete_task(&id, &Uuid::new_v4().to_string())
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    crate::lifecycle::ensure_idle(&conn, &id, None)?;
    if !matches!(status.as_str(), "todo" | "done") {
        return Err("dispatch.required".into());
    }
    if crate::workflow::binding(&conn, &id)?.operation.is_some() {
        return Err("dispatch.verify_required".into());
    }
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE tasks SET status=?1, completed_at=NULL, updated_at=?2 WHERE id=?3",
        params![status, now, id],
    )
    .map_err(|e| e.to_string())?;
    drop(conn);
    terminals.close_task(&id);
    Ok(crate::lifecycle::Outcome::default())
}

#[tauri::command]
pub fn archive_task(
    state: State<DbState>,
    terminals: State<Arc<crate::native_terminal::NativeTerminalManager>>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    crate::tasks::archive(&mut conn, &id)?;
    drop(conn);
    terminals.close_task(&id);
    Ok(())
}

#[tauri::command]
pub fn delete_task(
    state: State<DbState>,
    terminals: State<Arc<crate::native_terminal::NativeTerminalManager>>,
    id: String,
) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    crate::tasks::delete(&mut conn, &id)?;
    drop(conn);
    terminals.close_task(&id);
    Ok(())
}

#[tauri::command]
pub async fn archive_completed_tasks(
    state: State<'_, DbState>,
    terminals: State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    project_id: String,
    task_ids: Vec<String>,
) -> Result<crate::tasks::BatchTaskResult, String> {
    remove_completed_tasks(
        state.0.clone(),
        terminals.inner().clone(),
        project_id,
        task_ids,
        crate::tasks::CompletedTaskAction::Archive,
    )
    .await
}

#[tauri::command]
pub async fn delete_completed_tasks(
    state: State<'_, DbState>,
    terminals: State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    project_id: String,
    task_ids: Vec<String>,
) -> Result<crate::tasks::BatchTaskResult, String> {
    remove_completed_tasks(
        state.0.clone(),
        terminals.inner().clone(),
        project_id,
        task_ids,
        crate::tasks::CompletedTaskAction::Delete,
    )
    .await
}

async fn remove_completed_tasks(
    db: Arc<Mutex<Connection>>,
    terminals: Arc<crate::native_terminal::NativeTerminalManager>,
    project_id: String,
    task_ids: Vec<String>,
    action: crate::tasks::CompletedTaskAction,
) -> Result<crate::tasks::BatchTaskResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db.lock().map_err(|e| e.to_string())?;
        let result = crate::tasks::remove_completed(&mut conn, &project_id, &task_ids, action);
        drop(conn);
        for id in &result.succeeded {
            terminals.close_task(id);
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── File & Skill Discovery ────────────────────────────────────────────────────

#[tauri::command]
pub async fn search_files(
    root_path: String,
    query: String,
) -> Result<Vec<crate::file_search::FileEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::file_search::search(root_path, query))
        .await
        .map_err(|error| error.to_string())?
}

#[derive(serde::Serialize)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub skill_md_path: String,
}

/// Discover project skills before user skills. Missing roots are optional.
#[tauri::command]
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

// ── Read file contents (for @ expansion preview) ──────────────────────────────

#[tauri::command]
pub fn read_file_content(abs_path: String) -> Result<String, String> {
    std::fs::read_to_string(&abs_path).map_err(|e| format!("{abs_path}: {e}"))
}

#[tauri::command]
pub fn read_skill_content(skill_md_path: String) -> Result<String, String> {
    std::fs::read_to_string(&skill_md_path).map_err(|e| format!("{skill_md_path}: {e}"))
}

// ── Herdr integration ─────────────────────────────────────────────────────────

// ── ContextSwitcher integration ───────────────────────────────────────────────

/// Open a native folder picker. Returns the chosen absolute path, or null if cancelled.
#[tauri::command]
pub fn pick_folder(title: Option<String>) -> Result<Option<String>, String> {
    let folder = rfd::FileDialog::new()
        .set_title(title.as_deref().unwrap_or("选择本地目录"))
        .pick_folder();
    Ok(folder.map(|p| p.to_string_lossy().replace('\\', "/")))
}
