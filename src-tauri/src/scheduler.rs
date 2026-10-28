//! Periodic background checks. The interval is read on every tick so changing
//! it in the UI takes effect without restarting the app.

use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::state::AppState;

const TICK: Duration = Duration::from_secs(20);
const STARTUP_DELAY: Duration = Duration::from_secs(8);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        let mut last_run: Option<Instant> = None;

        loop {
            let interval_minutes = app.state::<AppState>().config().interval_minutes;
            let due = interval_minutes > 0
                && match last_run {
                    None => true,
                    Some(at) => at.elapsed() >= Duration::from_secs(interval_minutes * 60),
                };

            if due {
                crate::service::run_check(&app).await;
                last_run = Some(Instant::now());
            }

            tokio::time::sleep(TICK).await;
        }
    });
}
