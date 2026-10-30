// Views: pure functions from state to markup. Events are delegated in shell.ts.

import {
  actionableItems,
  items,
  journalEntries,
  pendingCount,
  state,
  translationKey,
  updatesCount,
  type JournalFilter,
  type View,
} from "../store";
import type {
  DoctorCheckView,
  HealthCheck,
  JournalEntry,
  PackageHealth,
  Release,
  UpdateItem,
} from "../types";
import {
  clockSeconds,
  clockTime,
  dayKey,
  dayLabel,
  delta,
  journalDetail,
  journalTitle,
  esc,
  gauge,
  doctorMeta,
  icons,
  isActionable,
  markdownToHtml,
  repositorySlug,
  relativeTime,
  sourceLabel,
  statusMeta,
} from "./format";

import { t, tn } from "../i18n";

const GROUPS: Array<{ kind: UpdateItem["kind"]; label: string }> = [
  { kind: "ecosystem", label: "group.ecosystem" },
  { kind: "pi", label: "group.pi" },
  { kind: "piPackage", label: "group.piPackages" },
];

export function renderRail(): string {
  const all = items();
  const pending = pendingCount();
  const total = all.length;
  const snapshot = state.snapshot;
  const warnings = snapshot?.warnings ?? [];

  const groups = GROUPS.map((group) => {
    const groupItems = all.filter((item) => item.kind === group.kind);
    if (groupItems.length === 0) return "";
    return `<div class="group">
      <div class="eyebrow group-label">${t(group.label)}</div>
      ${groupItems.map(pieceRow).join("")}
    </div>`;
  }).join("");

  return `
  <aside class="rail">
    <div class="rail-top">
      <div class="eyebrow">${t("rail.system")}</div>
      <div class="system-card">
        <div class="system-gauge">${gauge(pending > 0 ? "updateAvailable" : snapshot ? "upToDate" : "unknown", 78)}</div>
        <div class="system-numbers">
          <div class="big"><span class="accent">${pending}</span><span class="slash">/</span><span class="total">${total}</span></div>
          <div class="caption">${pending === 0 ? t("rail.allClear") : t("rail.pending", { count: pending })}</div>
          <div class="stamp">${t("rail.lastChecked", { time: relativeTime(snapshot?.checkedAt ?? null) })}</div>
        </div>
      </div>
      <button class="btn btn-primary btn-block" data-action="check" ${state.checking ? "disabled" : ""}>
        ${icons.refresh}
        <span>${state.checking ? t("rail.checking") : t("rail.checkNow")}</span>
      </button>
    </div>

    <nav class="nav" role="tablist" aria-label="${t("nav.sections")}">
      ${navItem("overview", t("nav.overview"), icons.overview)}
      ${navItem("doctor", t("nav.doctor"), icons.stethoscope)}
      ${navItem("log", t("nav.log"), icons.journal)}
      ${navItem("settings", t("nav.settings"), icons.sliders)}
    </nav>

    <div class="pieces">
      <div class="eyebrow pieces-label">${t("rail.pieces")}</div>
      ${total === 0 ? `<p class="rail-empty">${t("rail.empty")}</p>` : groups}
    </div>

    <div class="rail-foot">
      ${warnings.length > 0 ? `<button class="warn-link" data-action="warnings">${icons.alert}<span>${tn("rail.warnings", warnings.length)}</span></button>` : `<span class="quiet">${t("rail.noWarnings")}</span>`}
      <span class="version">v${esc(state.version || "0.1.0")}</span>
    </div>

    ${state.warningsOpen ? warningsPopover(warnings) : ""}
  </aside>`;
}

function navItem(view: View, label: string, icon: string): string {
  const active = state.view === view;
  return `<button role="tab" aria-selected="${active}" class="nav-item ${active ? "is-active" : ""}"
      data-action="view" data-view="${view}">${icon}<span>${label}</span>
      ${view === "doctor" && pendingCount() > 0 ? '<span class="nav-badge"></span>' : ""}
    </button>`;
}

function pieceRow(item: UpdateItem): string {
  const selected = state.selectedId === item.id;
  const checked = state.selection.has(item.id);
  const ignored = item.ignored;
  const meta = ignored ? { label: t("piece.ignored"), tone: "faint" as const } : statusMeta(item.status);
  const actionable = !ignored && isActionable(item.status);

  return `<div class="piece ${selected ? "is-selected" : ""} ${ignored ? "is-ignored" : ""}" role="button" tabindex="0"
      data-action="select" data-id="${esc(item.id)}" aria-current="${selected}">
    ${
      actionable
        ? `<span class="piece-check"><input type="checkbox" id="pick-${esc(item.id)}" data-action="toggle"
             data-id="${esc(item.id)}" ${checked ? "checked" : ""}
             aria-label="${t("piece.select", { name: esc(item.name) })}"/><label for="pick-${esc(item.id)}"></label></span>`
        : `<span class="piece-check is-empty"></span>`
    }
    <span class="dot dot--${meta.tone}" aria-hidden="true"></span>
    <span class="piece-name">${esc(item.name)}</span>
    <span class="delta" title="${esc(item.installed ?? t("piece.notInstalled"))} → ${esc(item.latest ?? "?")}">${esc(delta(item.installed, item.latest))}</span>
  </div>`;
}

function warningsPopover(warnings: string[]): string {
  return `<div class="popover" role="dialog" aria-label="${t("warnings.dialog")}">
    <div class="popover-head">
      <span class="eyebrow">${t("warnings.title")}</span>
      <button class="icon-btn" data-action="warnings" aria-label="${t("warnings.close")}">${icons.x}</button>
    </div>
    <ul class="warn-list">${warnings.map((warning) => `<li>${esc(warning)}</li>`).join("")}</ul>
  </div>`;
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

export function renderContent(): string {
  if (state.view === "doctor") return renderDoctor();
  if (state.view === "log") return renderJournal();
  if (state.view === "settings") return renderSettings();

  const selected = state.snapshot?.items.find((item) => item.id === state.selectedId);
  if (selected) return renderDetail(selected);
  return renderOverview();
}

function renderOverview(): string {
  const snapshot = state.snapshot;
  const pending = pendingCount();
  const selection = state.selection.size;

  if (!snapshot) {
    return `<section class="content">
      <div class="empty-state">
        ${gauge("unknown", 120)}
        <h1 class="display">${t("overview.emptyTitle")}</h1>
        <p class="lead">${state.checking ? t("overview.emptyLeadChecking") : t("overview.emptyLead")}</p>
        <button class="btn btn-primary" data-action="check" ${state.checking ? "disabled" : ""}>${icons.refresh}<span>${state.checking ? t("rail.checking") : t("rail.checkNow")}</span></button>
      </div>
    </section>`;
  }

  const groups = GROUPS.map((group) => {
    const groupItems = snapshot.items.filter((item) => item.kind === group.kind);
    if (groupItems.length === 0) return "";
    const pendingInGroup = groupItems.filter(
      (item) => !item.ignored && isActionable(item.status),
    ).length;
    return `<section class="card group-card">
      <header class="card-head">
        <h2 class="card-title">${t(group.label)}</h2>
        <span class="card-meta">${pendingInGroup > 0 ? `${pendingInGroup} ${t("overview.groupPending", { count: pendingInGroup })}` : t("overview.groupClear")}</span>
      </header>
      <div class="rows">${groupItems.map(overviewRow).join("")}</div>
    </section>`;
  }).join("");

  return `<section class="content">
    <header class="view-head">
      <div class="view-head-copy">
        <div class="eyebrow">${t("nav.overview")}</div>
        <h1 class="display">${pending === 0 ? t("overview.allClear") : `${pending} ${t("overview.available", { count: pending })}`}</h1>
        <p class="lead">${t("overview.lead", { when: relativeTime(snapshot.checkedAt), time: clockTime(snapshot.checkedAt), pieces: snapshot.items.length, seconds: Math.max(1, Math.round(snapshot.durationMs / 1000)) })}</p>
      </div>
      <div class="hero-gauge">${gauge(pending > 0 ? "updateAvailable" : "upToDate", 148, true)}</div>
    </header>

    <div class="toolbar">
      <button class="btn btn-primary" data-action="apply-selected" ${selection === 0 || state.applying ? "disabled" : ""}>
        ${icons.check}<span>${t("overview.applySelected")}${selection > 0 ? ` (${selection})` : ""}</span>
      </button>
      <button class="btn" data-action="select-all" ${pending === 0 ? "disabled" : ""}>
        ${selection === actionableItems().length && pending > 0 ? t("overview.clearSelection") : t("overview.selectAll")}
      </button>
      <span class="toolbar-hint">${updatesCount() > 0 ? `${updatesCount()} ${t("overview.newVersions", { count: updatesCount() })}` : t("overview.nothingToInstall")}</span>
    </div>

    ${renderNotesSearch()}
    ${state.blockers.some((blocker) => blocker.kind === "pi") ? renderPiRunningBand() : ""}
    ${renderPanelBand()}
    ${snapshot.warnings.length > 0 ? renderWarnBand(snapshot.warnings) : ""}
    ${groups}
  </section>`;
}

function overviewRow(item: UpdateItem): string {
  const ignored = item.ignored;
  const meta = ignored ? { label: t("piece.ignored"), tone: "faint" as const } : statusMeta(item.status);
  const actionable = !ignored && isActionable(item.status);
  const checked = state.selection.has(item.id);
  const showsDelta = Boolean(item.installed && item.latest && item.installed !== item.latest);
  const versions = showsDelta
    ? `${item.installed}<span class="arrow">${icons.arrowRight}</span><span class="latest">${item.latest}</span>`
    : `${item.installed ?? item.latest ?? "—"}`;
  const sourceLine =
    item.detail && !ignored
      ? `${sourceLabel(item.source)} · ${item.detail}`
      : sourceLabel(item.source);

  return `<div class="row ${state.selectedId === item.id ? "is-selected" : ""} ${ignored ? "is-ignored" : ""}">
    <div class="row-pick">
      ${actionable ? `<input type="checkbox" id="row-${esc(item.id)}" data-action="toggle" data-id="${esc(item.id)}" ${checked ? "checked" : ""} aria-label="${t("piece.select", { name: esc(item.name) })}"/><label for="row-${esc(item.id)}"></label>` : ""}
    </div>
    <button class="row-main" data-action="select" data-id="${esc(item.id)}">
      <span class="row-name">${esc(item.name)}</span>
      <span class="row-source">${esc(sourceLine)}</span>
    </button>
    <span class="row-versions">${versions}</span>
    <span class="pill pill--${meta.tone}">${meta.label}</span>
    ${actionable ? `<button class="btn btn-ghost btn-small" data-action="apply-one" data-id="${esc(item.id)}" ${state.applying ? "disabled" : ""}>${t("action.apply")}</button>` : ignored ? `<span class="row-ignored">${icons.x}</span>` : `<span class="row-ok">${icons.check}</span>`}
  </div>`;
}

function renderPiRunningBand(): string {
  return `<div class="warn-band">
    ${icons.alert}
    <div><strong>${t("overview.piRunningTitle")}</strong>
    <span>${t("overview.piRunningBody")}</span></div>
  </div>`;
}

function renderPanelBand(): string {
  const update = state.panelUpdate;
  if (!update) return "";

  if (state.panelBusy) {
    const total = state.panelTotal || update.size;
    const percent = total > 0 ? Math.min(100, Math.round((state.panelReceived / total) * 100)) : 0;
    return `<div class="panel-band is-busy">
    ${icons.refresh}
    <div><strong>${t("panel.installing", { version: esc(update.latest) })}</strong>
    <span>${t("panel.progress", { percent })}</span></div>
  </div>`;
  }

  return `<div class="panel-band">
    ${icons.arrowRight}
    <div><strong>${t("panel.available", { version: esc(update.latest) })}</strong>
    <span>${t("panel.current", { version: esc(update.current) })}</span></div>
    <button class="btn btn-primary btn-small" data-action="apply-panel-update" ${state.applying ? "disabled" : ""}>${t("panel.apply")}</button>
  </div>`;
}

function renderWarnBand(warnings: string[]): string {
  return `<div class="warn-band">
    ${icons.alert}
    <div><strong>${tn("overview.warningsBand", warnings.length)}</strong>
    <span>${esc(warnings.slice(0, 2).join(" · "))}${warnings.length > 2 ? "…" : ""}</span></div>
    <button class="btn btn-ghost btn-small" data-action="warnings">${t("overview.warningsShow")}</button>
  </div>`;
}

// ---------------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------------

function renderDetail(item: UpdateItem): string {
  const ignored = item.ignored;
  const meta = ignored ? { label: t("piece.ignored"), tone: "faint" as const } : statusMeta(item.status);
  const actionable = !ignored && isActionable(item.status);
  const notes = state.notes[item.id];
  const loading = state.notesLoading[item.id];

  return `<section class="content detail">
    <header class="detail-head">
      <div class="detail-copy">
        <button class="btn btn-ghost btn-small back-link" data-action="back">${t("detail.back")}</button>
        <div class="eyebrow">${item.kind === "ecosystem" ? t("group.ecosystem") : item.kind === "pi" ? t("detail.core") : t("detail.piPackage")}</div>
        <h1 class="display detail-name">${esc(item.name)}</h1>
        <div class="detail-sub"><span class="mono">${esc(sourceLabel(item.source))}</span><span class="pill pill--${meta.tone}">${meta.label}</span></div>
        ${item.detail && !ignored ? `<p class="detail-note">${esc(item.detail)}</p>` : ""}
        ${ignored ? `<p class="detail-note">${t("detail.ignoredNote")}</p>` : ""}
      </div>
      <div class="detail-gauge">${gauge(item.status, 132, true)}</div>
    </header>

    <div class="versions">
      <div class="version-cell"><span class="version-label">${t("detail.installed")}</span><span class="version-value">${esc(item.installed ?? "—")}</span></div>
      <span class="version-arrow">${icons.arrowRight}</span>
      <div class="version-cell"><span class="version-label">${t("detail.available")}</span><span class="version-value accent">${esc(item.latest ?? "—")}</span></div>
    </div>

    <div class="actions">
      ${actionable ? `<button class="btn btn-primary" data-action="apply-one" data-id="${esc(item.id)}" ${state.applying ? "disabled" : ""}>${icons.arrowRight}<span>${item.installed ? t("detail.updateTo", { version: esc(item.latest ?? "") }) : t("action.install")}</span></button>` : ""}
      ${item.notesKey ? `<button class="btn" data-action="load-notes" data-id="${esc(item.id)}">${icons.refresh}<span>${t("detail.viewNotes")}</span></button>` : ""}
      ${notes?.url ? `<button class="btn" data-action="open-url" data-url="${esc(notes.url)}">${icons.external}<span>${t("detail.openBrowser")}</span></button>` : ""}
      <button class="btn" data-action="doctor">${icons.stethoscope}<span>${t("nav.doctor")}</span></button>
      <button class="btn" data-action="toggle-ignored" data-id="${esc(item.id)}">
        ${ignored ? icons.check : icons.x}<span>${ignored ? t("action.unignore") : t("action.ignore")}</span>
      </button>
    </div>

    ${renderNotes(item, notes, Boolean(loading))}
  </section>`;
}

function renderNotes(
  item: UpdateItem,
  notes: import("../types").NotesPayload | undefined,
  loading: boolean,
): string {
  // No notes key means we do not know where its releases live. Say so instead
  // of showing another project's changelog.
  if (!item.notesKey) {
    return `<section class="card notes-card">
      <header class="card-head"><h2 class="card-title">${t("detail.notesTitle")}</h2></header>
      <div class="notes-empty"><p>${t("detail.notesNoRepo")}</p></div>
    </section>`;
  }

  if (loading && !notes) {
    return `<section class="card notes-card">
      <header class="card-head"><h2 class="card-title">${t("detail.notesTitle")}</h2></header>
      <div class="notes-loading"><span class="spinner" aria-hidden="true"></span><span>${t("detail.notesLoading", { name: esc(item.name) })}</span></div>
    </section>`;
  }

  if (!notes) {
    return `<section class="card notes-card">
      <header class="card-head"><h2 class="card-title">${t("detail.notesTitle")}</h2></header>
      <div class="notes-empty"><p>${t("detail.notesIdle")}</p>
      <button class="btn btn-small" data-action="load-notes" data-id="${esc(item.id)}">${t("detail.notesLoad")}</button></div>
    </section>`;
  }

  const total = notes.releases.length;
  const index = Math.min(state.releaseIndex[item.id] ?? 0, Math.max(total - 1, 0));
  const release = notes.releases[index];
  const repo = repositorySlug(notes.url);

  const nav =
    total > 1
      ? `<div class="release-nav" role="group" aria-label="${t("detail.versions")}">
          <button class="release-arrow" data-action="release-prev" data-id="${esc(item.id)}"
            ${index <= 0 ? "disabled" : ""} aria-label="${t("detail.newerRelease")}" title="${t("detail.newerRelease")}">${icons.chevronLeft}</button>
          <span class="release-counter">${index + 1} / ${total}</span>
          <button class="release-arrow" data-action="release-next" data-id="${esc(item.id)}"
            ${index >= total - 1 ? "disabled" : ""} aria-label="${t("detail.olderRelease")}" title="${t("detail.olderRelease")}">${icons.chevronRight}</button>
        </div>`
      : "";

  return `<section class="card notes-card">
    <header class="card-head">
      <h2 class="card-title">${t("detail.releaseTitle", { version: esc(release?.version ?? "") })}</h2>
      <div class="card-tools">
        ${nav}
        ${total > 0 ? `<button class="btn btn-small ${state.config?.translationEnabled ? "btn-primary" : ""}" data-action="toggle-translation">
          ${state.config?.translationEnabled ? t("detail.showOriginal") : t("detail.translate")}
        </button>` : ""}
        <button class="icon-btn" data-action="load-notes" data-id="${esc(item.id)}" aria-label="${t("detail.reloadNotes")}" title="${t("detail.reload")}">${icons.refresh}</button>
      </div>
    </header>
    ${
      notes.error
        ? `<div class="notes-empty"><p>${esc(notes.error)}</p>${notes.url ? `<button class="btn btn-small" data-action="open-url" data-url="${esc(notes.url)}">${t("detail.openProject")}</button>` : ""}</div>`
        : total === 0
          ? `<div class="notes-empty"><p>${t("detail.notesEmpty")}</p></div>`
          : `<div class="releases">${releaseBlock(item, release, repo)}</div>`
    }
  </section>`;
}

function releaseBlock(item: UpdateItem, release: Release, repo?: string): string {
  const key = translationKey(item, release);
  const translated = state.config?.translationEnabled ? state.translations[key] : undefined;
  const busy = Boolean(state.translationBusy[key]);
  const error = state.translationError[key];
  const body = markdownToHtml(translated?.text ?? release.body, repo);

  const badge = translated
    ? `<span class="pill pill--accent">${t("detail.translated", { engine: esc(engineLabel(translated.engine)) })}</span>`
    : busy
      ? `<span class="pill pill--faint">${t("detail.translating")}</span>`
      : "";

  return `<article class="release">
    <header class="release-head">
      <span class="release-version">${esc(release.version)}</span>
      ${release.date ? `<span class="release-date">${esc(release.date)}</span>` : ""}
      ${badge}
    </header>
    <div class="md">${body}</div>
    ${error ? `<p class="translation-error">${t("detail.translateFailed", { error: esc(error) })}</p>` : ""}
  </article>`;
}

function engineLabel(engine: string): string {
  if (engine === "google") return "Google";
  if (engine === "microsoft") return "Microsoft";
  return "—";
}

// ---------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------

function renderDoctor(): string {
  const view = state.doctor;
  const meta = doctorMeta(view?.status ?? "");
  const active = view?.checks.filter((check) => !check.ignored) ?? [];
  const hidden = view?.checks.filter((check) => check.ignored) ?? [];

  const lead = view
    ? [
        view.summary ?? "",
        view.ignoredCount > 0
          ? `${view.ignoredCount} ${t("doctor.silencedCount", { count: view.ignoredCount })}`
          : "",
        view.originalStatus !== view.status
          ? t("doctor.cliSaid", { status: view.originalStatus })
          : "",
      ]
        .filter(Boolean)
        .join(" · ")
    : t("doctor.emptyLead");

  const head = `<header class="view-head">
    <div class="view-head-copy">
      <div class="eyebrow">${t("doctor.eyebrow")}</div>
      <h1 class="display">${view ? meta.label : t("doctor.ecosystemState")}</h1>
      <p class="lead">${esc(lead)}</p>
    </div>
    <div class="hero-gauge"><span class="doctor-badge tone--${view ? meta.tone : "faint"}">${view ? meta.label : t("status.unknown")}</span></div>
  </header>`;

  const controls = `<div class="toolbar">
    <button class="btn btn-primary" data-action="doctor" ${state.doctorRunning ? "disabled" : ""}>${icons.stethoscope}<span>${state.doctorRunning ? t("doctor.running") : t("doctor.run")}</span></button>
    ${view ? `<span class="toolbar-hint">${active.length} ${t("doctor.activeChecks", { count: active.length })}${view.ignoredCount > 0 ? ` · ${t("doctor.hintSilenced", { count: view.ignoredCount })}` : ""}</span>` : ""}
  </div>`;

  if (!view) {
    return `<section class="content">${head}${controls}
      <div class="empty-state small">${icons.stethoscope}<p class="lead">${t("doctor.empty")}</p></div>
    </section>`;
  }

  const checks = active.map(doctorRow).join("");
  const hiddenBlock = hidden.length
    ? `<details class="raw-block ignored-checks">
        <summary>${tn("doctor.silencedChecks", hidden.length)}</summary>
        <div class="checks">${hidden.map(doctorRow).join("")}</div>
      </details>`
    : "";

  return `<section class="content">${head}${controls}
    <section class="card"><div class="checks">${checks || `<div class="notes-empty"><p>${t("doctor.allSilenced")}</p></div>`}</div></section>
    ${hiddenBlock}
    ${renderPiHealth()}
    <details class="raw-block">
      <summary>${t("doctor.raw")}</summary>
      <pre class="raw">${esc(view.raw)}</pre>
    </details>
  </section>`;
}

function doctorRow(check: DoctorCheckView): string {
  const tone = check.marker === "ok" ? "ok" : check.marker === "!!" ? "accent" : "danger";
  return checkRow(check.id, tone, check.marker, check.message, check.remedy);
}

function checkRow(
  id: string,
  tone: "ok" | "accent" | "danger",
  marker: string,
  message: string,
  remedy: string | null,
): string {
  return `<div class="check">
    <span class="check-marker tone--${tone}">${esc(marker)}</span>
    <div class="check-body">
      <div class="check-id">${esc(id)}</div>
      <div class="check-message">${esc(message)}</div>
      ${remedy ? `<div class="remedy">${icons.alert}<span>${esc(remedy)}</span>
        <button class="btn btn-ghost btn-small" data-action="copy" data-text="${esc(remedy)}">${icons.copy}<span>${t("action.copy")}</span></button></div>` : ""}
    </div>
  </div>`;
}

function healthTone(marker: string): "ok" | "accent" | "danger" {
  if (marker === "ok") return "ok";
  if (marker === "warn") return "accent";
  return "danger";
}

function healthRow(check: HealthCheck): string {
  return checkRow(check.id, healthTone(check.marker), check.marker, check.message, check.remedy);
}

function renderPiHealth(): string {
  const health = state.piHealth;

  const header = `<header class="card-head">
    <h2 class="card-title">${t("doctor.piHealthTitle")}</h2>
    <div class="card-tools">
      ${health ? `<span class="card-meta">${t("doctor.piHealthMeta", { pieces: health.packages.length, problems: health.packages.filter((pkg) => pkg.status !== "ok").length, time: esc(relativeTime(health.checkedAt)) })}</span>` : ""}
      <button class="btn btn-small" data-action="pi-health" ${state.piHealthRunning ? "disabled" : ""}>
        ${state.piHealthRunning ? t("rail.checking") : health ? t("doctor.recheck") : t("doctor.checkInstall")}
      </button>
    </div>
  </header>`;

  if (!health) {
    return `<section class="card">${header}
      <div class="notes-empty"><p>${t("doctor.piHealthEmpty")}</p></div>
    </section>`;
  }

  const packages = health.packages.map(packageBlock).join("");
  return `<section class="card">
    ${header}
    <div class="checks">${health.checks.map(healthRow).join("")}</div>
    <div class="pkg-list">${packages || `<div class="notes-empty"><p>${t("doctor.piHealthNoPackages")}</p></div>`}</div>
  </section>`;
}

function packageBlock(pkg: PackageHealth): string {
  const tone = healthTone(pkg.status);
  const label =
    pkg.status === "ok"
      ? t("doctor.ok")
      : pkg.status === "warn"
        ? t("doctor.review")
        : t("doctor.failed");
  const pillTone = tone === "accent" ? "accent" : tone;
  const open = pkg.status === "ok" ? "" : "open";

  return `<details class="pkg" ${open}>
    <summary class="pkg-summary">
      <span class="dot dot--${tone}"></span>
      <span class="pkg-name">${esc(pkg.name)}</span>
      <span class="pkg-path mono">${esc(pkg.path)}</span>
      <span class="pill pill--${pillTone}">${label}</span>
    </summary>
    <div class="pkg-checks">${pkg.checks.map(healthRow).join("")}</div>
  </details>`;
}

// ---------------------------------------------------------------------------
// Registro
// ---------------------------------------------------------------------------

const JOURNAL_FILTERS: Array<{ value: JournalFilter; label: string }> = [
  { value: "all", label: "journal.filterAll" },
  { value: "detected", label: "journal.filterDetected" },
  { value: "updates", label: "journal.filterUpdates" },
  { value: "checks", label: "journal.filterChecks" },
  { value: "problems", label: "journal.filterProblems" },
];

interface JournalMeta {
  icon: string;
  tone: "ok" | "accent" | "danger" | "faint";
}

function journalMeta(kind: JournalEntry["kind"]): JournalMeta {
  switch (kind) {
    case "detected":
      return { icon: icons.arrowRight, tone: "accent" };
    case "preflight":
      return { icon: icons.alert, tone: "accent" };
    case "apply":
      return { icon: icons.refresh, tone: "accent" };
    case "step":
      return { icon: icons.terminal, tone: "faint" };
    case "applied":
      return { icon: icons.check, tone: "ok" };
    case "failed":
      return { icon: icons.alert, tone: "danger" };
    case "doctor":
      return { icon: icons.stethoscope, tone: "faint" };
    case "panel":
      return { icon: icons.refresh, tone: "accent" };
    case "check":
      return { icon: icons.refresh, tone: "faint" };
    default:
      return { icon: icons.mark, tone: "faint" };
  }
}

function renderJournal(): string {
  const all = state.journal;
  const entries = journalEntries();
  const oldest = all[0]?.at ?? null;
  const newest = all[all.length - 1]?.at ?? null;
  // Derived from the backend path so a future rename never lies in the copy.
  const fileName = state.journalPath.split(/[\\/]/).pop() || "journal.ndjson";

  const head = `<header class="view-head">
    <div class="view-head-copy">
      <div class="eyebrow">${t("nav.log")}</div>
      <h1 class="display">${all.length === 0 ? t("journal.emptyTitle") : t("journal.title")}</h1>
      <p class="lead">${all.length === 0 ? t("journal.emptyLead") : tn("journal.lead", all.length, { since: relativeTime(oldest), file: esc(fileName) })}</p>
    </div>
    <div class="journal-stats">
      <span class="journal-stat"><strong>${all.length}</strong><em>${t("journal.events", { count: all.length })}</em></span>
      <span class="journal-stamp">${t("journal.last", { time: relativeTime(newest) })}</span>
    </div>
  </header>`;

  const toolbar = `<div class="toolbar journal-toolbar">
    <button class="btn btn-primary" data-action="check" ${state.checking ? "disabled" : ""}>${icons.refresh}<span>${state.checking ? t("rail.checking") : t("journal.check")}</span></button>
    <button class="btn" data-action="journal-copy" ${all.length === 0 ? "disabled" : ""}>${icons.copy}<span>${t("action.copy")}</span></button>
    <button class="btn ${state.journalClearArmed ? "btn-danger" : "btn-ghost"}" data-action="journal-clear" ${all.length === 0 ? "disabled" : ""}>
      ${icons.x}<span>${state.journalClearArmed ? t("journal.confirmClear") : t("journal.clear")}</span>
    </button>
    <span class="toolbar-hint">${state.journalClearArmed ? t("journal.clearArmedHint") : entries.length === all.length ? "" : t("journal.filtered", { shown: entries.length, total: all.length })}</span>
  </div>`;

  const controls = `<div class="journal-controls">
    <div class="segmented journal-filters" role="group" aria-label="${t("journal.filterLabel")}">
      ${JOURNAL_FILTERS.map(
        (filter) => `<button class="segment ${state.journalFilter === filter.value ? "is-active" : ""}"
          data-action="journal-filter" data-value="${filter.value}"
          aria-pressed="${state.journalFilter === filter.value}">${t(filter.label)}</button>`,
      ).join("")}
    </div>
    <div class="journal-search">
      <input class="search-input" type="search" data-role="journal-search"
        placeholder="${t("journal.searchPlaceholder")}"
        value="${esc(state.journalQuery)}" aria-label="${t("journal.filterLabel")}"/>
      <button class="btn btn-small" data-action="journal-search">${t("journal.filterButton")}</button>
      ${state.journalQuery ? `<button class="btn btn-ghost btn-small" data-action="journal-search-clear">${t("action.clear")}</button>` : ""}
    </div>
  </div>`;

  let body: string;
  if (all.length === 0) {
    body = `<div class="empty-state small">${icons.journal}
      <p class="lead">${t("journal.noEntries")}</p>
    </div>`;
  } else if (entries.length === 0) {
    body = `<div class="empty-state small"><p class="lead">${t("journal.noMatches")}</p></div>`;
  } else {
    body = journalDays(entries);
  }

  return `<section class="content journal-view">${head}${toolbar}${controls}${body}</section>`;
}

/** Groups newest-first entries into one card per local day. */
function journalDays(entries: JournalEntry[]): string {
  const days = new Map<string, JournalEntry[]>();
  for (const entry of entries) {
    const key = dayKey(entry.at);
    const bucket = days.get(key);
    if (bucket) bucket.push(entry);
    else days.set(key, [entry]);
  }

  return [...days.entries()]
    .map(
      ([key, list]) => `<section class="card journal-day">
      <header class="card-head">
        <h2 class="card-title">${esc(dayLabel(key))}</h2>
        <span class="card-meta">${list.length} ${t("journal.events", { count: list.length })}</span>
      </header>
      <div class="journal-rows">${list.map(journalRow).join("")}</div>
    </section>`,
    )
    .join("");
}

function journalRow(entry: JournalEntry): string {
  const time = `<span class="jtime mono">${clockSeconds(entry.at)}</span>`;

  // Streamed output keeps its own compact shape: no icon, no emphasis.
  if (entry.kind === "output") {
    return `<div class="jrow jrow--output is-output">${time}
      <div class="jcopy"><span class="jline mono">${esc(journalTitle(entry))}</span></div>
    </div>`;
  }

  const meta = journalMeta(entry.kind);
  const detail = journalDetail(entry);
  return `<div class="jrow jrow--${entry.kind}">
    ${time}
    <span class="jicon tone--${meta.tone}">${meta.icon}</span>
    <div class="jcopy">
      <span class="jtitle">${esc(journalTitle(entry))}</span>
      ${detail ? `<span class="jdetail">${esc(detail)}</span>` : ""}
    </div>
  </div>`;
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

const INTERVALS: Array<{ minutes: number; label: string }> = [
  { minutes: 15, label: "settings.interval15" },
  { minutes: 60, label: "settings.interval1h" },
  { minutes: 180, label: "settings.interval3h" },
  { minutes: 360, label: "settings.interval6h" },
  { minutes: 720, label: "settings.interval12h" },
  { minutes: 1440, label: "settings.interval24h" },
  { minutes: 0, label: "settings.manual" },
];

const AUTOMATION: Array<{ value: "notify" | "confirm" | "auto"; title: string; body: string }> = [
  {
    value: "notify",
    title: "settings.automationNotify",
    body: "settings.automationNotifyBody",
  },
  {
    value: "confirm",
    title: "settings.automationConfirm",
    body: "settings.automationConfirmBody",
  },
  {
    value: "auto",
    title: "settings.automationAuto",
    body: "settings.automationAutoBody",
  },
];

function renderSettings(): string {
  const config = state.config;
  if (!config) return `<section class="content"><div class="empty-state"><p class="lead">${t("settings.loading")}</p></div></section>`;

  return `<section class="content settings">
    <header class="view-head">
      <div class="view-head-copy">
        <div class="eyebrow">${t("nav.settings")}</div>
        <h1 class="display">${t("settings.title")}</h1>
        <p class="lead">${t("settings.lead")}</p>
      </div>
    </header>

    <section class="card">
      <header class="card-head"><h2 class="card-title">${t("settings.frequency")}</h2><span class="card-meta">${config.intervalMinutes === 0 ? t("settings.manualOnly") : t("settings.every", { interval: labelFor(config.intervalMinutes) })}</span></header>
      <div class="segmented" role="group" aria-label="${t("settings.frequencyLabel")}">
        ${INTERVALS.map((option) => `<button class="segment ${config.intervalMinutes === option.minutes ? "is-active" : ""}"
          data-action="set-interval" data-minutes="${option.minutes}"
          aria-pressed="${config.intervalMinutes === option.minutes}">${t(option.label)}</button>`).join("")}
      </div>
    </section>

    <section class="card">
      <header class="card-head"><h2 class="card-title">${t("settings.onNewVersions")}</h2></header>
      <div class="radio-cards">
        ${AUTOMATION.map((option) => `<button class="radio-card ${config.automation === option.value ? "is-active" : ""}"
          data-action="set-automation" data-value="${option.value}" role="radio" aria-checked="${config.automation === option.value}">
          <span class="radio-dot" aria-hidden="true"></span>
          <span class="radio-title">${t(option.title)}</span>
          <span class="radio-body">${t(option.body)}</span>
        </button>`).join("")}
      </div>
    </section>

    <section class="card">
      <header class="card-head"><h2 class="card-title">${t("settings.behaviour")}</h2></header>
      <div class="switches">
        ${switchRow(t("settings.notifyTitle"), t("settings.notifyBody"), "notifyOnUpdate", config.notifyOnUpdate, "toggle-config")}
        ${switchRow(t("settings.doctorAfterUpdateTitle"), t("settings.doctorAfterUpdateBody"), "doctorAfterUpdate", config.doctorAfterUpdate, "toggle-config")}
        ${switchRow(t("settings.startMinimizedTitle"), t("settings.startMinimizedBody"), "startMinimized", config.startMinimized, "toggle-config")}
        ${switchRow(t("settings.autostartTitle"), t("settings.autostartBody"), "autostart", state.autostart, "toggle-autostart")}
      </div>
    </section>

    ${renderTranslationCard()}

    ${renderDoctorRulesCard()}

    ${renderIgnoredCard()}
  </section>`;
}

function switchRow(
  title: string,
  body: string,
  key: string,
  checked: boolean,
  action: string,
): string {
  return `<div class="switch-row">
    <div class="switch-copy"><span class="switch-title">${title}</span><span class="switch-body">${body}</span></div>
    <button class="switch ${checked ? "is-on" : ""}" role="switch" aria-checked="${checked}"
      aria-label="${title}" data-action="${action}" data-key="${key}">
      <span class="knob"></span>
    </button>
  </div>`;
}

function labelFor(minutes: number): string {
  return t(INTERVALS.find((option) => option.minutes === minutes)?.label ?? `${minutes} min`);
}

function renderIgnoredCard(): string {
  const all = items();
  const ignoredCount = all.filter((item) => item.ignored).length;

  const groups = GROUPS.map((group) => {
    const groupItems = all.filter((item) => item.kind === group.kind);
    if (groupItems.length === 0) return "";
    return `<div class="ignore-group">
      <div class="eyebrow group-label">${t(group.label)}</div>
      ${groupItems
        .map(
          (item) => `<div class="ignore-row">
            <span class="ignore-copy">
              <span class="ignore-name">${esc(item.name)}</span>
              <span class="ignore-source">${esc(sourceLabel(item.source))}</span>
            </span>
            <button class="switch ${item.ignored ? "is-on" : ""}" role="switch"
              aria-checked="${item.ignored}" aria-label="${t("action.ignoreName", { name: esc(item.name) })}"
              title="${item.ignored ? t("action.unignore") : t("action.ignore")}"
              data-action="toggle-ignored" data-id="${esc(item.id)}">
              <span class="knob"></span>
            </button>
          </div>`,
        )
        .join("")}
    </div>`;
  }).join("");

  return `<section class="card">
    <header class="card-head">
      <h2 class="card-title">${t("settings.ignoredTitle")}</h2>
      <span class="card-meta">${ignoredCount} ${t("settings.ignoredCount", { count: ignoredCount })}</span>
    </header>
    <p class="card-help">${t("settings.ignoredHelp")}</p>
    ${all.length === 0 ? `<div class="notes-empty"><p>${t("settings.ignoredEmpty")}</p></div>` : `<div class="ignore-groups">${groups}</div>`}
  </section>`;
}

function renderDoctorRulesCard(): string {
  const config = state.config;
  if (!config) return "";

  const known = new Set<string>([
    ...config.doctorIgnored,
    ...(state.doctor?.checks.map((check) => check.id) ?? []),
  ]);

  const rows = [...known]
    .sort()
    .map((id) => {
      const check = state.doctor?.checks.find((candidate) => candidate.id === id);
      const ignored = config.doctorIgnored.includes(id);
      return `<div class="ignore-row">
        <span class="ignore-copy">
          <span class="ignore-name mono">${esc(id)}</span>
          <span class="ignore-source">${check ? esc(check.message.slice(0, 100)) : t("settings.doctorRuleNoData")}</span>
        </span>
        <button class="switch ${ignored ? "is-on" : ""}" role="switch" aria-checked="${ignored}"
          aria-label="${t("action.ignoreName", { name: esc(id) })}" title="${ignored ? t("action.unignore") : t("action.ignore")}"
          data-action="toggle-doctor-ignore" data-id="${esc(id)}"><span class="knob"></span></button>
      </div>`;
    })
    .join("");

  return `<section class="card">
    <header class="card-head">
      <h2 class="card-title">${t("settings.doctorRules")}</h2>
      <span class="card-meta">${config.doctorIgnored.length} ${t("doctor.silencedCount", { count: config.doctorIgnored.length })}</span>
    </header>
    <p class="card-help">${t("settings.doctorRulesHelpBefore")} <span class="mono">gentle-ai doctor</span>. ${t("settings.doctorRulesHelpAfter")}</p>
    <div class="switches">
      <div class="switch-row">
        <div class="switch-copy">
          <span class="switch-title">${t("settings.ignoreToolsTitle")}</span>
          <span class="switch-body">${t("settings.ignoreToolsBodyBefore")} <span class="mono">tool:*</span> ${t("settings.ignoreToolsBodyAfter")}</span>
        </div>
        <button class="switch ${config.ignoreMissingTools ? "is-on" : ""}" role="switch"
          aria-checked="${config.ignoreMissingTools}" aria-label="${t("settings.ignoreToolsLabel")}"
          data-action="toggle-config" data-key="ignoreMissingTools"><span class="knob"></span></button>
      </div>
    </div>
    ${rows ? `<div class="ignore-groups">${rows}</div>` : `<div class="notes-empty"><p>${t("settings.doctorRulesEmpty")}</p></div>`}
  </section>`;
}

function renderTranslationCard(): string {
  const config = state.config;
  if (!config) return "";

  return `<section class="card">
    <header class="card-head">
      <h2 class="card-title">${t("settings.translationTitle")}</h2>
      <span class="card-meta">${config.translationEnabled ? t("settings.enabled") : t("settings.disabled")}</span>
    </header>
    <div class="switches">
      <div class="switch-row">
        <div class="switch-copy">
          <span class="switch-title">${t("settings.translateNotes")}</span>
          <span class="switch-body">${t("settings.translateNotesBody")}</span>
        </div>
        <button class="switch ${config.translationEnabled ? "is-on" : ""}" role="switch"
          aria-checked="${config.translationEnabled}" aria-label="${t("settings.translateNotes")}"
          data-action="toggle-config" data-key="translationEnabled"><span class="knob"></span></button>
      </div>
    </div>
  </section>`;
}

function renderNotesSearch(): string {
  const search = state.notesSearch;
  const results = search
    ? search.busy
      ? `<p class="search-status">${t("overview.searchBusy")}</p>`
      : search.hits.length === 0
        ? `<p class="search-status">${t("overview.searchNoHits")}</p>`
        : `<ul class="search-results">${search.hits
            .map(
              (hit) => `<li>
                <button class="search-hit" data-action="open-hit" data-key="${esc(hit.notesKey)}" data-version="${esc(hit.version)}">
                  <span class="search-hit-head">
                    <span class="search-hit-title">${esc(itemNameForNotesKey(hit.notesKey))}</span>
                    <span class="mono">${esc(hit.version)}</span>
                  </span>
                  <span class="search-hit-snippet">${esc(hit.snippet)}</span>
                </button>
              </li>`,
            )
            .join("")}</ul>`
    : "";

  return `<section class="card search-card">
    <div class="search-row">
      <input class="search-input" type="search" data-role="notes-search"
        placeholder="${t("overview.searchPlaceholder")}"
        value="${esc(search?.query ?? "")}" aria-label="${t("overview.searchLabel")}"/>
      <button class="btn btn-small" data-action="notes-search">${t("action.search")}</button>
      ${search ? `<button class="btn btn-ghost btn-small" data-action="notes-search-clear">${t("action.clear")}</button>` : ""}
    </div>
    ${results}
  </section>`;
}

function itemNameForNotesKey(notesKey: string): string {
  const match = items().find((item) => item.notesKey === notesKey);
  return match ? match.name : notesKey;
}
