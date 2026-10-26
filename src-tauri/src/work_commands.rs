use crate::native_terminal::{NativeTerminalManager as TerminalManager, Output};
use crate::{models::DispatchRecord, workflow::Workflow};
use std::sync::Arc;
use tauri::{Emitter, State};

#[tauri::command]
pub async fn start_task(
    app: State<'_, Arc<Workflow>>,
    handle: tauri::AppHandle,
    task_id: String,
    operation_id: String,
) -> Result<crate::lifecycle::Flow, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app = app.for_task(&task_id)?;
        app.submit_start(
            &task_id,
            &operation_id,
            Arc::new(move |event| {
                let _ = handle.emit("task-start-changed", event);
            }),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn complete_task(
    app: State<'_, Arc<Workflow>>,
    manager: State<'_, Arc<TerminalManager>>,
    task_id: String,
    operation_id: String,
) -> Result<crate::lifecycle::Outcome, String> {
    let app = app.inner().clone();
    manager.close_task(&task_id);
    tauri::async_runtime::spawn_blocking(move || {
        app.for_task(&task_id)?
            .complete_task(&task_id, &operation_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn recover_lifecycle(
    app: State<'_, Arc<Workflow>>,
    task_id: String,
    operation_id: String,
) -> Result<(), String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.for_task(&task_id)?
            .recover_lifecycle(&task_id, &operation_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn herdr_list_agents(
    app: State<'_, Arc<Workflow>>,
    project_id: String,
) -> Result<Vec<crate::agent_status::AgentChoice>, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.for_project(&project_id)?.list_agents(&project_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn dispatch_to_herdr(
    app: State<'_, Arc<Workflow>>,
    task_id: String,
    agent_name: String,
    prompt_text: String,
    operation_id: String,
    prompt_json: Option<String>,
) -> Result<DispatchRecord, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app = app.for_task(&task_id)?;
        let saved: String = app
            .db
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT prompt_json FROM tasks WHERE id=?1",
                [&task_id],
                |r| r.get(0),
            )
            .map_err(|_| "task.missing")?;
        let document = prompt_json.as_deref().unwrap_or(&saved);
        if !crate::images::document_images(document)?.is_empty()
            || !crate::images::document_images(&saved)?.is_empty()
        {
            return app.dispatch_document(&task_id, &agent_name, document, &operation_id);
        }
        app.dispatch(&task_id, &agent_name, &prompt_text, &operation_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn resolve_dispatch(
    app: State<'_, Arc<Workflow>>,
    task_id: String,
    operation_id: String,
    sent: bool,
    evidence: String,
) -> Result<(), String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.for_task(&task_id)?
            .resolve(&task_id, &operation_id, sent, &evidence)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn repair_binding(
    app: State<'_, Arc<Workflow>>,
    manager: State<'_, Arc<TerminalManager>>,
    task_id: String,
    pane_id: String,
) -> Result<(), String> {
    let app = app.inner().clone();
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.for_task(&task_id)?.repair(&task_id, &pane_id)?;
        manager.close_task(&task_id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn terminal_open(
    window: tauri::Window,
    app: State<'_, Arc<Workflow>>,
    native: State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    task_id: String,
    connection_id: String,
    bounds: crate::native_terminal::Bounds,
    output: tauri::ipc::Channel<Output>,
) -> Result<String, String> {
    let app = app.inner().clone();
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let view = native.select(
        connection_id.clone(),
        task_id.clone(),
        bounds,
        Arc::new(move |packet| {
            let _ = output.send(packet);
        }),
    )?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::native_terminal::open(view, app.for_task(&task_id)?, hwnd)?;
        Ok(connection_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn terminal_close(
    native: State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    connection_id: String,
) {
    native.close(&connection_id);
}
#[tauri::command]
pub fn terminal_bounds(
    native: State<'_, Arc<crate::native_terminal::NativeTerminalManager>>,
    connection_id: String,
    bounds: crate::native_terminal::Bounds,
) -> Result<(), String> {
    native.bounds(&connection_id, bounds)
}
