import { describe, expect, it } from 'vitest';
import { sanitizeDetails } from './diagnostics';

describe('sanitizeDetails', () => {
  it('redacts credentials and bearer values', () => {
    expect(sanitizeDetails({ accessToken: 'abc', note: 'Bearer top-secret', rtt: 5 })).toEqual({
      accessToken: '[REDACTED]',
      note: 'Bearer [REDACTED]',
      rtt: 5
    });
  });
});
