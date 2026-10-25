use crate::{
    models::Project,
    projects::{CreateOperation, ProjectService, Snapshot},
    native_terminal::NativeTerminalManager as TerminalManager,
};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn project_git_statuses(
    state: State<'_, crate::commands::DbState>,
    project_ids: Vec<String>,
) -> Result<Vec<crate::project_git::ProjectGitStatus>, String> {
    let db = state.0.clone();
    run(move || {
        let requested: std::collections::HashSet<_> = project_ids.into_iter().collect();
        let projects = {
            let conn = db.lock().map_err(|e| e.to_string())?;
            crate::projects::visible_list(&conn)?
                .into_iter()
                .filter(|p| requested.contains(&p.id))
                .map(|p| (p.id, p.root_path))
                .collect()
        };
        Ok(crate::project_git::read_projects(projects))
    })
    .await
}

async fn run<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn sync_projects(service: State<'_, Arc<ProjectService>>) -> Result<Snapshot, String> {
    let service = service.inner().clone();
    run(move || service.sync()).await
}
#[tauri::command]
pub async fn move_project(
    service: State<'_, Arc<ProjectService>>,
    project_id: String,
    before_project_id: Option<String>,
) -> Result<Snapshot, String> {
    let service = service.inner().clone();
    run(move || service.move_project(&project_id, before_project_id.as_deref())).await
}
#[tauri::command]
pub async fn workspace_operations(
    service: State<'_, Arc<ProjectService>>,
) -> Result<Vec<CreateOperation>, String> {
    let service = service.inner().clone();
    run(move || service.operations()).await
}
#[tauri::command]
pub async fn create_project(
    service: State<'_, Arc<ProjectService>>,
    operation_id: String,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    let service = service.inner().clone();
    run(move || service.create(&operation_id, &name, &root_path)).await
}
#[tauri::command]
pub async fn recover_workspace(
    service: State<'_, Arc<ProjectService>>,
    operation_id: String,
    workspace_id: Option<String>,
) -> Result<Project, String> {
    let service = service.inner().clone();
    run(move || service.recover(&operation_id, workspace_id.as_deref())).await
}
#[tauri::command]
pub async fn update_project(
    service: State<'_, Arc<ProjectService>>,
    id: String,
    name: String,
    fallback_path: Option<String>,
) -> Result<Project, String> {
    let service = service.inner().clone();
    run(move || service.update(&id, &name, fallback_path.as_deref())).await
}
#[tauri::command]
pub async fn link_workspace(
    service: State<'_, Arc<ProjectService>>,
    project_id: Option<String>,
    workspace_id: String,
    fallback_path: Option<String>,
) -> Result<Project, String> {
    let service = service.inner().clone();
    run(move || {
        service.link(
            project_id.as_deref(),
            &workspace_id,
            fallback_path.as_deref(),
        )
    })
    .await
}
#[tauri::command]
pub async fn delete_project(
    service: State<'_, Arc<ProjectService>>,
    terminals: State<'_, Arc<TerminalManager>>,
    id: String,
) -> Result<(), String> {
    let service = service.inner().clone();
    let terminals = terminals.inner().clone();
    run(move || {
        for task in service.remove(&id)? {
            terminals.close_task(&task);
        }
        Ok(())
    })
    .await
}
