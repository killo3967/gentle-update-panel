//! Tray icon: the panel's real home. Two faces — calm and "something is pending".

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

const IDLE: &[u8] = include_bytes!("../icons/tray-idle.png");
const ALERT: &[u8] = include_bytes!("../icons/tray-alert.png");

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", crate::i18n::t("tray.open"), true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", crate::i18n::t("tray.check"), true, None::<&str>)?;
    let doctor = MenuItem::with_id(app, "doctor", crate::i18n::t("tray.doctor"), true, None::<&str>)?;
    let panel = MenuItem::with_id(app, "panel-update", crate::i18n::t("tray.updatePanel"), false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", crate::i18n::t("tray.quit"), true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&open, &check, &doctor, &panel, &separator, &quit],
    )?;

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
            "panel-update" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    // The window shows the progress; the app exits when the helper takes over.
                    show_main(&handle);
                    let _ = handle.emit("panel:applying", ());
                    if let Err(error) = crate::panel::apply(&handle).await {
                        let _ = handle.emit("panel:error", error);
                        crate::tray::reflect(&handle);
                    }
                });
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

    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut slot) = state.tray_panel_item.lock() {
            *slot = Some(panel);
        }
    }

    Ok(())
}

/// Reflects both the snapshot and the panel's own update in the tray.
pub fn reflect(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let snapshot = state.snapshot();
    let panel = state.panel_update();

    if let Ok(mut slot) = state.tray_panel_item.lock() {
        if let Some(item) = slot.as_ref() {
            let _ = item.set_enabled(panel.is_some());
            let label = match &panel {
                Some(update) => {
                    crate::i18n::t_args("tray.updatePanelTo", &[("version", &update.latest)])
                }
                None => crate::i18n::t("tray.updatePanel").to_string(),
            };
            let _ = item.set_text(label);
        }
    }

    let Some(tray) = app.tray_by_id("main") else {
        return;
    };

    let pending = snapshot
        .as_ref()
        .map(|snapshot| snapshot.needs_attention())
        .unwrap_or(0);
    let bytes = if pending > 0 || panel.is_some() { ALERT } else { IDLE };
    if let Ok(image) = Image::from_bytes(bytes) {
        let _ = tray.set_icon(Some(image));
    }

    let mut parts: Vec<String> = Vec::new();
    if pending > 0 {
        parts.push(crate::i18n::t_args(
            "tray.tooltip.pending",
            &[("count", &pending.to_string())],
        ));
    }
    if let Some(update) = &panel {
        parts.push(crate::i18n::t_args(
            "tray.tooltip.panel",
            &[("version", &update.latest)],
        ));
    }
    let tooltip = if parts.is_empty() {
        crate::i18n::t("tray.tooltip.idle").to_string()
    } else {
        format!("Gentle Update Panel · {}", parts.join(" · "))
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
