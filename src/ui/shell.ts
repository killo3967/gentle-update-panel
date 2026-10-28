// Shell: titlebar, layout, event delegation, drawer and toasts.

import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  cancelPreflight,
  check,
  clearNotesSearch,
  closeContextMenu,
  confirmPreflight,
  dispose,
  init,
  items,
  loadNotes,
  moveSelection,
  openContextMenu,
  openHit,
  openUrl,
  pendingCount,
  requestApply,
  runDoctor,
  runPiHealth,
  saveConfig,
  select,
  selectAll,
  searchNotes,
  selectedIds,
  setState,
  setView,
  state,
  subscribe,
  toast,
  toggleAutostart,
  toggleDoctorIgnore,
  toggleIgnored,
  stepRelease,
  togglePreflightStep,
  toggleTranslation,
  toggleSelection,
  type View,
} from "../store";
import type { Config } from "../types";
import { esc, icons, plural } from "./format";
import { renderContent, renderRail } from "./views";

let drawerCollapsed = false;
let drawerDismissed = false;
let lastRail = "";
let lastContent = "";
let lastDrawer = "";
let lastToast = "";
let lastStatus = "";
let lastModal = "";
let lastContext = "";

export function mount(): void {
  const root = document.querySelector<HTMLDivElement>("#app");
  if (!root) return;

  root.innerHTML = `
    <div class="app">
      <header class="titlebar" data-tauri-drag-region>
        <div class="brand" data-tauri-drag-region>
          <span class="brand-mark">${icons.mark}</span>
          <span class="wordmark">Gentle <span class="wordmark-sep">·</span> Update Panel</span>
        </div>
        <div class="title-status" id="title-status" data-tauri-drag-region></div>
        <div class="win-controls">
          <button class="win-btn" data-action="win-minimize" aria-label="Minimizar">${icons.minimize}</button>
          <button class="win-btn" data-action="win-maximize" aria-label="Maximizar o restaurar">${icons.maximize}</button>
          <button class="win-btn win-close" data-action="win-close" aria-label="Cerrar a la bandeja">${icons.close}</button>
        </div>
      </header>

      <div class="progress-line" id="progress-line" aria-hidden="true"><span></span><em></em></div>

      <div class="layout">
        <div class="rail-host" id="rail"></div>
        <main class="content-host" id="content"></main>
      </div>

      <div id="modal-host"></div>
      <div id="context-host"></div>
      <div id="drawer-host"></div>
      <div id="toast-host" class="toasts" role="status" aria-live="polite"></div>
    </div>`;

  document.addEventListener("click", handleClick);
  document.addEventListener("change", handleChange);
  document.addEventListener("keydown", handleKeydown);
  document.addEventListener("contextmenu", handleContextMenu);
  window.addEventListener("beforeunload", dispose);

  subscribe(render);
  render();
  void init();
}

function render(): void {
  const rail = renderRail();
  const content = renderContent();
  const status = renderStatus();
  const drawer = renderDrawer();
  const modal = renderPreflight();
  const context = renderContextMenu();
  const toasts = renderToasts();

  if (rail !== lastRail) {
    const host = document.querySelector("#rail");
    if (host) {
      const before = host.querySelector<HTMLElement>(".pieces")?.scrollTop ?? 0;
      host.innerHTML = rail;
      const after = host.querySelector<HTMLElement>(".pieces");
      if (after) after.scrollTop = before;
    }
    lastRail = rail;
  }
  if (content !== lastContent) {
    const host = document.querySelector("#content");
    if (host) {
      const scroll = host.scrollTop;
      host.innerHTML = content;
      host.scrollTop = scroll;
    }
    lastContent = content;
  }
  if (status !== lastStatus) {
    const host = document.querySelector("#title-status");
    if (host) host.innerHTML = status;
    lastStatus = status;
  }
  if (drawer !== lastDrawer) {
    const host = document.querySelector("#drawer-host");
    if (host) {
      const log = host.querySelector<HTMLElement>(".drawer-log");
      const atBottom = !log || log.scrollHeight - log.scrollTop - log.clientHeight < 40;
      host.innerHTML = drawer;
      const next = host.querySelector<HTMLElement>(".drawer-log");
      if (next && atBottom) next.scrollTop = next.scrollHeight;
    }
    lastDrawer = drawer;
  }
  if (modal !== lastModal) {
    const host = document.querySelector("#modal-host");
    if (host) host.innerHTML = modal;
    lastModal = modal;
  }
  if (context !== lastContext) {
    const host = document.querySelector("#context-host");
    if (host) host.innerHTML = context;
    lastContext = context;
  }
  if (toasts !== lastToast) {
    const host = document.querySelector("#toast-host");
    if (host) host.innerHTML = toasts;
    lastToast = toasts;
  }

  const progress = document.querySelector<HTMLElement>("#progress-line");
  if (progress) {
    progress.classList.toggle("is-on", state.checking);
    const label = progress.querySelector("em");
    if (label) label.textContent = state.progress;
  }

  if (state.applying) {
    drawerDismissed = false;
  }
}

function renderStatus(): string {
  const pending = pendingCount();
  const tone = pending > 0 ? "accent" : state.snapshot ? "ok" : "faint";
  const label = state.checking
    ? "Comprobando"
    : pending > 0
      ? `${pending} ${plural(pending, "pendiente", "pendientes")}`
      : state.snapshot
        ? "Al día"
        : "Sin datos";

  return `<span class="status-pill tone--${tone}"><span class="status-dot"></span>${label}</span>`;
}

function renderDrawer(): string {
  if (drawerDismissed || (!state.applying && !state.outcome)) return "";

  const step = state.step;
  const title = state.applying
    ? step
      ? `Actualizando ${step.label.split(" · ")[0]} · ${step.index + 1}/${step.total}`
      : "Preparando la actualización"
    : state.outcome
      ? "Actualización terminada"
      : "Actividad";

  const percent = state.applying
    ? step
      ? Math.round(((step.index + 1) / Math.max(step.total, 1)) * 100)
      : 6
    : 100;

  const lines = state.log
    .slice(-400)
    .map((event) => {
      const modifier =
        event.stream === "command" ? "is-command" : event.stream === "done" ? "is-done" : "";
      return `<div class="log-line ${modifier}"><span class="log-id">${esc(event.id)}</span><span class="log-text">${esc(event.line)}</span></div>`;
    })
    .join("");

  const outcome = state.outcome;
  const summary = outcome
    ? `<div class="drawer-summary">
        <span class="tone--ok">${outcome.applied.length} ${plural(outcome.applied.length, "aplicada", "aplicadas")}</span>
        ${outcome.failed.length > 0 ? `<span class="tone--danger">${outcome.failed.length} ${plural(outcome.failed.length, "fallida", "fallidas")}</span>` : ""}
        ${outcome.failed.map((failure) => `<div class="failure"><strong>${esc(failure.label)}</strong><span>${esc(failure.reason)}</span></div>`).join("")}
      </div>`
    : `<span class="drawer-hint">${state.applying ? "No cierres la ventana hasta que termine." : ""}</span>`;

  return `<aside class="drawer ${drawerCollapsed ? "is-collapsed" : ""}" aria-label="Actividad de actualización">
    <header class="drawer-head">
      <div class="drawer-title"><span class="eyebrow">Actividad</span><strong>${esc(title)}</strong></div>
      <div class="drawer-tools">
        <button class="icon-btn" data-action="drawer-toggle" aria-label="${drawerCollapsed ? "Expandir" : "Plegar"}">${icons.chevron}</button>
        ${state.applying ? "" : `<button class="icon-btn" data-action="drawer-close" aria-label="Cerrar actividad">${icons.x}</button>`}
      </div>
    </header>
    <div class="bar"><span style="width:${percent}%"></span></div>
    ${drawerCollapsed ? "" : `<div class="drawer-log">${lines || '<div class="log-line"><span class="log-text">Esperando salida…</span></div>'}</div>`}
    <footer class="drawer-foot">${summary}</footer>
  </aside>`;
}

function renderToasts(): string {
  if (!state.toast) return "";
  return `<div class="toast toast--${state.toast.kind}"><span>${esc(state.toast.text)}</span></div>`;
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

async function handleClick(event: MouseEvent): Promise<void> {
  const target = event.target as HTMLElement | null;
  if (!target) return;

  // Any https link opens in the system browser, never inside the webview.
  const anchor = target.closest<HTMLAnchorElement>('a[href^="https://"]');
  if (anchor) {
    event.preventDefault();
    event.stopPropagation();
    closeContextMenu();
    await openUrl(anchor.getAttribute("href") ?? anchor.href);
    return;
  }

  if (state.contextMenu && !target.closest(".context-menu")) {
    closeContextMenu();
  }

  // Checkboxes toggle the batch, not the selected item.
  if (target.closest('input[type="checkbox"]')) return;

  const element = target.closest<HTMLElement>("[data-action]");
  if (!element) return;

  const action = element.dataset.action;
  const id = element.dataset.id;
  const currentWindow = getCurrentWindow();

  switch (action) {
    case "check":
      await check();
      break;
    case "view":
      setView((element.dataset.view as View) ?? "overview");
      break;
    case "select":
      if (id) select(id);
      break;
    case "back":
      select(null);
      break;
    case "apply-selected":
      await requestApply(selectedIds());
      break;
    case "apply-one":
      if (id) await requestApply([id]);
      break;
    case "release-prev":
      if (id) stepRelease(id, -1);
      break;
    case "release-next":
      if (id) stepRelease(id, 1);
      break;
    case "notes-search": {
      const input = document.querySelector<HTMLInputElement>('[data-role="notes-search"]');
      await searchNotes(input?.value ?? "");
      break;
    }
    case "notes-search-clear":
      clearNotesSearch();
      break;
    case "open-hit":
      openHit({
        notesKey: element.dataset.key ?? "",
        version: element.dataset.version ?? "",
        title: "",
        snippet: "",
      });
      break;
    case "context-open":
      if (state.contextMenu) await openUrl(state.contextMenu.href);
      closeContextMenu();
      break;
    case "context-copy":
      if (state.contextMenu) {
        try {
          await navigator.clipboard.writeText(state.contextMenu.href);
          toast("Dirección copiada", "ok");
        } catch {
          toast("No se pudo copiar", "error");
        }
      }
      closeContextMenu();
      break;
    case "preflight-cancel":
      cancelPreflight();
      break;
    case "preflight-confirm":
      await confirmPreflight();
      break;
    case "select-all":
      selectAll();
      break;
    case "toggle-ignored":
      if (id) toggleIgnored(id);
      break;
    case "toggle-doctor-ignore":
      if (id) await toggleDoctorIgnore(id);
      break;
    case "toggle-translation":
      await toggleTranslation();
      break;
    case "set-translation-engine":
      await saveConfig({
        translationEngine: (element.dataset.value as Config["translationEngine"]) ?? "auto",
      });
      break;
    case "set-translation-lang":
      await saveConfig({ translationLang: element.dataset.value ?? "es" });
      break;
    case "load-notes": {
      const item = items().find((candidate) => candidate.id === id);
      if (item) await loadNotes(item, true);
      break;
    }
    case "open-url":
      if (element.dataset.url) await openUrl(element.dataset.url);
      break;
    case "doctor":
      setView("doctor");
      await runDoctor();
      break;
    case "pi-health":
      await runPiHealth();
      break;
    case "warnings":
      setState({ warningsOpen: !state.warningsOpen });
      break;
    case "set-interval":
      await saveConfig({ intervalMinutes: Number(element.dataset.minutes ?? 360) });
      break;
    case "set-automation":
      await saveConfig({ automation: (element.dataset.value as Config["automation"]) ?? "notify" });
      break;
    case "toggle-config": {
      if (!state.config) break;
      const key = element.dataset.key as
        | "notifyOnUpdate"
        | "doctorAfterUpdate"
        | "startMinimized"
        | "ignoreMissingTools"
        | "translationEnabled";
      await saveConfig({ [key]: !state.config[key] } as Partial<Config>);
      break;
    }
    case "toggle-autostart":
      await toggleAutostart(!state.autostart);
      break;
    case "copy":
      if (element.dataset.text) {
        try {
          await navigator.clipboard.writeText(element.dataset.text);
          toast("Copiado al portapapeles", "ok");
        } catch {
          toast("No se pudo copiar", "error");
        }
      }
      break;
    case "drawer-toggle":
      drawerCollapsed = !drawerCollapsed;
      lastDrawer = "";
      render();
      break;
    case "drawer-close":
      drawerDismissed = true;
      setState({ outcome: null, log: [], step: null });
      break;
    case "win-minimize":
      await currentWindow.minimize();
      break;
    case "win-maximize":
      await currentWindow.toggleMaximize();
      break;
    case "win-close":
      await currentWindow.hide();
      break;
    default:
      break;
  }
}

function handleChange(event: Event): void {
  const target = event.target as HTMLElement | null;
  if (!target) return;

  const checkbox = target.closest<HTMLInputElement>('input[type="checkbox"][data-action="toggle"]');
  if (checkbox?.dataset.id) {
    toggleSelection(checkbox.dataset.id);
    return;
  }

  const preflight = target.closest<HTMLInputElement>("input[data-preflight-step]");
  if (preflight?.dataset.id) {
    togglePreflightStep(preflight.dataset.id);
  }
}

function handleContextMenu(event: MouseEvent): void {
  const target = event.target as HTMLElement | null;
  const anchor = target?.closest<HTMLAnchorElement>('a[href^="https://"]');
  if (anchor) {
    event.preventDefault();
    openContextMenu(
      Math.min(event.clientX, window.innerWidth - 250),
      Math.min(event.clientY, window.innerHeight - 96),
      anchor.href,
    );
    return;
  }

  // The native menu's "open link in new window" cannot work inside the webview.
  const editable = target?.closest("input, textarea");
  const hasSelection = (window.getSelection()?.toString() ?? "").length > 0;
  if (!editable && !hasSelection) {
    event.preventDefault();
    closeContextMenu();
  } else {
    closeContextMenu();
  }
}

function handleKeydown(event: KeyboardEvent): void {
  const active = document.activeElement;
  const typing =
    active instanceof HTMLInputElement ||
    active instanceof HTMLTextAreaElement ||
    active instanceof HTMLSelectElement;

  if (event.key === "Enter" && active instanceof HTMLInputElement && active.dataset.role === "notes-search") {
    event.preventDefault();
    void searchNotes(active.value);
    return;
  }

  if (event.key === "Escape") {
    if (state.contextMenu) {
      closeContextMenu();
      return;
    }
    if (state.preflight) {
      cancelPreflight();
      return;
    }
    if (state.outcome && !state.applying) {
      drawerDismissed = true;
      lastDrawer = "";
      render();
    }
    return;
  }

  if (typing) return;

  if (event.key === "ArrowDown") {
    event.preventDefault();
    moveSelection(1);
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    moveSelection(-1);
  }
}

function renderPreflight(): string {
  const report = state.preflight;
  if (!report) return "";

  const blockers = report.blockers;
  const selected = state.preflightSelected.size;

  const grouped = new Map<string, typeof blockers>();
  for (const blocker of blockers) {
    grouped.set(blocker.kind, [...(grouped.get(blocker.kind) ?? []), blocker]);
  }
  const blockerRows = [...grouped.entries()]
    .map(([kind, list]) => {
      const detail = list.map((blocker) => `${blocker.name} (${blocker.pid})`).join(" · ");
      const commands = list.map((blocker) => blocker.command).join(" | ");
      return `<li class="blocker-row">
        <span class="blocker-kind">${esc(kind)}</span>
        <span class="mono">${list.length} ${plural(list.length, "proceso", "procesos")}</span>
        <span class="blocker-cmd" title="${esc(commands)}">${esc(detail)}</span>
      </li>`;
    })
    .join("");

  const stepRows = report.steps
    .map((step) => {
      const checked = state.preflightSelected.has(step.id);
      const tone = step.impact === "block" ? "danger" : step.impact === "warn" ? "accent" : "ok";
      const label = step.impact === "block" ? "Bloquea" : step.impact === "warn" ? "Aviso" : "Sin impacto";
      return `<label class="preflight-step is-${step.impact}">
        <input type="checkbox" data-preflight-step data-id="${esc(step.id)}" ${checked ? "checked" : ""}/>
        <span class="step-box" aria-hidden="true"></span>
        <span class="step-copy">
          <span class="step-label">${esc(step.label)}</span>
          ${step.reason ? `<small>${esc(step.reason)}</small>` : ""}
        </span>
        <span class="pill pill--${tone}">${label}</span>
      </label>`;
    })
    .join("");

  return `<div class="modal-backdrop" role="presentation">
    <div class="modal" role="dialog" aria-modal="true" aria-labelledby="preflight-title">
      <header class="modal-head">
        <div>
          <span class="eyebrow">Antes de actualizar</span>
          <h2 id="preflight-title" class="display">Procesos en marcha</h2>
        </div>
        <button class="icon-btn" data-action="preflight-cancel" aria-label="Cancelar">${icons.x}</button>
      </header>
      <p class="lead">Se han detectado ${blockers.length} ${plural(blockers.length, "proceso", "procesos")}. Revisa qué se ve afectado antes de continuar.</p>
      <ul class="blocker-list">${blockerRows}</ul>
      <div class="preflight-steps">${stepRows}</div>
      <footer class="modal-foot">
        <button class="btn" data-action="preflight-cancel">Cancelar</button>
        <button class="btn btn-primary" data-action="preflight-confirm" ${selected === 0 ? "disabled" : ""}>
          Actualizar ${selected} ${plural(selected, "pieza", "piezas")}
        </button>
      </footer>
    </div>
  </div>`;
}

function renderContextMenu(): string {
  const menu = state.contextMenu;
  if (!menu) return "";

  return `<div class="context-menu" style="left:${menu.x}px;top:${menu.y}px" role="menu">
    <button role="menuitem" data-action="context-open">${icons.external}<span>Abrir enlace en el navegador</span></button>
    <button role="menuitem" data-action="context-copy">${icons.copy}<span>Copiar dirección</span></button>
  </div>`;
}
