use crate::{commands::DbState, images};
use tauri::State;

#[tauri::command]
pub async fn open_image_draft(
    state: State<'_, DbState>,
    project_id: String,
    task_id: Option<String>,
) -> Result<String, String> {
    let db = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        images::open_draft(
            &*db.lock().map_err(|e| e.to_string())?,
            &project_id,
            task_id.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn import_prompt_image(
    state: State<'_, DbState>,
    draft_id: String,
    name: String,
    mime: String,
    data: String,
) -> Result<images::PromptImage, String> {
    let db = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let decoded = images::decode_upload(&name, &mime, &data)?;
        let mut pending = {
            let connection = db.lock().map_err(|e| e.to_string())?;
            images::stage_upload(&connection, &draft_id, decoded)?
        };
        pending.write()?;
        let result = {
            let connection = db.lock().map_err(|e| e.to_string())?;
            pending.commit(&connection)
        };
        result
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn preview_prompt_image(
    state: State<'_, DbState>,
    draft_id: String,
    image_id: String,
) -> Result<String, String> {
    let db = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        images::preview(
            &*db.lock().map_err(|e| e.to_string())?,
            &draft_id,
            &image_id,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn close_image_draft(state: State<'_, DbState>, draft_id: String) -> Result<(), String> {
    let db = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        images::close_draft(&*db.lock().map_err(|e| e.to_string())?, &draft_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
