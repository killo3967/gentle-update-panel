mod blockers;
mod cache;
mod cli;
mod commands;
mod doctor;
mod domain;
mod health;
mod journal;
mod locale;
mod notes;
mod parsers;
mod probe;
mod scheduler;
mod service;
mod state;
mod translate;
mod tray;
mod upgrade;

use tauri::{Manager, WindowEvent};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_main(app);
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // Visibility belongs to the tray logic, not to the plugin: restoring a
                // stale `visible: false` would start the app invisible with no way back.
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(state::AppState::new(&handle));
            tray::build(&handle)?;

            journal::record(
                &handle,
                journal::Draft::new(
                    journal::kind::APP,
                    format!(
                        "Panel iniciado · v{}",
                        app.package_info().version
                    ),
                ),
            );

            // Autostart launches with --minimized; a manual launch shows the panel.
            let minimized = std::env::args().any(|arg| arg == "--minimized");
            if !minimized {
                tray::show_main(&handle);
            }

            scheduler::spawn(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // The X button parks the panel in the tray; quitting lives in the tray menu.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::save_config,
            commands::check_now,
            commands::run_doctor,
            commands::clear_journal,
            commands::doctor_view,
            commands::run_pi_health,
            commands::apply_updates,
            commands::get_notes,
            commands::set_autostart,
            commands::open_url,
            commands::search_notes,
            commands::translate_release,
            commands::blockers,
            commands::apply_preflight,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Gentle Update Panel");
}
