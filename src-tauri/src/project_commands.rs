use crate::{
    models::Project,
    native_terminal::NativeTerminalManager as TerminalManager,
    projects::{CreateOperation, ProjectService, Snapshot},
};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn workspace_choices(
    service: State<'_, Arc<ProjectService>>,
    machine_id: String,
) -> Result<crate::projects::WorkspaceChoices, String> {
    let service = service.inner().clone();
    run(move || service.for_machine(&machine_id)?.choices()).await
}
#[tauri::command]
pub async fn add_existing_workspaces(
    service: State<'_, Arc<ProjectService>>,
    machine_id: String,
    session: String,
    workspace_ids: Vec<String>,
) -> Result<(), String> {
    let service = service.inner().clone();
    run(move || {
        service
            .for_machine(&machine_id)?
            .add_existing(&session, &workspace_ids)
    })
    .await
}
#[tauri::command]
pub fn hide_project(db: State<'_, crate::commands::DbState>, id: String) -> Result<(), String> {
    crate::project_visibility::hide(&*db.0.lock().map_err(|e| e.to_string())?, &id)
}
#[tauri::command]
pub fn navigation_state(
    db: State<'_, crate::commands::DbState>,
) -> Result<crate::project_visibility::NavigationState, String> {
    crate::project_visibility::state(&*db.0.lock().map_err(|e| e.to_string())?)
}

#[tauri::command]
pub async fn project_git_statuses(
    app: State<'_, Arc<crate::workflow::Workflow>>,
    state: State<'_, crate::commands::DbState>,
    project_ids: Vec<String>,
) -> Result<Vec<crate::project_git::ProjectGitStatus>, String> {
    let db = state.0.clone();
    let app = app.inner().clone();
    run(move || {
        let requested: std::collections::HashSet<_> = project_ids.into_iter().collect();
        let projects = {
            let conn = db.lock().map_err(|e| e.to_string())?;
            crate::projects::visible_list(&conn)?
                .into_iter()
                .filter(|p| requested.contains(&p.id))
                .collect::<Vec<_>>()
        };
        let mut groups = std::collections::HashMap::<String, Vec<(String, String)>>::new();
        for project in projects {
            groups
                .entry(project.machine_id)
                .or_default()
                .push((project.id, project.root_path));
        }
        let mut results = Vec::new();
        for (machine, projects) in groups {
            match crate::machine_routing::adapter(&db, &app.herdr, &machine)
                .and_then(|adapter| adapter.resources().git_statuses(projects.clone()))
            {
                Ok(statuses) => results.extend(statuses),
                Err(_) => results.extend(projects.into_iter().map(|(project_id, root_path)| {
                    crate::project_git::ProjectGitStatus {
                        project_id,
                        root_path,
                        status: crate::project_git::GitStatus {
                            state: crate::project_git::GitState::Unavailable,
                            head: None,
                        },
                    }
                })),
            }
        }
        Ok(results)
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
pub async fn sync_projects(
    service: State<'_, Arc<ProjectService>>,
    machine_id: Option<String>,
) -> Result<Snapshot, String> {
    let service = service.inner().clone();
    run(move || {
        service
            .for_machine(machine_id.as_deref().unwrap_or(crate::machines::LOCAL_ID))?
            .sync()
    })
    .await
}
#[tauri::command]
pub async fn move_project(
    service: State<'_, Arc<ProjectService>>,
    project_id: String,
    before_project_id: Option<String>,
) -> Result<Snapshot, String> {
    let service = service.inner().clone();
    run(move || {
        service
            .for_project(&project_id)?
            .move_project(&project_id, before_project_id.as_deref())
    })
    .await
}
#[tauri::command]
pub async fn workspace_operations(
    service: State<'_, Arc<ProjectService>>,
    machine_id: Option<String>,
) -> Result<Vec<CreateOperation>, String> {
    let service = service.inner().clone();
    run(move || {
        if machine_id
            .as_deref()
            .is_none_or(|id| id == crate::machines::LOCAL_ID)
        {
            return service.operations();
        }
        let db = service.db.lock().map_err(|e| e.to_string())?;
        let machine = crate::machines::get(
            &db,
            machine_id.as_deref().unwrap_or(crate::machines::LOCAL_ID),
        )?;
        crate::projects::operations(&db, &machine.id, &machine.session)
    })
    .await
}
#[tauri::command]
pub async fn create_project(
    service: State<'_, Arc<ProjectService>>,
    machine_id: Option<String>,
    operation_id: String,
    name: String,
    root_path: String,
) -> Result<Project, String> {
    let service = service.inner().clone();
    run(move || {
        service
            .for_machine(machine_id.as_deref().unwrap_or(crate::machines::LOCAL_ID))?
            .create(&operation_id, &name, &root_path)
    })
    .await
}
#[tauri::command]
pub async fn recover_workspace(
    service: State<'_, Arc<ProjectService>>,
    operation_id: String,
    workspace_id: Option<String>,
) -> Result<Project, String> {
    let service = service.inner().clone();
    run(move || {
        service
            .for_operation(&operation_id)?
            .recover(&operation_id, workspace_id.as_deref())
    })
    .await
}
#[tauri::command]
pub async fn update_project(
    service: State<'_, Arc<ProjectService>>,
    id: String,
    name: String,
    fallback_path: Option<String>,
) -> Result<Project, String> {
    let service = service.inner().clone();
    run(move || {
        service
            .for_project(&id)?
            .update(&id, &name, fallback_path.as_deref())
    })
    .await
}
#[tauri::command]
pub async fn link_workspace(
    service: State<'_, Arc<ProjectService>>,
    machine_id: Option<String>,
    project_id: Option<String>,
    workspace_id: String,
    fallback_path: Option<String>,
) -> Result<Project, String> {
    let service = service.inner().clone();
    run(move || {
        let service = match &project_id {
            Some(id) => service.for_project(id)?,
            None => {
                service.for_machine(machine_id.as_deref().unwrap_or(crate::machines::LOCAL_ID))?
            }
        };
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
