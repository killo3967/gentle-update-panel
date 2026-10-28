// Views: pure functions from state to markup. Events are delegated in shell.ts.

import {
  actionableItems,
  items,
  pendingCount,
  state,
  translationKey,
  updatesCount,
} from "../store";
import type {
  DoctorCheckView,
  HealthCheck,
  PackageHealth,
  Release,
  UpdateItem,
} from "../types";
import {
  clockTime,
  delta,
  esc,
  gauge,
  doctorMeta,
  icons,
  isActionable,
  markdownToHtml,
  plural,
  repositorySlug,
  relativeTime,
  sourceLabel,
  statusMeta,
} from "./format";

const GROUPS: Array<{ kind: UpdateItem["kind"]; label: string }> = [
  { kind: "ecosystem", label: "Ecosistema Gentle" },
  { kind: "pi", label: "pi" },
  { kind: "piPackage", label: "Paquetes de pi" },
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
      <div class="eyebrow group-label">${group.label}</div>
      ${groupItems.map(pieceRow).join("")}
    </div>`;
  }).join("");

  return `
  <aside class="rail">
    <div class="rail-top">
      <div class="eyebrow">Sistema</div>
      <div class="system-card">
        <div class="system-gauge">${gauge(pending > 0 ? "updateAvailable" : snapshot ? "upToDate" : "unknown", 78)}</div>
        <div class="system-numbers">
          <div class="big"><span class="accent">${pending}</span><span class="slash">/</span><span class="total">${total}</span></div>
          <div class="caption">${pending === 0 ? "todo al día" : plural(pending, "pieza pendiente", "piezas pendientes")}</div>
          <div class="stamp">Última · ${relativeTime(snapshot?.checkedAt ?? null)}</div>
        </div>
      </div>
      <button class="btn btn-primary btn-block" data-action="check" ${state.checking ? "disabled" : ""}>
        ${icons.refresh}
        <span>${state.checking ? "Comprobando…" : "Comprobar ahora"}</span>
      </button>
    </div>

    <nav class="nav" role="tablist" aria-label="Secciones">
      ${navItem("overview", "Resumen", icons.overview)}
      ${navItem("doctor", "Doctor", icons.stethoscope)}
      ${navItem("settings", "Ajustes", icons.sliders)}
    </nav>

    <div class="pieces">
      <div class="eyebrow pieces-label">Piezas</div>
      ${total === 0 ? `<p class="rail-empty">Sin datos todavía. Comprueba para descubrir qué hay instalado.</p>` : groups}
    </div>

    <div class="rail-foot">
      ${warnings.length > 0 ? `<button class="warn-link" data-action="warnings">${icons.alert}<span>${warnings.length} ${plural(warnings.length, "aviso", "avisos")}</span></button>` : `<span class="quiet">Sin avisos</span>`}
      <span class="version">v${esc(state.version || "0.1.0")}</span>
    </div>

    ${state.warningsOpen ? warningsPopover(warnings) : ""}
  </aside>`;
}

function navItem(view: "overview" | "doctor" | "settings", label: string, icon: string): string {
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
  const meta = ignored ? { label: "Ignorada", tone: "faint" as const } : statusMeta(item.status);
  const actionable = !ignored && isActionable(item.status);

  return `<div class="piece ${selected ? "is-selected" : ""} ${ignored ? "is-ignored" : ""}" role="button" tabindex="0"
      data-action="select" data-id="${esc(item.id)}" aria-current="${selected}">
    ${
      actionable
        ? `<span class="piece-check"><input type="checkbox" id="pick-${esc(item.id)}" data-action="toggle"
             data-id="${esc(item.id)}" ${checked ? "checked" : ""}
             aria-label="Seleccionar ${esc(item.name)}"/><label for="pick-${esc(item.id)}"></label></span>`
        : `<span class="piece-check is-empty"></span>`
    }
    <span class="dot dot--${meta.tone}" aria-hidden="true"></span>
    <span class="piece-name">${esc(item.name)}</span>
    <span class="delta" title="${esc(item.installed ?? "sin instalar")} → ${esc(item.latest ?? "?")}">${esc(delta(item.installed, item.latest))}</span>
  </div>`;
}

function warningsPopover(warnings: string[]): string {
  return `<div class="popover" role="dialog" aria-label="Avisos de la comprobación">
    <div class="popover-head">
      <span class="eyebrow">Avisos</span>
      <button class="icon-btn" data-action="warnings" aria-label="Cerrar avisos">${icons.x}</button>
    </div>
    <ul class="warn-list">${warnings.map((warning) => `<li>${esc(warning)}</li>`).join("")}</ul>
  </div>`;
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

export function renderContent(): string {
  if (state.view === "doctor") return renderDoctor();
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
        <h1 class="display">Sin datos todavía</h1>
        <p class="lead">${state.checking ? "Consultando el ecosistema…" : "Lanza una comprobación para ver qué hay instalado y qué se puede actualizar."}</p>
        <button class="btn btn-primary" data-action="check" ${state.checking ? "disabled" : ""}>${icons.refresh}<span>${state.checking ? "Comprobando…" : "Comprobar ahora"}</span></button>
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
        <h2 class="card-title">${group.label}</h2>
        <span class="card-meta">${pendingInGroup > 0 ? `${pendingInGroup} ${plural(pendingInGroup, "pendiente", "pendientes")}` : "al día"}</span>
      </header>
      <div class="rows">${groupItems.map(overviewRow).join("")}</div>
    </section>`;
  }).join("");

  return `<section class="content">
    <header class="view-head">
      <div class="view-head-copy">
        <div class="eyebrow">Resumen</div>
        <h1 class="display">${pending === 0 ? "Todo al día" : `${pending} ${plural(pending, "actualización disponible", "actualizaciones disponibles")}`}</h1>
        <p class="lead">Comprobado ${relativeTime(snapshot.checkedAt)} · ${clockTime(snapshot.checkedAt)} · ${snapshot.items.length} piezas · ${Math.max(1, Math.round(snapshot.durationMs / 1000))} s</p>
      </div>
      <div class="hero-gauge">${gauge(pending > 0 ? "updateAvailable" : "upToDate", 148, true)}</div>
    </header>

    <div class="toolbar">
      <button class="btn btn-primary" data-action="apply-selected" ${selection === 0 || state.applying ? "disabled" : ""}>
        ${icons.check}<span>Actualizar seleccionadas${selection > 0 ? ` (${selection})` : ""}</span>
      </button>
      <button class="btn" data-action="select-all" ${pending === 0 ? "disabled" : ""}>
        ${selection === actionableItems().length && pending > 0 ? "Quitar selección" : "Seleccionar todo"}
      </button>
      <span class="toolbar-hint">${updatesCount() > 0 ? `${updatesCount()} ${plural(updatesCount(), "versión nueva detectada", "versiones nuevas detectadas")}` : "Nada que instalar"}</span>
    </div>

    ${renderNotesSearch()}
    ${state.blockers.some((blocker) => blocker.kind === "pi") ? renderPiRunningBand() : ""}
    ${snapshot.warnings.length > 0 ? renderWarnBand(snapshot.warnings) : ""}
    ${groups}
  </section>`;
}

function overviewRow(item: UpdateItem): string {
  const ignored = item.ignored;
  const meta = ignored ? { label: "Ignorada", tone: "faint" as const } : statusMeta(item.status);
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
      ${actionable ? `<input type="checkbox" id="row-${esc(item.id)}" data-action="toggle" data-id="${esc(item.id)}" ${checked ? "checked" : ""} aria-label="Seleccionar ${esc(item.name)}"/><label for="row-${esc(item.id)}"></label>` : ""}
    </div>
    <button class="row-main" data-action="select" data-id="${esc(item.id)}">
      <span class="row-name">${esc(item.name)}</span>
      <span class="row-source">${esc(sourceLine)}</span>
    </button>
    <span class="row-versions">${versions}</span>
    <span class="pill pill--${meta.tone}">${meta.label}</span>
    ${actionable ? `<button class="btn btn-ghost btn-small" data-action="apply-one" data-id="${esc(item.id)}" ${state.applying ? "disabled" : ""}>Actualizar</button>` : ignored ? `<span class="row-ignored">${icons.x}</span>` : `<span class="row-ok">${icons.check}</span>`}
  </div>`;
}

function renderPiRunningBand(): string {
  return `<div class="warn-band">
    ${icons.alert}
    <div><strong>pi está en marcha</strong>
    <span>Actualiza pi o sus paquetes cuando cierres la sesión: ahora cambiarían bajo un proceso vivo.</span></div>
  </div>`;
}

function renderWarnBand(warnings: string[]): string {
  return `<div class="warn-band">
    ${icons.alert}
    <div><strong>La comprobación dejó ${warnings.length} ${plural(warnings.length, "aviso", "avisos")}</strong>
    <span>${esc(warnings.slice(0, 2).join(" · "))}${warnings.length > 2 ? "…" : ""}</span></div>
    <button class="btn btn-ghost btn-small" data-action="warnings">Ver</button>
  </div>`;
}

// ---------------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------------

function renderDetail(item: UpdateItem): string {
  const ignored = item.ignored;
  const meta = ignored ? { label: "Ignorada", tone: "faint" as const } : statusMeta(item.status);
  const actionable = !ignored && isActionable(item.status);
  const notes = state.notes[item.id];
  const loading = state.notesLoading[item.id];

  return `<section class="content detail">
    <header class="detail-head">
      <div class="detail-copy">
        <button class="btn btn-ghost btn-small back-link" data-action="back">← Volver a la lista</button>
        <div class="eyebrow">${item.kind === "ecosystem" ? "Ecosistema Gentle" : item.kind === "pi" ? "Núcleo" : "Paquete de pi"}</div>
        <h1 class="display detail-name">${esc(item.name)}</h1>
        <div class="detail-sub"><span class="mono">${esc(sourceLabel(item.source))}</span><span class="pill pill--${meta.tone}">${meta.label}</span></div>
        ${item.detail && !ignored ? `<p class="detail-note">${esc(item.detail)}</p>` : ""}
        ${ignored ? `<p class="detail-note">Está ignorada: no cuenta como pendiente, no avisa y no se actualiza. Puedes revertirlo en Ajustes.</p>` : ""}
      </div>
      <div class="detail-gauge">${gauge(item.status, 132, true)}</div>
    </header>

    <div class="versions">
      <div class="version-cell"><span class="version-label">instalada</span><span class="version-value">${esc(item.installed ?? "—")}</span></div>
      <span class="version-arrow">${icons.arrowRight}</span>
      <div class="version-cell"><span class="version-label">disponible</span><span class="version-value accent">${esc(item.latest ?? "—")}</span></div>
    </div>

    <div class="actions">
      ${actionable ? `<button class="btn btn-primary" data-action="apply-one" data-id="${esc(item.id)}" ${state.applying ? "disabled" : ""}>${icons.arrowRight}<span>${item.installed ? `Actualizar a ${esc(item.latest ?? "")}` : "Instalar"}</span></button>` : ""}
      <button class="btn" data-action="load-notes" data-id="${esc(item.id)}">${icons.refresh}<span>Ver notas</span></button>
      ${notes?.url ? `<button class="btn" data-action="open-url" data-url="${esc(notes.url)}">${icons.external}<span>Abrir en el navegador</span></button>` : ""}
      <button class="btn" data-action="doctor">${icons.stethoscope}<span>Doctor</span></button>
      <button class="btn" data-action="toggle-ignored" data-id="${esc(item.id)}">
        ${ignored ? icons.check : icons.x}<span>${ignored ? "Dejar de ignorar" : "Ignorar"}</span>
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
  if (loading && !notes) {
    return `<section class="card notes-card">
      <header class="card-head"><h2 class="card-title">Notas de versión</h2></header>
      <div class="notes-loading"><span class="spinner" aria-hidden="true"></span><span>Cargando notas de ${esc(item.name)}…</span></div>
    </section>`;
  }

  if (!notes) {
    return `<section class="card notes-card">
      <header class="card-head"><h2 class="card-title">Notas de versión</h2></header>
      <div class="notes-empty"><p>Aún no se han cargado las notas de esta pieza.</p>
      <button class="btn btn-small" data-action="load-notes" data-id="${esc(item.id)}">Cargar notas</button></div>
    </section>`;
  }

  const total = notes.releases.length;
  const index = Math.min(state.releaseIndex[item.id] ?? 0, Math.max(total - 1, 0));
  const release = notes.releases[index];
  const repo = repositorySlug(notes.url);

  const nav =
    total > 1
      ? `<div class="release-nav" role="group" aria-label="Versiones">
          <button class="release-arrow" data-action="release-prev" data-id="${esc(item.id)}"
            ${index <= 0 ? "disabled" : ""} aria-label="Versión más reciente" title="Versión más reciente">${icons.chevronLeft}</button>
          <span class="release-counter">${index + 1} / ${total}</span>
          <button class="release-arrow" data-action="release-next" data-id="${esc(item.id)}"
            ${index >= total - 1 ? "disabled" : ""} aria-label="Versión anterior" title="Versión anterior">${icons.chevronRight}</button>
        </div>`
      : "";

  return `<section class="card notes-card">
    <header class="card-head">
      <h2 class="card-title">Notas de la versión ${esc(release?.version ?? "")}</h2>
      <div class="card-tools">
        ${nav}
        <button class="btn btn-small ${state.config?.translationEnabled ? "btn-primary" : ""}" data-action="toggle-translation">
          ${state.config?.translationEnabled ? "Ver original" : "Traducir"}
        </button>
        <button class="icon-btn" data-action="load-notes" data-id="${esc(item.id)}" aria-label="Recargar notas" title="Recargar">${icons.refresh}</button>
      </div>
    </header>
    ${
      notes.error
        ? `<div class="notes-empty"><p>${esc(notes.error)}</p>${notes.url ? `<button class="btn btn-small" data-action="open-url" data-url="${esc(notes.url)}">Abrir la ficha del proyecto</button>` : ""}</div>`
        : total === 0
          ? `<div class="notes-empty"><p>No hay notas publicadas para esta pieza.</p></div>`
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
    ? `<span class="pill pill--accent">Traducido · ${esc(engineLabel(translated.engine))}</span>`
    : busy
      ? `<span class="pill pill--faint">Traduciendo…</span>`
      : "";

  return `<article class="release">
    <header class="release-head">
      <span class="release-version">${esc(release.version)}</span>
      ${release.date ? `<span class="release-date">${esc(release.date)}</span>` : ""}
      ${badge}
    </header>
    <div class="md">${body}</div>
    ${error ? `<p class="translation-error">No se pudo traducir: ${esc(error)}</p>` : ""}
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
          ? `${view.ignoredCount} ${plural(view.ignoredCount, "silenciada", "silenciadas")}`
          : "",
        view.originalStatus !== view.status ? `el CLI dijo ${view.originalStatus}` : "",
      ]
        .filter(Boolean)
        .join(" · ")
    : "Ejecuta gentle-ai doctor para revisar binarios, estado y assets instalados.";

  const head = `<header class="view-head">
    <div class="view-head-copy">
      <div class="eyebrow">Diagnóstico</div>
      <h1 class="display">${view ? meta.label : "Estado del ecosistema"}</h1>
      <p class="lead">${esc(lead)}</p>
    </div>
    <div class="hero-gauge"><span class="doctor-badge tone--${view ? meta.tone : "faint"}">${view ? meta.label : "Sin datos"}</span></div>
  </header>`;

  const controls = `<div class="toolbar">
    <button class="btn btn-primary" data-action="doctor" ${state.doctorRunning ? "disabled" : ""}>${icons.stethoscope}<span>${state.doctorRunning ? "Ejecutando…" : "Ejecutar doctor"}</span></button>
    ${view ? `<span class="toolbar-hint">${active.length} ${plural(active.length, "comprobación activa", "comprobaciones activas")}${view.ignoredCount > 0 ? ` · ${view.ignoredCount} silenciada(s)` : ""}</span>` : ""}
  </div>`;

  if (!view) {
    return `<section class="content">${head}${controls}
      <div class="empty-state small">${icons.stethoscope}<p class="lead">Sin resultados todavía.</p></div>
    </section>`;
  }

  const checks = active.map(doctorRow).join("");
  const hiddenBlock = hidden.length
    ? `<details class="raw-block ignored-checks">
        <summary>${hidden.length} ${plural(hidden.length, "comprobación silenciada", "comprobaciones silenciadas")}</summary>
        <div class="checks">${hidden.map(doctorRow).join("")}</div>
      </details>`
    : "";

  return `<section class="content">${head}${controls}
    <section class="card"><div class="checks">${checks || `<div class="notes-empty"><p>Todas las comprobaciones están silenciadas. Revísalas en Ajustes → Reglas del doctor.</p></div>`}</div></section>
    ${hiddenBlock}
    ${renderPiHealth()}
    <details class="raw-block">
      <summary>Salida completa</summary>
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
        <button class="btn btn-ghost btn-small" data-action="copy" data-text="${esc(remedy)}">${icons.copy}<span>Copiar</span></button></div>` : ""}
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
    <h2 class="card-title">Salud de pi y sus paquetes</h2>
    <div class="card-tools">
      ${health ? `<span class="card-meta">${health.packages.length} piezas · ${health.packages.filter((pkg) => pkg.status !== "ok").length} con problemas · ${esc(relativeTime(health.checkedAt))}</span>` : ""}
      <button class="btn btn-small" data-action="pi-health" ${state.piHealthRunning ? "disabled" : ""}>
        ${state.piHealthRunning ? "Comprobando…" : health ? "Recomprobar" : "Comprobar instalación"}
      </button>
    </div>
  </header>`;

  if (!health) {
    return `<section class="card">${header}
      <div class="notes-empty"><p>Verifica que cada paquete existe en disco, que su manifiesto se lee, que declara versión y que sus recursos están donde dice.</p></div>
    </section>`;
  }

  const packages = health.packages.map(packageBlock).join("");
  return `<section class="card">
    ${header}
    <div class="checks">${health.checks.map(healthRow).join("")}</div>
    <div class="pkg-list">${packages || `<div class="notes-empty"><p>pi no declara paquetes.</p></div>`}</div>
  </section>`;
}

function packageBlock(pkg: PackageHealth): string {
  const tone = healthTone(pkg.status);
  const label = pkg.status === "ok" ? "Correcto" : pkg.status === "warn" ? "Revisar" : "Con fallos";
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
// Settings
// ---------------------------------------------------------------------------

const INTERVALS: Array<{ minutes: number; label: string }> = [
  { minutes: 15, label: "15 min" },
  { minutes: 60, label: "1 h" },
  { minutes: 180, label: "3 h" },
  { minutes: 360, label: "6 h" },
  { minutes: 720, label: "12 h" },
  { minutes: 1440, label: "24 h" },
  { minutes: 0, label: "Manual" },
];

const AUTOMATION: Array<{ value: "notify" | "confirm" | "auto"; title: string; body: string }> = [
  { value: "notify", title: "Avisar", body: "Notificación y distintivo en la bandeja. No instala nada." },
  { value: "confirm", title: "Confirmar", body: "Abre el panel con el lote preparado y espera tu clic." },
  { value: "auto", title: "Automático", body: "Aplica las novedades en cuanto las detecta. Después pasa el doctor." },
];

function renderSettings(): string {
  const config = state.config;
  if (!config) return `<section class="content"><div class="empty-state"><p class="lead">Cargando ajustes…</p></div></section>`;

  return `<section class="content settings">
    <header class="view-head">
      <div class="view-head-copy">
        <div class="eyebrow">Ajustes</div>
        <h1 class="display">Cómo vigila</h1>
        <p class="lead">La comprobación corre en segundo plano; aquí decides cada cuánto y qué hace al encontrar novedades.</p>
      </div>
    </header>

    <section class="card">
      <header class="card-head"><h2 class="card-title">Frecuencia</h2><span class="card-meta">${config.intervalMinutes === 0 ? "solo manual" : `cada ${labelFor(config.intervalMinutes)}`}</span></header>
      <div class="segmented" role="group" aria-label="Frecuencia de comprobación">
        ${INTERVALS.map((option) => `<button class="segment ${config.intervalMinutes === option.minutes ? "is-active" : ""}"
          data-action="set-interval" data-minutes="${option.minutes}"
          aria-pressed="${config.intervalMinutes === option.minutes}">${option.label}</button>`).join("")}
      </div>
    </section>

    <section class="card">
      <header class="card-head"><h2 class="card-title">Al encontrar novedades</h2></header>
      <div class="radio-cards">
        ${AUTOMATION.map((option) => `<button class="radio-card ${config.automation === option.value ? "is-active" : ""}"
          data-action="set-automation" data-value="${option.value}" role="radio" aria-checked="${config.automation === option.value}">
          <span class="radio-dot" aria-hidden="true"></span>
          <span class="radio-title">${option.title}</span>
          <span class="radio-body">${option.body}</span>
        </button>`).join("")}
      </div>
    </section>

    <section class="card">
      <header class="card-head"><h2 class="card-title">Comportamiento</h2></header>
      <div class="switches">
        ${switchRow("Avisar con notificación", "Un toast de Windows cuando aparezca una versión nueva.", "notifyOnUpdate", config.notifyOnUpdate, "toggle-config")}
        ${switchRow("Ejecutar doctor tras actualizar", "Verifica la instalación en cuanto termina el lote.", "doctorAfterUpdate", config.doctorAfterUpdate, "toggle-config")}
        ${switchRow("Arrancar minimizado", "Al iniciar con Windows, directo a la bandeja.", "startMinimized", config.startMinimized, "toggle-config")}
        ${switchRow("Arrancar con Windows", "Registro de usuario (HKCU). No requiere permisos de administrador.", "autostart", state.autostart, "toggle-autostart")}
      </div>
    </section>

    ${renderTranslationCard()}

    ${renderDoctorRulesCard()}

    ${renderIgnoredCard()}

    <section class="card">
      <header class="card-head"><h2 class="card-title">Datos</h2></header>
      <dl class="data-list">
        <div><dt>Configuración</dt><dd class="mono">${esc(state.appDir || "—")}</dd></div>
        <div><dt>Versión del panel</dt><dd class="mono">v${esc(state.version || "—")}</dd></div>
        <div><dt>Comprobación</dt><dd class="mono">${state.checking ? "en curso" : relativeTime(state.snapshot?.checkedAt ?? null)}</dd></div>
      </dl>
      <div class="card-actions">
        <button class="btn" data-action="check" ${state.checking ? "disabled" : ""}>${icons.refresh}<span>Comprobar ahora</span></button>
        <button class="btn btn-ghost" data-action="doctor">${icons.stethoscope}<span>Ejecutar doctor</span></button>
      </div>
    </section>
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
  return INTERVALS.find((option) => option.minutes === minutes)?.label ?? `${minutes} min`;
}

function renderIgnoredCard(): string {
  const all = items();
  const ignoredCount = all.filter((item) => item.ignored).length;

  const groups = GROUPS.map((group) => {
    const groupItems = all.filter((item) => item.kind === group.kind);
    if (groupItems.length === 0) return "";
    return `<div class="ignore-group">
      <div class="eyebrow group-label">${group.label}</div>
      ${groupItems
        .map(
          (item) => `<div class="ignore-row">
            <span class="ignore-copy">
              <span class="ignore-name">${esc(item.name)}</span>
              <span class="ignore-source">${esc(sourceLabel(item.source))}</span>
            </span>
            <button class="switch ${item.ignored ? "is-on" : ""}" role="switch"
              aria-checked="${item.ignored}" aria-label="Ignorar ${esc(item.name)}"
              title="${item.ignored ? "Dejar de ignorar" : "Ignorar"}"
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
      <h2 class="card-title">Piezas ignoradas</h2>
      <span class="card-meta">${ignoredCount} ${plural(ignoredCount, "ignorada", "ignoradas")}</span>
    </header>
    <p class="card-help">Se siguen listando para que puedas revertirlo, pero no cuentan como pendientes, no avisan y no se actualizan. Tampoco se consulta su versión publicada.</p>
    ${all.length === 0 ? `<div class="notes-empty"><p>Comprueba primero para ver las piezas instaladas.</p></div>` : `<div class="ignore-groups">${groups}</div>`}
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
          <span class="ignore-source">${check ? esc(check.message.slice(0, 100)) : "sin datos en el último informe"}</span>
        </span>
        <button class="switch ${ignored ? "is-on" : ""}" role="switch" aria-checked="${ignored}"
          aria-label="Ignorar ${esc(id)}" title="${ignored ? "Dejar de ignorar" : "Ignorar"}"
          data-action="toggle-doctor-ignore" data-id="${esc(id)}"><span class="knob"></span></button>
      </div>`;
    })
    .join("");

  return `<section class="card">
    <header class="card-head">
      <h2 class="card-title">Reglas del doctor</h2>
      <span class="card-meta">${config.doctorIgnored.length} ${plural(config.doctorIgnored.length, "silenciada", "silenciadas")}</span>
    </header>
    <p class="card-help">Silencia comprobaciones concretas de <span class="mono">gentle-ai doctor</span>. Dejan de contar para el estado, pero siguen consultables en un desplegable.</p>
    <div class="switches">
      <div class="switch-row">
        <div class="switch-copy">
          <span class="switch-title">Ignorar herramientas deshabilitadas</span>
          <span class="switch-body">Si el ecosistema informa que una herramienta no está instalada (por ejemplo, gga deshabilitado), se oculta en Resumen y su check <span class="mono">tool:*</span> deja de contar en Doctor.</span>
        </div>
        <button class="switch ${config.ignoreMissingTools ? "is-on" : ""}" role="switch"
          aria-checked="${config.ignoreMissingTools}" aria-label="Ignorar herramientas no instaladas"
          data-action="toggle-config" data-key="ignoreMissingTools"><span class="knob"></span></button>
      </div>
    </div>
    ${rows ? `<div class="ignore-groups">${rows}</div>` : `<div class="notes-empty"><p>Ejecuta el doctor una vez para listar sus comprobaciones.</p></div>`}
  </section>`;
}

function renderTranslationCard(): string {
  const config = state.config;
  if (!config) return "";

  const engines: Array<{ value: "auto" | "google" | "microsoft"; label: string }> = [
    { value: "auto", label: "Automático" },
    { value: "google", label: "Google" },
    { value: "microsoft", label: "Microsoft" },
  ];
  const languages = [
    { value: "es", label: "Español" },
    { value: "en", label: "Inglés" },
    { value: "fr", label: "Francés" },
    { value: "de", label: "Alemán" },
    { value: "pt", label: "Portugués" },
    { value: "it", label: "Italiano" },
  ];

  return `<section class="card">
    <header class="card-head">
      <h2 class="card-title">Traducción de notas</h2>
      <span class="card-meta">${config.translationEnabled ? "activada" : "desactivada"}</span>
    </header>
    <p class="card-help">Traduce las notas de versión bajo demanda y las guarda en caché. El código, las rutas y los enlaces se protegen antes de enviarlos. Son endpoints gratuitos no oficiales: si Google falla, se usa Microsoft.</p>
    <div class="switches">
      <div class="switch-row">
        <div class="switch-copy">
          <span class="switch-title">Traducir las notas</span>
          <span class="switch-body">Al abrir las notas de una pieza verás el botón Traducir / Ver original.</span>
        </div>
        <button class="switch ${config.translationEnabled ? "is-on" : ""}" role="switch"
          aria-checked="${config.translationEnabled}" aria-label="Traducir las notas"
          data-action="toggle-config" data-key="translationEnabled"><span class="knob"></span></button>
      </div>
    </div>
    <div class="segmented" role="group" aria-label="Motor de traducción">
      ${engines.map((engine) => `<button class="segment ${config.translationEngine === engine.value ? "is-active" : ""}"
        data-action="set-translation-engine" data-value="${engine.value}"
        aria-pressed="${config.translationEngine === engine.value}">${engine.label}</button>`).join("")}
    </div>
    <div class="segmented" role="group" aria-label="Idioma de destino">
      ${languages.map((language) => `<button class="segment ${config.translationLang === language.value ? "is-active" : ""}"
        data-action="set-translation-lang" data-value="${language.value}"
        aria-pressed="${config.translationLang === language.value}">${language.label}</button>`).join("")}
    </div>
  </section>`;
}

function renderNotesSearch(): string {
  const search = state.notesSearch;
  const results = search
    ? search.busy
      ? `<p class="search-status">Buscando…</p>`
      : search.hits.length === 0
        ? `<p class="search-status">Sin coincidencias en las notas ya descargadas.</p>`
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
        placeholder="Buscar en las notas de todos los productos…"
        value="${esc(search?.query ?? "")}" aria-label="Buscar en las notas"/>
      <button class="btn btn-small" data-action="notes-search">Buscar</button>
      ${search ? `<button class="btn btn-ghost btn-small" data-action="notes-search-clear">Limpiar</button>` : ""}
    </div>
    ${results}
  </section>`;
}

function itemNameForNotesKey(notesKey: string): string {
  const match = items().find((item) => item.notesKey === notesKey);
  return match ? match.name : notesKey;
}
