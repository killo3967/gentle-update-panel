// Formatting, escaping, icons and the gauge — everything the views share.

import type { Status } from "../types";

export const esc = (value: string): string =>
  value.replace(/[&<>"']/g, (character) => {
    switch (character) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      default:
        return "&#39;";
    }
  });

export function relativeTime(iso: string | null): string {
  if (!iso) return "nunca";
  const stamp = Date.parse(iso);
  if (Number.isNaN(stamp)) return "nunca";

  const seconds = Math.round((Date.now() - stamp) / 1000);
  if (seconds < 45) return "ahora mismo";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `hace ${minutes} min`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `hace ${hours} h`;
  const days = Math.round(hours / 24);
  return `hace ${days} d`;
}

export function clockTime(iso: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleTimeString("es-ES", { hour: "2-digit", minute: "2-digit" });
}

export const ACTIONABLE: Status[] = ["updateAvailable", "missing"];

export function isActionable(status: Status): boolean {
  return ACTIONABLE.includes(status);
}

export interface StatusMeta {
  label: string;
  tone: "accent" | "ok" | "danger" | "faint";
}

export function statusMeta(status: Status): StatusMeta {
  switch (status) {
    case "updateAvailable":
      return { label: "Actualizable", tone: "accent" };
    case "missing":
      return { label: "Sin instalar", tone: "danger" };
    case "upToDate":
      return { label: "Al día", tone: "ok" };
    default:
      return { label: "Sin datos", tone: "faint" };
  }
}

export function doctorMeta(status: string): { label: string; tone: string } {
  switch (status.toLowerCase()) {
    case "ok":
    case "healthy":
      return { label: "Correcto", tone: "ok" };
    case "degraded":
    case "warn":
      return { label: "Degradado", tone: "accent" };
    case "failed":
    case "fail":
      return { label: "Con fallos", tone: "danger" };
    case "timeout":
      return { label: "Sin respuesta", tone: "danger" };
    default:
      return { label: "Sin datos", tone: "faint" };
  }
}

export function sourceLabel(source: string | null): string {
  if (!source) return "binario gestionado por gentle-ai";
  return source.replace(/^npm:/, "").replace(/^git:/, "");
}

export function delta(installed: string | null, latest: string | null): string {
  if (!installed && latest) return `→ ${latest}`;
  if (installed && latest && installed !== latest) return `${installed}→${latest}`;
  if (installed) return installed;
  return "—";
}

// ---------------------------------------------------------------------------
// Markdown (release notes)
// ---------------------------------------------------------------------------

export function markdownToHtml(markdown: string, repo?: string): string {
  const output: string[] = [];
  let inList = false;
  let inCode = false;

  const closeList = () => {
    if (inList) {
      output.push("</ul>");
      inList = false;
    }
  };

  for (const rawLine of markdown.split(/\r?\n/)) {
    const line = rawLine.trimEnd();

    if (line.trim().startsWith("```")) {
      closeList();
      output.push(inCode ? "</code></pre>" : "<pre><code>");
      inCode = !inCode;
      continue;
    }
    if (inCode) {
      output.push(esc(rawLine));
      continue;
    }

    if (line.trim() === "") {
      closeList();
      continue;
    }

    const bullet = line.match(/^\s*[-*]\s+(.*)$/);
    if (bullet) {
      if (!inList) {
        output.push("<ul>");
        inList = true;
      }
      output.push(`<li>${inlineMd(bullet[1], repo)}</li>`);
      continue;
    }
    closeList();

    const heading = line.match(/^(#{1,4})\s+(.*)$/);
    if (heading) {
      const level = Math.min(heading[1].length + 2, 6);
      output.push(`<h${level}>${inlineMd(heading[2], repo)}</h${level}>`);
      continue;
    }

    if (line.startsWith(">")) {
      output.push(`<blockquote>${inlineMd(line.replace(/^>\s?/, ""), repo)}</blockquote>`);
      continue;
    }

    output.push(`<p>${inlineMd(line, repo)}</p>`);
  }

  closeList();
  if (inCode) output.push("</code></pre>");
  return output.join("\n");
}

const anchor = (href: string, label: string): string =>
  `<a href="${href}" target="_blank" rel="noreferrer">${label}</a>`;

/**
 * Inline pass. Code spans and finished anchors are parked in placeholders so
 * later passes (bare URLs, `#123`, 40-char SHAs) cannot rewrite inside them.
 */
function inlineMd(text: string, repo?: string): string {
  const codes: string[] = [];
  const links: string[] = [];

  let html = esc(text);

  html = html.replace(/`([^`]+)`/g, (_match, code: string) => {
    codes.push(code);
    return `\u0000${codes.length - 1}\u0000`;
  });

  // Markdown links first, parked so the bare-URL pass cannot touch their href.
  html = html.replace(
    /\[([^\]]+)\]\((https:\/\/[^\s)]+)\)/g,
    (_match, label: string, url: string) => {
      links.push(anchor(url, label));
      return `\u0001${links.length - 1}\u0001`;
    },
  );

  html = html.replace(
    /(^|[\s(])(https:\/\/[^\s<\u0001]+)/g,
    (_match, prefix: string, url: string) => `${prefix}${anchor(url, url)}`,
  );

  // GitHub autolinks: issue/PR references and full commit ids.
  if (repo) {
    html = html.replace(
      /(^|[\s(])#(\d+)\b/g,
      (_match, prefix: string, id: string) =>
        `${prefix}${anchor(`https://github.com/${repo}/issues/${id}`, `#${id}`)}`,
    );
    html = html.replace(
      /\b([0-9a-f]{40})\b/g,
      (_match, sha: string) => anchor(`https://github.com/${repo}/commit/${sha}`, sha.slice(0, 7)),
    );
  }

  html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");

  html = html.replace(/\u0001(\d+)\u0001/g, (_match, index: string) => links[Number(index)] ?? "");
  html = html.replace(/\u0000(\d+)\u0000/g, (_match, index: string) =>
    `<code>${codes[Number(index)] ?? ""}</code>`,
  );

  return html;
}

/** `https://github.com/owner/repo/releases` → `owner/repo`. */
export function repositorySlug(url: string | null): string | undefined {
  if (!url) return undefined;
  const match = url.match(/github\.com\/([^/]+)\/([^/?#]+)/);
  if (!match) return undefined;
  return `${match[1]}/${match[2]}`;
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

const svg = (paths: string, size = 16, stroke = 1.5): string =>
  `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="${stroke}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;

export const icons = {
  mark: `<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none">
    <circle cx="12" cy="12" r="8.4" stroke="#f0a63c" stroke-width="2.1" stroke-linecap="round"
      stroke-dasharray="41 12" transform="rotate(-58 12 12)" />
    <circle cx="19.1" cy="7.4" r="2.1" fill="#f0a63c" />
  </svg>`,
  refresh: svg('<path d="M20 11a8 8 0 1 0-2.3 6.3"/><path d="M20 4v7h-7"/>'),
  overview: svg('<rect x="3" y="3" width="7" height="7" rx="1.4"/><rect x="14" y="3" width="7" height="7" rx="1.4"/><rect x="3" y="14" width="7" height="7" rx="1.4"/><rect x="14" y="14" width="7" height="7" rx="1.4"/>'),
  stethoscope: svg('<path d="M5 3v6a4 4 0 0 0 8 0V3"/><path d="M9 13v3a5 5 0 0 0 10 0v-2"/><circle cx="19" cy="11" r="2"/>'),
  sliders: svg('<path d="M4 6h10"/><path d="M18 6h2"/><circle cx="16" cy="6" r="2"/><path d="M4 12h4"/><path d="M12 12h8"/><circle cx="10" cy="12" r="2"/><path d="M4 18h12"/><path d="M20 18h0"/><circle cx="18" cy="18" r="2"/>'),
  external: svg('<path d="M14 4h6v6"/><path d="M20 4 11 13"/><path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4"/>', 14),
  arrowRight: svg('<path d="M5 12h14"/><path d="m13 6 6 6-6 6"/>', 14),
  check: svg('<path d="m4 12.5 5 5L20 6.5"/>', 14),
  alert: svg('<path d="M12 3 2.5 20h19L12 3Z"/><path d="M12 9v5"/><path d="M12 17h.01"/>', 14),
  copy: svg('<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/>', 14),
  chevron: svg('<path d="m6 9 6 6 6-6"/>', 14),
  chevronLeft: svg('<path d="m15 5-7 7 7 7"/>', 14),
  chevronRight: svg('<path d="m9 5 7 7-7 7"/>', 14),
  minimize: svg('<path d="M5 12h14"/>', 10, 1.6),
  maximize: svg('<rect x="5" y="5" width="14" height="14" rx="1.5"/>', 10, 1.6),
  close: svg('<path d="M6 6l12 12"/><path d="M18 6 6 18"/>', 10, 1.6),
  x: svg('<path d="M6 6l12 12"/><path d="M18 6 6 18"/>', 12, 1.6),
};

// ---------------------------------------------------------------------------
// Gauge — the visual anchor
// ---------------------------------------------------------------------------

export function gauge(status: Status, size: number, ticks = false): string {
  const stroke = size >= 120 ? 6 : 5;
  const radius = (size - stroke * 2) / 2 - 4;
  const center = size / 2;
  const circumference = 2 * Math.PI * radius;

  const meta = gaugeShape(status);
  const dash = (circumference * meta.fraction).toFixed(1);
  const gap = (circumference - Number(dash)).toFixed(1);

  const tickMarks = ticks
    ? Array.from({ length: 48 }, (_, index) => {
        const angle = (index / 48) * Math.PI * 2 - Math.PI / 2;
        const inner = radius + stroke + 2;
        const outer = inner + (index % 4 === 0 ? 6 : 3);
        const x1 = center + Math.cos(angle) * inner;
        const y1 = center + Math.sin(angle) * inner;
        const x2 = center + Math.cos(angle) * outer;
        const y2 = center + Math.sin(angle) * outer;
        return `<line x1="${x1.toFixed(1)}" y1="${y1.toFixed(1)}" x2="${x2.toFixed(1)}" y2="${y2.toFixed(1)}" stroke="currentColor" stroke-width="1" opacity=".28"/>`;
      }).join("")
    : "";

  const pulseDot =
    meta.dot && status === "updateAvailable"
      ? `<circle class="gauge-dot" cx="${center + Math.cos(meta.dotAngle) * radius}" cy="${center + Math.sin(meta.dotAngle) * radius}" r="${stroke * 1.15}" fill="var(--accent)"/>`
      : "";

  return `<svg class="gauge gauge--${meta.tone}" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}" role="img" aria-label="${esc(statusMeta(status).label)}">
    <g color="var(--line-strong)">${tickMarks}</g>
    <circle cx="${center}" cy="${center}" r="${radius}" fill="none" stroke="var(--panel-3)" stroke-width="${stroke}"/>
    <circle class="gauge-arc" cx="${center}" cy="${center}" r="${radius}" fill="none"
      stroke="currentColor" stroke-width="${stroke}" stroke-linecap="round"
      stroke-dasharray="${dash} ${gap}" transform="rotate(-90 ${center} ${center})"
      ${meta.dashed ? 'stroke-dasharray="2 7"' : ""} />
    ${pulseDot}
    <circle cx="${center}" cy="${center}" r="${radius - stroke - 6}" fill="none" stroke="currentColor" stroke-width="1" opacity=".18"/>
  </svg>`;
}

function gaugeShape(status: Status): {
  fraction: number;
  tone: string;
  dashed: boolean;
  dot: boolean;
  dotAngle: number;
} {
  switch (status) {
    case "updateAvailable":
      return { fraction: 0.72, tone: "accent", dashed: false, dot: true, dotAngle: -Math.PI / 2 + 0.72 * Math.PI * 2 };
    case "missing":
      return { fraction: 0.58, tone: "danger", dashed: true, dot: false, dotAngle: 0 };
    case "upToDate":
      return { fraction: 1, tone: "ok", dashed: false, dot: false, dotAngle: 0 };
    default:
      return { fraction: 0.25, tone: "faint", dashed: false, dot: false, dotAngle: 0 };
  }
}

export function counterTone(count: number): string {
  if (count === 0) return "ok";
  return "accent";
}

export function plural(count: number, one: string, many: string): string {
  return count === 1 ? one : many;
}
