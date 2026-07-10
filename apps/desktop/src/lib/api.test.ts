import { describe, expect, it } from 'vitest';
import { normalizeServerUrl } from './api';

describe('normalizeServerUrl', () => {
  it('allows HTTPS and loopback HTTP', () => {
    expect(normalizeServerUrl('https://sanser.example/')).toBe('https://sanser.example');
    expect(normalizeServerUrl('http://127.0.0.1:5174')).toBe('http://127.0.0.1:5174');
  });

  it('rejects insecure remote and credential-bearing URLs', () => {
    expect(() => normalizeServerUrl('http://example.test')).toThrow();
    expect(() => normalizeServerUrl('https://user:password@example.test')).toThrow();
  });
});
