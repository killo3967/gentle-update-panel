// Text in the language the system uses. Dictionaries are flat `key -> text` maps:
// a missing key is a missing map entry, and every other dictionary is typed
// against the Spanish one, so `tsc` rejects an incomplete translation.
//
// Placeholders are `{name}` and are filled from the second argument. Plural
// entries carry one/other (plus many where the language needs it) and are picked
// with Intl.PluralRules, so a language with more than two forms still works.

import { de } from "./de";
import { en } from "./en";
import { es } from "./es";
import { fr } from "./fr";
import { it } from "./it";
import { pt } from "./pt";

export const LOCALES = ["es", "en", "pt", "fr", "it", "de"] as const;
export type Locale = (typeof LOCALES)[number];

export interface Plural {
  one: string;
  other: string;
  many?: string;
  few?: string;
  two?: string;
  zero?: string;
}

export type Entry = string | Plural;
export type Params = Record<string, string | number>;

/** The dictionary every other one is checked against.
 *
 * Mapped rather than `typeof es` so the Spanish literals stay narrow while the
 * other dictionaries may use any wording: what must match is the key set.
 */
export type SourceDict = { [K in keyof typeof es]: Entry };

// Typed against the Spanish key set: a translation that misses a key does not
// compile, so the six dictionaries cannot drift apart.
export const DICTIONARIES: Record<Locale, SourceDict> = { es, en, pt, fr, it, de };

const FALLBACK: Locale = "es";
let current: Locale = FALLBACK;

/** Language tag for Intl formatters, e.g. `es-ES`. */
const TAGS: Record<Locale, string> = {
  es: "es-ES",
  en: "en-GB",
  pt: "pt-PT",
  fr: "fr-FR",
  it: "it-IT",
  de: "de-DE",
};

export function locale(): Locale {
  return current;
}

/** Intl tag of the active language. */
export function localeTag(): string {
  return TAGS[current];
}

/**
 * `es-ES` → `es`. An unknown or empty tag keeps Spanish: the panel always has to
 * say something, and the source language is the one guaranteed to be complete.
 */
export function setLocale(tag: string | null | undefined): Locale {
  const base = (tag ?? "").trim().toLowerCase().split(/[-_]/)[0];
  current = (LOCALES as readonly string[]).includes(base) ? (base as Locale) : FALLBACK;
  return current;
}

function fill(text: string, params?: Params): string {
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in params ? String(params[name]) : match,
  );
}

function entry(key: string): Entry | undefined {
  // The dictionaries are keyed precisely, so a runtime lookup needs the wide form.
  const active = DICTIONARIES[current] as Record<string, Entry | undefined>;
  const fallback = DICTIONARIES[FALLBACK] as Record<string, Entry | undefined>;
  return active[key] ?? fallback[key];
}

function pluralForm(forms: Plural, count: number): string {
  const category = new Intl.PluralRules(TAGS[current]).select(count);
  const chosen = (forms as unknown as Record<string, string | undefined>)[category];
  return chosen ?? forms.other;
}

/** Plain lookup. Plural entries render their `other` form unless `count` is given. */
export function t(key: string, params?: Params): string {
  const found = entry(key);
  if (found === undefined) {
    // The key itself, so a missing translation is visible instead of silent.
    return key;
  }
  if (typeof found === "string") return fill(found, params);
  if (params && "count" in params) return fill(pluralForm(found, Number(params.count)), params);
  return fill(found.other, params);
}

/** Plural-aware lookup: `tn("notes.count", 3)` → "3 notas". */
export function tn(key: string, count: number, params?: Params): string {
  const merged: Params = { count, ...params };
  const found = entry(key);
  if (found === undefined) return key;
  if (typeof found === "string") return fill(found, merged);
  return fill(pluralForm(found, count), merged);
}

/**
 * Backend errors arrive as `key|name=value|name=value`; anything else is shown as
 * it came. That keeps the Rust side free of six copies of each message while the
 * numbers that matter stay in the text.
 */
export function translateError(error: unknown): string {
  const raw = typeof error === "string" ? error : String(error);
  if (!raw.startsWith("error.")) return raw;

  const [key, ...pairs] = raw.split("|");
  const params: Params = {};
  for (const pair of pairs) {
    const at = pair.indexOf("=");
    if (at > 0) params[pair.slice(0, at)] = pair.slice(at + 1);
  }
  return t(key, params);
}
