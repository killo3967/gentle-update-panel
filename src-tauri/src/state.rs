//! Durable state: configuration, last snapshot and notes cache.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};

use crate::domain::{Config, DoctorReport, Snapshot};
use crate::health::PiHealth;
use crate::translate::TranslationService;
use crate::notes::NotesService;

pub struct AppState {
    pub config: Mutex<Config>,
    pub snapshot: Mutex<Option<Snapshot>>,
    pub doctor: Mutex<Option<DoctorReport>>,
    pub pi_health: Mutex<Option<PiHealth>>,
    pub notes: Arc<NotesService>,
    pub translations: Arc<TranslationService>,
    pub config_path: PathBuf,
    pub snapshot_path: PathBuf,
    pub doctor_path: PathBuf,
    pub pi_health_path: PathBuf,
    pub applying: AtomicBool,
    /// Serializes checks and applies: one writer, no reader mid-write.
    pub work_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub fn new(app: &AppHandle) -> Self {
        let directory = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."));

        let config_path = directory.join("config.json");
        let snapshot_path = directory.join("snapshot.json");
        let doctor_path = directory.join("doctor.json");
        let pi_health_path = directory.join("pi-health.json");
        let notes_path = directory.join("notes-cache.json");
        let translations_path = directory.join("translations.json");

        let config = read_json::<Config>(&config_path).unwrap_or_default();
        let mut snapshot = read_json::<Snapshot>(&snapshot_path);
        if let Some(stored) = snapshot.as_mut() {
            // The snapshot on disk may predate a config edit made outside the app.
            stored.apply_ignored(&config.ignored, config.ignore_missing_tools);
        }
        let doctor = read_json::<DoctorReport>(&doctor_path);
        let pi_health = read_json::<PiHealth>(&pi_health_path);

        Self {
            config: Mutex::new(config),
            snapshot: Mutex::new(snapshot),
            doctor: Mutex::new(doctor),
            pi_health: Mutex::new(pi_health),
            notes: Arc::new(NotesService::new(notes_path)),
            translations: Arc::new(TranslationService::new(translations_path)),
            config_path,
            snapshot_path,
            doctor_path,
            pi_health_path,
            applying: AtomicBool::new(false),
            work_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub fn config(&self) -> Config {
        self.config
            .lock()
            .map(|config| config.clone())
            .unwrap_or_default()
    }

    pub fn save_config(&self, config: &Config) {
        let _ = write_json(&self.config_path, config);
    }

    pub fn store_snapshot(&self, snapshot: &Snapshot) {
        if let Ok(mut slot) = self.snapshot.lock() {
            *slot = Some(snapshot.clone());
        }
        let _ = write_json(&self.snapshot_path, snapshot);
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.snapshot.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn store_doctor(&self, report: &DoctorReport) {
        if let Ok(mut slot) = self.doctor.lock() {
            *slot = Some(report.clone());
        }
        let _ = write_json(&self.doctor_path, report);
    }

    pub fn doctor(&self) -> Option<DoctorReport> {
        self.doctor.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn store_pi_health(&self, report: &PiHealth) {
        if let Ok(mut slot) = self.pi_health.lock() {
            *slot = Some(report.clone());
        }
        let _ = write_json(&self.pi_health_path, report);
    }

    pub fn pi_health(&self) -> Option<PiHealth> {
        self.pi_health.lock().ok().and_then(|slot| slot.clone())
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Option<T> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_json<T: serde::Serialize>(path: &PathBuf, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    // Write beside the target first so a crash never leaves a half file.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, raw)?;
    std::fs::rename(temporary, path)
}
