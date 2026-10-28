//! Tray icon: the panel's real home. Two faces — calm and "something is pending".

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::domain::Snapshot;

const IDLE: &[u8] = include_bytes!("../icons/tray-idle.png");
const ALERT: &[u8] = include_bytes!("../icons/tray-alert.png");

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Abrir panel", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Comprobar ahora", true, None::<&str>)?;
    let doctor = MenuItem::with_id(app, "doctor", "Ejecutar doctor", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &check, &doctor, &separator, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(IDLE)?)
        .icon_as_template(false)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Gentle Update Panel")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "check" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    crate::service::run_check(&handle).await;
                });
            }
            "doctor" => {
                show_main(app);
                let _ = app.emit("open-doctor", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

pub fn reflect(app: &AppHandle, snapshot: &Snapshot) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };

    let pending = snapshot.needs_attention();
    let bytes = if pending > 0 { ALERT } else { IDLE };
    if let Ok(image) = Image::from_bytes(bytes) {
        let _ = tray.set_icon(Some(image));
    }

    let tooltip = if pending > 0 {
        format!("Gentle Update Panel · {pending} pendiente(s)")
    } else {
        "Gentle Update Panel · al día".to_string()
    };
    let _ = tray.set_tooltip(Some(tooltip));
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn toggle_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);

    if visible && !minimized {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
