import { derived, get, writable } from 'svelte/store';
import { preferences } from '../stores/preferences';
import { VI } from './i18n.vi';

export type Locale = 'vi' | 'en';
export type TranslateVars = Record<string, string | number>;
export type Translate = (text: string, vars?: TranslateVars) => string;

/**
 * Source strings are English and double as keys; `VI` maps them to Vietnamese.
 * A missing Vietnamese entry falls back to the English text, so the UI never
 * shows a raw key. `{name}` placeholders are filled from `vars`.
 */
// Component tests assert the English source strings; i18n.test.ts covers Vietnamese.
const override = writable<Locale | null>(import.meta.env.MODE === 'test' ? 'en' : null);

/** Forces a locale regardless of preferences (used by the test setup). */
export function setLocaleOverride(locale: Locale | null): void {
  override.set(locale);
}

function fill(text: string, vars?: TranslateVars): string {
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (match, key: string) => (key in vars ? String(vars[key]) : match));
}

export function translateFor(locale: Locale, text: string, vars?: TranslateVars): string {
  return fill(locale === 'vi' ? (VI[text] ?? text) : text, vars);
}

export const locale = derived([preferences, override], ([$preferences, $override]) => $override ?? $preferences.locale);

export const t = derived(locale, ($locale): Translate => (text, vars) => translateFor($locale, text, vars));

/** Non-reactive translation for code outside components (stores, error messages). */
export function translate(text: string, vars?: TranslateVars): string {
  return translateFor(get(locale), text, vars);
}

export async function setLocale(next: Locale): Promise<void> {
  const current = get(preferences);
  if (current.locale === next) return;
  await preferences.save({ ...current, locale: next });
}

/** Marks a source string defined outside markup as translatable; translate it where it is shown. */
export const tk = (text: string): string => text;
