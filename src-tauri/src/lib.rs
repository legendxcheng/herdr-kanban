mod agent_status;
mod commands;
mod db;
mod file_search;
mod herdr;
mod image_commands;
mod images;
mod lifecycle;
mod models;
mod project_commands;
mod project_git;
mod projects;
mod start;
mod task_overview;
mod tasks;
mod terminal;
mod work_commands;
mod workflow;

use commands::DbState;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db_path = db::db_path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).expect("failed to create data directory");
    }
    let _database_guard =
        db::lock_database(&db_path).expect("another Kanban instance is using this database");
    let conn = Connection::open(&db_path).expect("failed to open kanban.db");
    db::init(&conn).expect("failed to init schema");

    db::recover_interrupted(&conn).expect("failed to recover dispatch operations");
    let conn = Arc::new(Mutex::new(conn));
    let workflow = Arc::new(workflow::Workflow {
        db: conn.clone(),
        herdr: Arc::new(herdr::Cli::from_env()),
    });
    let projects = Arc::new(projects::ProjectService::new(
        conn.clone(),
        workflow.herdr.clone(),
    ));
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .manage(DbState(conn))
        .manage(workflow)
        .manage(projects)
        .manage(Arc::new(terminal::TerminalManager::default()))
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                use tauri::Manager;
                window.state::<Arc<terminal::TerminalManager>>().close_all();
            }
        })
        .invoke_handler(tauri::generate_handler![
            image_commands::open_image_draft,
            image_commands::import_prompt_image,
            image_commands::preview_prompt_image,
            image_commands::close_image_draft,
            commands::list_projects,
            project_commands::create_project,
            project_commands::update_project,
            project_commands::delete_project,
            project_commands::sync_projects,
            project_commands::project_git_statuses,
            project_commands::move_project,
            project_commands::workspace_operations,
            project_commands::recover_workspace,
            project_commands::link_workspace,
            commands::list_tasks,
            commands::task_overview,
            commands::create_task,
            commands::update_task,
            commands::update_task_status,
            commands::delete_task,
            commands::archive_task,
            commands::archive_completed_tasks,
            commands::delete_completed_tasks,
            commands::search_files,
            commands::search_skills,
            commands::read_file_content,
            commands::read_skill_content,
            work_commands::herdr_list_agents,
            work_commands::start_task,
            work_commands::complete_task,
            work_commands::recover_lifecycle,
            work_commands::dispatch_to_herdr,
            work_commands::resolve_dispatch,
            work_commands::repair_binding,
            work_commands::terminal_open,
            work_commands::terminal_close,
            work_commands::terminal_input,
            work_commands::terminal_scroll,
            work_commands::terminal_resize,
            work_commands::terminal_ack,
            commands::pick_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
