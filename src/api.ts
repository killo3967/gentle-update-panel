import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type {
  AppStatus,
  ApplyOutcome,
  Config,
  DoctorView,
  NotesPayload,
  PiHealth,
  PanelUpdate,
  Blocker,
  PreflightReport,
  NoteHit,
  TranslatedText,
  JournalEntry,
  Snapshot,
} from "./types";

export const api = {
  status: () => invoke<AppStatus>("app_status"),
  check: () => invoke<Snapshot>("check_now"),
  doctor: () => invoke<DoctorView>("run_doctor"),
  doctorView: () => invoke<DoctorView | null>("doctor_view"),
  piHealth: () => invoke<PiHealth>("run_pi_health"),
  applyPanelUpdate: () => invoke<void>("apply_panel_update"),
  apply: (ids: string[]) => invoke<ApplyOutcome>("apply_updates", { ids }),
  notes: (key: string, installed: string | null, latest: string | null, force = false) =>
    invoke<NotesPayload>("get_notes", { key, installed, latest, force }),
  saveConfig: (config: Config) => invoke<Config>("save_config", { config }),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  blockers: () => invoke<Blocker[]>("blockers"),
  searchNotes: (query: string) => invoke<NoteHit[]>("search_notes", { query }),
  preflight: (ids: string[]) => invoke<PreflightReport>("apply_preflight", { ids }),
  clearJournal: () => invoke<void>("clear_journal"),
  translate: (cacheKey: string, text: string, force = false) =>
    invoke<TranslatedText>("translate_release", { cacheKey, text, force }),
};

export const events = {
  snapshot: (handler: (snapshot: Snapshot) => void) =>
    listen<Snapshot>("snapshot", (event) => handler(event.payload)),
  progress: (handler: (phase: string, message: string) => void) =>
    listen<{ phase: string; message: string }>("check:progress", (event) =>
      handler(event.payload.phase, event.payload.message),
    ),
  log: (handler: (event: import("./types").LogEvent) => void) =>
    listen<import("./types").LogEvent>("apply:log", (event) => handler(event.payload)),
  step: (handler: (event: import("./types").ApplyStep) => void) =>
    listen<import("./types").ApplyStep>("apply:step", (event) => handler(event.payload)),
  applied: (handler: (outcome: ApplyOutcome) => void) =>
    listen<ApplyOutcome>("apply:done", (event) => handler(event.payload)),
  preselect: (handler: (ids: string[]) => void) =>
    listen<string[]>("preselect", (event) => handler(event.payload)),
  openDoctor: (handler: () => void) => listen("open-doctor", () => handler()),
  journal: (handler: (entry: JournalEntry) => void) =>
    listen<JournalEntry>("journal:entry", (event) => handler(event.payload)),
  panelUpdate: (handler: (update: PanelUpdate | null) => void) =>
    listen<PanelUpdate | null>("panel:update", (event) => handler(event.payload)),
  panelProgress: (handler: (received: number, total: number) => void) =>
    listen<{ phase: string; received: number; total: number }>("panel:progress", (event) =>
      handler(event.payload.received, event.payload.total),
    ),
  panelApplying: (handler: () => void) => listen("panel:applying", () => handler()),
  panelError: (handler: (message: string) => void) =>
    listen<string>("panel:error", (event) => handler(event.payload)),
};
