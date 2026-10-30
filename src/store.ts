// One tiny store: mutate through actions, re-render on every change.

import { api, events } from "./api";
import type {
  AppStatus,
  ApplyOutcome,
  ApplyStep,
  Config,
  DoctorView,
  JournalEntry,
  LogEvent,
  PiHealth,
  PanelUpdate,
  NotesPayload,
  Snapshot,
  UpdateItem,
  Blocker,
  PreflightReport,
  NoteHit,
  Release,
} from "./types";
import { isActionable, journalDetail, journalTitle } from "./ui/format";
import { setLocale, t, tn, translateError } from "./i18n";

export type View = "overview" | "doctor" | "log" | "settings";

/** Chips of the Registro view. */
export type JournalFilter = "all" | "detected" | "updates" | "checks" | "problems";

/** Same capacity as the Rust ring buffer; the UI never shows more than this. */
const JOURNAL_CAPACITY = 800;

export interface Toast {
  kind: "ok" | "error" | "info";
  text: string;
}

export interface State {
  booted: boolean;
  config: Config | null;
  snapshot: Snapshot | null;
  autostart: boolean;
  version: string;
  appDir: string;
  view: View;
  selectedId: string | null;
  selection: Set<string>;
  checking: boolean;
  progress: string;
  doctor: DoctorView | null;
  doctorRunning: boolean;
  piHealth: PiHealth | null;
  piHealthRunning: boolean;
  panelUpdate: PanelUpdate | null;
  /** Set while the installer is downloading and the helper is being started. */
  panelBusy: boolean;
  panelReceived: number;
  panelTotal: number;
  notes: Record<string, NotesPayload>;
  notesLoading: Record<string, boolean>;
  translations: Record<string, { text: string; engine: string }>;
  translationBusy: Record<string, boolean>;
  translationError: Record<string, string>;
  applying: boolean;
  log: LogEvent[];
  step: ApplyStep | null;
  outcome: ApplyOutcome | null;
  journal: JournalEntry[];
  journalPath: string;
  journalFilter: JournalFilter;
  journalQuery: string;
  /** First click arms the wipe, the second one clears the journal. */
  journalClearArmed: boolean;
  blockers: Blocker[];
  releaseIndex: Record<string, number>;
  pendingRelease: Record<string, string>;
  notesSearch: { query: string; hits: NoteHit[]; busy: boolean } | null;
  contextMenu: { x: number; y: number; href: string } | null;
  preflight: PreflightReport | null;
  preflightSelected: Set<string>;
  toast: Toast | null;
  warningsOpen: boolean;
}

export const state: State = {
  booted: false,
  config: null,
  snapshot: null,
  autostart: false,
  version: "",
  appDir: "",
  view: "overview",
  selectedId: null,
  selection: new Set<string>(),
  checking: false,
  progress: "",
  doctor: null,
  doctorRunning: false,
  piHealth: null,
  piHealthRunning: false,
  panelUpdate: null,
  panelBusy: false,
  panelReceived: 0,
  panelTotal: 0,
  notes: {},
  notesLoading: {},
  translations: {},
  translationBusy: {},
  translationError: {},
  applying: false,
  log: [],
  step: null,
  outcome: null,
  journal: [],
  journalPath: "",
  journalFilter: "all",
  journalQuery: "",
  journalClearArmed: false,
  blockers: [],
  releaseIndex: {},
  pendingRelease: {},
  notesSearch: null,
  contextMenu: null,
  preflight: null,
  preflightSelected: new Set<string>(),
  toast: null,
  warningsOpen: false,
};

type Listener = () => void;
const listeners = new Set<Listener>();

export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function setState(patch: Partial<State>): void {
  Object.assign(state, patch);
  listeners.forEach((listener) => listener());
}

// ---------------------------------------------------------------------------
// Derived
// ---------------------------------------------------------------------------

export function items(): UpdateItem[] {
  return state.snapshot?.items ?? [];
}

export function actionableItems(): UpdateItem[] {
  return items().filter((item) => !item.ignored && isActionable(item.status));
}

export function selectedIds(): string[] {
  return actionableItems()
    .filter((item) => state.selection.has(item.id))
    .map((item) => item.id);
}

export function updatesCount(): number {
  return items().filter((item) => !item.ignored && item.status === "updateAvailable")
    .length;
}

export function pendingCount(): number {
  return actionableItems().length;
}

// ---------------------------------------------------------------------------
// Registro
// ---------------------------------------------------------------------------

const JOURNAL_KINDS: Record<JournalFilter, JournalEntry["kind"][] | null> = {
  all: null,
  detected: ["detected", "panel"],
  updates: ["apply", "step", "output", "applied", "failed", "panel"],
  checks: ["check", "doctor"],
  problems: ["failed", "preflight"],
};

/** Keeps sequence order, drops duplicates and trims to the stored capacity. */
function mergeJournal(...lists: JournalEntry[][]): JournalEntry[] {
  const bySeq = new Map<number, JournalEntry>();
  for (const list of lists) {
    for (const entry of list) bySeq.set(entry.seq, entry);
  }
  return [...bySeq.values()]
    .sort((left, right) => left.seq - right.seq)
    .slice(-JOURNAL_CAPACITY);
}

export function appendJournal(entry: JournalEntry): void {
  if (state.journal.some((known) => known.seq === entry.seq)) return;
  setState({ journal: mergeJournal(state.journal, [entry]) });
}

/** Newest first, already filtered: the view and the copy share one order. */
export function journalEntries(): JournalEntry[] {
  const kinds = JOURNAL_KINDS[state.journalFilter];
  const query = state.journalQuery.trim().toLowerCase();

  return [...state.journal].reverse().filter((entry) => {
    if (kinds && !kinds.includes(entry.kind)) return false;
    if (query.length === 0) return true;
    const haystack = `${journalTitle(entry)} ${journalDetail(entry) ?? ""} ${entry.itemId ?? ""}`;
    return haystack.toLowerCase().includes(query);
  });
}

/** Plain text for the clipboard, oldest first.
 */
export function journalText(): string {
  return state.journal
    .map((entry) => {
      const detail = journalDetail(entry);
      return `${entry.at}  ${entry.kind}  ${journalTitle(entry)}${detail ? ` — ${detail}` : ""}`;
    })
    .join("\n");
}

export function setJournalFilter(filter: JournalFilter): void {
  if (state.journalFilter === filter) return;
  setState({ journalFilter: filter, journalClearArmed: false });
}

export function setJournalQuery(query: string): void {
  setState({ journalQuery: query.trim(), journalClearArmed: false });
}

export function resetJournalQuery(): void {
  setState({ journalQuery: "" });
}

/** First click arms, second click wipes: no dialog, no silent data loss. */
export async function clearJournal(): Promise<void> {
  if (!state.journalClearArmed) {
    setState({ journalClearArmed: true });
    return;
  }

  setState({ journalClearArmed: false });
  try {
    await api.clearJournal();
    setState({ journal: [] });
    toast(t("toast.journalCleared"), "ok");
  } catch (error) {
    toast(t("toast.journalClearFailed", { error: translateError(error) }), "error");
  }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

let toastTimer: number | undefined;

export function toast(text: string, kind: Toast["kind"] = "info"): void {
  setState({ toast: { text, kind } });
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => setState({ toast: null }), 4200);
}

let initialized = false;
const unlisteners: Array<() => void> = [];

export async function init(): Promise<void> {
  if (initialized) return;
  initialized = true;

  const [
    offSnapshot,
    offProgress,
    offLog,
    offStep,
    offApplied,
    offPreselect,
    offDoctor,
    offJournal,
    offPanel,
    offPanelProgress,
    offPanelApplying,
    offPanelError,
  ] = await Promise.all([
      events.snapshot((snapshot) => {
        setState({ snapshot });
        void refreshBlockers();
      }),
      events.progress((_phase, message) => setState({ progress: message })),
      events.log((event) => setState({ log: [...state.log, event].slice(-400) })),
      events.step((step) => setState({ step })),
      events.applied((outcome) => settle(outcome)),
      events.preselect((ids) => {
        setState({ view: "overview", selection: new Set(ids) });
      }),
      events.openDoctor(() => {
        setState({ view: "doctor" });
        void runDoctor();
      }),
      events.journal((entry) => appendJournal(entry)),
      events.panelUpdate((update) => {
        setState({ panelUpdate: update });
        if (update) toast(t("panel.available", { version: update.latest }), "info");
      }),
      events.panelProgress((received, total) =>
        setState({ panelReceived: received, panelTotal: total }),
      ),
      events.panelApplying(() => setState({ panelBusy: true, view: "overview" })),
      events.panelError((message) => {
        setState({ panelBusy: false });
        toast(t("toast.panelFailed", { error: message }), "error");
      }),
    ]);

  unlisteners.push(
    offSnapshot,
    offProgress,
    offLog,
    offStep,
    offApplied,
    offPreselect,
    offDoctor,
    offJournal,
    offPanel,
    offPanelProgress,
    offPanelApplying,
    offPanelError,
  );

  try {
    const status: AppStatus = await api.status();
    const view: View =
      status.config.lastView === "doctor" ||
      status.config.lastView === "log" ||
      status.config.lastView === "settings"
        ? status.config.lastView
        : "overview";

    // The dictionary is chosen once, from the system language, before rendering.
    setLocale(status.locale);

    setState({
      booted: true,
      config: status.config,
      snapshot: status.snapshot,
      doctor: status.doctor,
      piHealth: status.piHealth,
      panelUpdate: status.panelUpdate ?? null,
      journal: mergeJournal(status.journal ?? [], state.journal),
      journalPath: status.journalPath,
      autostart: status.autostart,
      version: status.version,
      appDir: status.appDir,
      view,
    });

    if (view === "doctor" && !status.doctor) void runDoctor();
    if (view === "doctor" && !status.piHealth) void runPiHealth();
    if (view === "overview" && status.config.lastSelectedId) {
      select(status.config.lastSelectedId);
    }

    const snapshot = status.snapshot;
    const interval = (status.config.intervalMinutes || 360) * 60_000;
    const stale = !snapshot || Date.now() - Date.parse(snapshot.checkedAt) > interval;
    void refreshBlockers();
    if (stale) void check();
  } catch (error) {
    setState({ booted: true });
    toast(t("toast.stateFailed", { error: translateError(error) }), "error");
  }
}

export function dispose(): void {
  unlisteners.forEach((off) => off());
  unlisteners.length = 0;
}

export async function check(): Promise<void> {
  if (state.checking) return;
  setState({ checking: true, progress: t("progress.checkStart") });
  try {
    const snapshot = await api.check();
    setState({ snapshot, checking: false, progress: "" });
  } catch (error) {
    setState({ checking: false, progress: "" });
    toast(t("toast.checkFailed", { error: translateError(error) }), "error");
  }
}

export async function runDoctor(): Promise<void> {
  if (state.doctorRunning) return;
  setState({ doctorRunning: true });
  try {
    const doctor = await api.doctor();
    setState({ doctor, doctorRunning: false });
  } catch (error) {
    setState({ doctorRunning: false });
    toast(t("toast.doctorFailed", { error: translateError(error) }), "error");
  }
}

export async function refreshDoctorView(): Promise<void> {
  try {
    const view = await api.doctorView();
    if (view) setState({ doctor: view });
  } catch {
    // A missing stored report is not an error; the next run fills it.
  }
}

export async function toggleDoctorIgnore(id: string): Promise<void> {
  if (!state.config) return;
  const next = new Set(state.config.doctorIgnored);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  await saveConfig({ doctorIgnored: [...next] });
}

export async function runPiHealth(): Promise<void> {
  if (state.piHealthRunning) return;
  setState({ piHealthRunning: true });
  try {
    const piHealth = await api.piHealth();
    setState({ piHealth, piHealthRunning: false });
  } catch (error) {
    setState({ piHealthRunning: false });
    toast(t("toast.piHealthFailed", { error: translateError(error) }), "error");
  }
}

/// Downloads the panel's own installer and hands it to the helper: the app
/// closes, installs and comes back on its own.
export async function applyPanelUpdate(): Promise<void> {
  if (state.panelBusy || !state.panelUpdate) return;
  setState({
    panelBusy: true,
    panelReceived: 0,
    panelTotal: state.panelUpdate.size,
  });
  try {
    await api.applyPanelUpdate();
  } catch (error) {
    setState({ panelBusy: false });
    toast(t("toast.panelFailed", { error: translateError(error) }), "error");
  }
}

export async function refreshBlockers(): Promise<void> {
  try {
    const blockers = await api.blockers();
    setState({ blockers });
  } catch {
    // Detection is advisory; never block the UI on it.
  }
}

/// Asks the backend what is running before touching anything.
export async function requestApply(ids: string[]): Promise<void> {
  if (state.applying) return;
  if (ids.length === 0) {
    toast(t("toast.nothingSelected"), "info");
    return;
  }

  let report: PreflightReport;
  try {
    report = await api.preflight(ids);
  } catch {
    // If preflight fails, fall back to the plain apply (the backend still guards).
    await apply(ids);
    return;
  }

  setState({ blockers: report.blockers });

  const interference = report.steps.some((step) => step.impact !== "none");
  if (!interference) {
    await apply(ids);
    return;
  }

  setState({
    preflight: report,
    preflightSelected: new Set(
      report.steps.filter((step) => step.impact !== "block").map((step) => step.id),
    ),
  });
}

export function togglePreflightStep(id: string): void {
  const next = new Set(state.preflightSelected);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  setState({ preflightSelected: next });
}

export async function confirmPreflight(): Promise<void> {
  const ids = [...state.preflightSelected];
  setState({ preflight: null, preflightSelected: new Set<string>() });
  if (ids.length === 0) {
    toast(t("toast.nothingPicked"), "info");
    return;
  }
  await apply(ids);
}

export function cancelPreflight(): void {
  setState({ preflight: null, preflightSelected: new Set<string>() });
}

export async function apply(ids: string[]): Promise<void> {
  if (state.applying) return;
  if (ids.length === 0) {
    toast(t("toast.nothingSelected"), "info");
    return;
  }

  setState({ applying: true, outcome: null, log: [], step: null, view: "overview" });
  try {
    const outcome = await api.apply(ids);
    settle(outcome);
  } catch (error) {
    setState({ applying: false });
    toast(t("toast.applyFailed", { error: translateError(error) }), "error");
  }
}

function settle(outcome: ApplyOutcome): void {
  setState({
    applying: false,
    outcome,
    snapshot: outcome.snapshot ?? state.snapshot,
    doctor: outcome.doctor ?? state.doctor,
    selection: new Set<string>(),
  });

  if (outcome.failed.length > 0) {
    toast(
      t("toast.applyPartial", {
        applied: outcome.applied.length,
        failed: outcome.failed.length,
      }),
      "error",
    );
  } else if (outcome.applied.length > 0) {
    toast(tn("toast.applied", outcome.applied.length), "ok");
  } else {
    toast(t("toast.noChanges"), "info");
  }
}

export function select(id: string | null): void {
  const item = items().find((candidate) => candidate.id === id) ?? null;
  setState({ selectedId: item ? item.id : null, view: "overview" });
  if (item) {
    void loadNotes(item);
    if (state.config) {
      const config = { ...state.config, lastSelectedId: item.id };
      setState({ config });
      void api.saveConfig(config);
    }
  }
}

export function setView(view: View): void {
  // Clicking the active Resumen tab again acts as "back to the list".
  if (view === "overview" && state.view === "overview" && state.selectedId) {
    setState({ selectedId: null, view });
    return;
  }
  setState({ view, journalClearArmed: false });
  if (view === "doctor" && !state.doctor) void runDoctor();
  if (view === "doctor" && !state.piHealth) void runPiHealth();
  if (state.config) {
    const config = { ...state.config, lastView: view };
    setState({ config });
    void api.saveConfig(config);
  }
}

export function toggleIgnored(id: string): void {
  if (!state.config) return;

  const next = new Set(state.config.ignored);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  const ignored = [...next];

  const snapshot = state.snapshot
    ? {
        ...state.snapshot,
        items: state.snapshot.items.map((item) =>
          item.id === id ? { ...item, ignored: ignored.includes(id) } : item,
        ),
      }
    : null;

  const selection = new Set(state.selection);
  if (ignored.includes(id)) selection.delete(id);

  const config: Config = { ...state.config, ignored };
  setState({ config, snapshot, selection });

  void api
    .saveConfig(config)
    .then((saved) => setState({ config: saved }))
    .catch((error) => toast(t("toast.saveFailed", { error: translateError(error) }), "error"));
}

export function toggleSelection(id: string): void {
  const next = new Set(state.selection);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  setState({ selection: next });
}

export function selectAll(): void {
  const all = actionableItems();
  const everythingSelected = all.every((item) => state.selection.has(item.id));
  setState({ selection: everythingSelected ? new Set() : new Set(all.map((item) => item.id)) });
}

export async function saveConfig(patch: Partial<Config>): Promise<void> {
  if (!state.config) return;
  const config: Config = { ...state.config, ...patch };
  setState({ config });
  try {
    const saved = await api.saveConfig(config);
    setState({ config: saved });
    if (
      "doctorIgnored" in patch ||
      "ignoreMissingTools" in patch
    ) {
      void refreshDoctorView();
    }
  } catch (error) {
    toast(t("toast.saveFailed", { error: translateError(error) }), "error");
  }
}

export async function toggleAutostart(enabled: boolean): Promise<void> {
  try {
    const autostart = await api.setAutostart(enabled);
    setState({ autostart });
    toast(
      autostart ? t("toast.autostartOn") : t("toast.autostartOff"),
      "ok",
    );
  } catch (error) {
    toast(t("toast.autostartFailed", { error: translateError(error) }), "error");
  }
}

export async function loadNotes(item: UpdateItem, force = false): Promise<void> {
  if (!item.notesKey) return;
  if (state.notes[item.id] && !force) return;
  if (state.notesLoading[item.id]) return;

  setState({ notesLoading: { ...state.notesLoading, [item.id]: true } });
  try {
    const payload = await api.notes(item.notesKey, item.installed, item.latest, force);
    setState({
      notes: { ...state.notes, [item.id]: payload },
      notesLoading: { ...state.notesLoading, [item.id]: false },
    });

    const pending = state.pendingRelease[item.id];
    if (pending) {
      const index = payload.releases.findIndex((release) => release.version === pending);
      if (index >= 0) {
        setState({
          releaseIndex: { ...state.releaseIndex, [item.id]: index },
          pendingRelease: { ...state.pendingRelease, [item.id]: "" },
        });
      }
    }

    if (state.config?.translationEnabled) {
      for (const release of payload.releases) void loadTranslation(item, release);
    }
  } catch (error) {
    setState({
      notes: {
        ...state.notes,
        [item.id]: {
          key: item.notesKey,
          title: item.name,
          releases: [],
          url: null,
          fetchedAt: new Date().toISOString(),
          error: String(error),
        },
      },
      notesLoading: { ...state.notesLoading, [item.id]: false },
    });
  }
}

export function translationKey(item: UpdateItem, release: Release): string {
  return `${item.id}|${release.version}`;
}

export async function loadTranslation(item: UpdateItem, release: Release): Promise<void> {
  if (!state.config?.translationEnabled) return;
  if (!item.notesKey) return;

  const key = translationKey(item, release);
  if (state.translations[key] || state.translationBusy[key]) return;

  setState({ translationBusy: { ...state.translationBusy, [key]: true } });
  try {
    const result = await api.translate(`${item.notesKey}|${release.version}`, release.body);
    setState({
      translations: {
        ...state.translations,
        [key]: { text: result.text, engine: result.engine },
      },
      translationBusy: { ...state.translationBusy, [key]: false },
      translationError: { ...state.translationError, [key]: "" },
    });
  } catch (error) {
    setState({
      translationBusy: { ...state.translationBusy, [key]: false },
      translationError: { ...state.translationError, [key]: String(error) },
    });
  }
}

export async function toggleTranslation(): Promise<void> {
  if (!state.config) return;
  const enabled = !state.config.translationEnabled;
  await saveConfig({ translationEnabled: enabled });

  if (enabled) {
    const item = items().find((candidate) => candidate.id === state.selectedId);
    const payload = item ? state.notes[item.id] : undefined;
    if (item && payload) {
      for (const release of payload.releases) void loadTranslation(item, release);
    }
  }
}

export function releaseCount(itemId: string): number {
  return state.notes[itemId]?.releases.length ?? 0;
}

export function stepRelease(itemId: string, delta: 1 | -1): void {
  const total = releaseCount(itemId);
  if (total === 0) return;
  const current = state.releaseIndex[itemId] ?? 0;
  const next = Math.min(Math.max(current + delta, 0), total - 1);
  if (next === current) return;
  setState({ releaseIndex: { ...state.releaseIndex, [itemId]: next } });
}

export async function searchNotes(query: string): Promise<void> {
  const trimmed = query.trim();
  if (trimmed.length < 2) {
    setState({ notesSearch: null });
    return;
  }
  setState({ notesSearch: { query: trimmed, hits: [], busy: true } });
  try {
    const hits = await api.searchNotes(trimmed);
    setState({ notesSearch: { query: trimmed, hits, busy: false } });
  } catch (error) {
    setState({ notesSearch: { query: trimmed, hits: [], busy: false } });
    toast(t("toast.searchFailed", { error: translateError(error) }), "error");
  }
}

export function clearNotesSearch(): void {
  setState({ notesSearch: null });
}

export function openHit(hit: NoteHit): void {
  const item = items().find((candidate) => candidate.notesKey === hit.notesKey);
  if (!item) {
    toast(t("toast.noItemForNotes", { key: hit.notesKey }), "info");
    return;
  }
  setState({
    pendingRelease: { ...state.pendingRelease, [item.id]: hit.version },
    notesSearch: null,
  });
  select(item.id);
}

export function openContextMenu(x: number, y: number, href: string): void {
  setState({ contextMenu: { x, y, href } });
}

export function closeContextMenu(): void {
  if (state.contextMenu) setState({ contextMenu: null });
}

export async function openUrl(url: string): Promise<void> {
  try {
    await api.openUrl(url);
  } catch (error) {
    toast(t("toast.openUrlFailed", { error: translateError(error) }), "error");
  }
}

export function moveSelection(direction: 1 | -1): void {
  const list = items();
  if (list.length === 0) return;
  const current = list.findIndex((item) => item.id === state.selectedId);
  const next = current === -1 ? 0 : Math.min(Math.max(current + direction, 0), list.length - 1);
  select(list[next].id);
}
