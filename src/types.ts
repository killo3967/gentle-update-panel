// Shapes returned by the Rust core (serde camelCase).

export type ItemKind = "ecosystem" | "pi" | "piPackage";
export type Status = "upToDate" | "updateAvailable" | "missing" | "unknown";
export type Automation = "notify" | "confirm" | "auto";
export type TranslationEngine = "auto" | "google" | "microsoft";

export type Upgrade =
  | { type: "gentleAiTool"; tool: string }
  | { type: "piSelf" }
  | { type: "piPackage"; source: string }
  | { type: "manual"; hint: string };

export interface UpdateItem {
  id: string;
  kind: ItemKind;
  name: string;
  source: string | null;
  installed: string | null;
  latest: string | null;
  status: Status;
  detail: string | null;
  upgrade: Upgrade;
  notesKey: string | null;
  ignored: boolean;
}

export interface Snapshot {
  checkedAt: string;
  durationMs: number;
  items: UpdateItem[];
  warnings: string[];
}

export interface Config {
  intervalMinutes: number;
  automation: Automation;
  notifyOnUpdate: boolean;
  doctorAfterUpdate: boolean;
  startMinimized: boolean;
  lastView: string;
  lastSelectedId: string | null;
  ignored: string[];
  doctorIgnored: string[];
  ignoreMissingTools: boolean;
  translationEnabled: boolean;
  translationEngine: TranslationEngine;
  translationLang: string;
}

export interface TranslatedText {
  text: string;
  engine: string;
  cached: boolean;
}

export interface HealthCheck {
  id: string;
  marker: "ok" | "warn" | "fail";
  message: string;
  remedy: string | null;
}

export interface PackageHealth {
  id: string;
  name: string;
  source: string;
  path: string;
  checks: HealthCheck[];
  status: string;
}

export interface PiHealth {
  checkedAt: string;
  status: string;
  version: string | null;
  checks: HealthCheck[];
  packages: PackageHealth[];
}

export interface AppStatus {
  config: Config;
  snapshot: Snapshot | null;
  doctor: DoctorView | null;
  piHealth: PiHealth | null;
  journal: JournalEntry[];
  journalPath: string;
  autostart: boolean;
  version: string;
  appDir: string;
}

export interface DoctorCheck {
  id: string;
  marker: string;
  message: string;
  remedy: string | null;
}

export interface DoctorCheckView {
  id: string;
  marker: string;
  message: string;
  remedy: string | null;
  ignored: boolean;
}

export interface DoctorView {
  checks: DoctorCheckView[];
  ignoredCount: number;
  status: string;
  originalStatus: string;
  summary: string | null;
  raw: string;
}

export interface Release {
  version: string;
  date: string | null;
  body: string;
}

export interface NotesPayload {
  key: string;
  title: string;
  releases: Release[];
  url: string | null;
  fetchedAt: string;
  error: string | null;
}

export interface NoteHit {
  notesKey: string;
  version: string;
  title: string;
  snippet: string;
}

export interface ApplyFailure {
  id: string;
  label: string;
  reason: string;
  output: string;
}

export interface ApplyOutcome {
  applied: string[];
  failed: ApplyFailure[];
  doctor: DoctorView | null;
  snapshot: Snapshot | null;
}

export type Impact = "none" | "warn" | "block";

export interface Blocker {
  name: string;
  pid: number;
  kind: "pi" | "engram" | "gentle-ai" | "gga" | "codegraph";
  command: string;
}

export interface PreflightStep {
  id: string;
  label: string;
  impact: Impact;
  reason: string | null;
}

export interface PreflightReport {
  blockers: Blocker[];
  steps: PreflightStep[];
}

export interface LogEvent {
  id: string;
  stream: string;
  line: string;
}

export interface ApplyStep {
  id: string;
  label: string;
  command: string;
  index: number;
  total: number;
}

// ---------------------------------------------------------------------------
// Registro
// ---------------------------------------------------------------------------

export type JournalKind =
  | "app"
  | "check"
  | "detected"
  | "preflight"
  | "apply"
  | "step"
  | "output"
  | "applied"
  | "failed"
  | "doctor";

/** One line of the durable journal (`journal.ndjson`), oldest first when listed. */
export interface JournalEntry {
  seq: number;
  at: string;
  kind: JournalKind;
  title: string;
  detail: string | null;
  itemId: string | null;
  stream: string | null;
}
