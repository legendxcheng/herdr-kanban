mod agent_status;
mod commands;
mod db;
mod execution;
mod file_search;
mod herdr;
mod image_commands;
mod images;
mod lifecycle;
mod machine_commands;
mod machine_enrollment;
mod machine_routing;
mod machines;
#[cfg(test)]
mod remote_acceptance;
mod remote_helper;
mod remote_images;
mod remote_install;
mod remote_resources;
mod remote_transport;

pub fn remote_helper_main() -> i32 {
    remote_helper::run()
}

pub fn remote_bridge_main() -> i32 {
    remote_helper::bridge()
}
mod models;
#[cfg(windows)]
mod native_ssh;
mod native_terminal;
mod project_commands;
mod project_visibility;
mod project_git;
mod projects;
mod skill_discovery;
mod start;
mod task_overview;
mod task_summary;
mod task_titles;
mod tasks;
#[cfg(test)]
mod terminal;
mod title_cli;
mod title_process;
mod title_prompt;
mod work_commands;
mod workflow;

use commands::DbState;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

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
        .manage(DbState(conn.clone()))
        .setup(move |app| {
            let handle = app.handle().clone();
            app.manage(task_titles::TaskTitles::new(
                conn.clone(),
                Arc::new(title_cli::CodexTitleGenerator::from_env()),
                Arc::new(move |project_id| {
                    let _ = handle.emit(
                        "task-title-changed",
                        serde_json::json!({"project_id":project_id}),
                    );
                }),
            ));
            Ok(())
        })
        .manage(workflow)
        .manage(projects)
        .manage(Arc::new(native_terminal::NativeTerminalManager::default()))
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                use tauri::Manager;
                window
                    .state::<Arc<native_terminal::NativeTerminalManager>>()
                    .close_all();
                window.state::<task_titles::TaskTitles>().shutdown();
            }
        })
        .invoke_handler(tauri::generate_handler![
            machine_commands::list_machines,
            machine_commands::enroll_machine,
            machine_commands::enrollment_status,
            machine_commands::saved_machines,
            machine_commands::connect_machine,
            machine_commands::rename_machine,
            machine_commands::remove_machine,
            machine_commands::machine_status,
            image_commands::open_image_draft,
            image_commands::import_prompt_image,
            image_commands::preview_prompt_image,
            image_commands::close_image_draft,
            commands::list_projects,
            project_commands::create_project,
            project_commands::workspace_choices,
            project_commands::add_existing_workspaces,
            project_commands::hide_project,
            project_commands::navigation_state,
            project_commands::update_project,
            project_commands::delete_project,
            project_commands::sync_projects,
            project_commands::project_git_statuses,
            project_commands::move_project,
            project_commands::workspace_operations,
            project_commands::recover_workspace,
            project_commands::link_workspace,
            commands::list_tasks,
            commands::task_summary,
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
            work_commands::terminal_bounds,
            commands::pick_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
