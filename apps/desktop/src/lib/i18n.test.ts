import { get } from 'svelte/store';
import { afterEach, describe, expect, it } from 'vitest';
import { locale, setLocaleOverride, t, translateFor } from './i18n';
import { SAME_IN_VI, VI } from './i18n.vi';

const sources = import.meta.glob(['../**/*.svelte', '../**/*.ts', '!../**/*.test.ts', '!./i18n*.ts'], {
  query: '?raw',
  import: 'default',
  eager: true
}) as Record<string, string>;

function sourceKeys(): Set<string> {
  const keys = new Set<string>();
  const call = /(?:\$t|\btk|\btranslate)\(\s*'((?:[^'\\\n]|\\.)*)'/g;
  for (const text of Object.values(sources)) {
    for (const match of text.matchAll(call)) keys.add((match[1] ?? '').replace(/\\'/g, "'"));
  }
  return keys;
}

afterEach(() => setLocaleOverride('en'));

describe('i18n', () => {
  it('translates to Vietnamese and fills placeholders', () => {
    expect(translateFor('vi', 'Sign in')).toBe('Đăng nhập');
    expect(translateFor('vi', '{count} online', { count: 3 })).toBe('3 trực tuyến');
    expect(translateFor('en', '{count} online', { count: 3 })).toBe('3 online');
  });

  it('falls back to the source text for unknown strings such as server errors', () => {
    expect(translateFor('vi', 'Some server error')).toBe('Some server error');
    expect(translateFor('vi', 'Missing {value}')).toBe('Missing {value}');
  });

  it('switches the reactive translator with the locale', () => {
    setLocaleOverride('vi');
    expect(get(locale)).toBe('vi');
    expect(get(t)('Settings')).toBe('Cài đặt');
    setLocaleOverride('en');
    expect(get(t)('Settings')).toBe('Settings');
  });

  it('has a Vietnamese entry for every string the UI marks as translatable', () => {
    const keys = sourceKeys();
    expect(keys.size).toBeGreaterThan(500);
    const missing = [...keys].filter((key) => !(key in VI) && !SAME_IN_VI.has(key));
    expect(missing).toEqual([]);
  });

  it('keeps the same placeholders in every Vietnamese string', () => {
    const names = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
    for (const [english, vietnamese] of Object.entries(VI)) expect(names(vietnamese), english).toEqual(names(english));
  });
});
