import { expect, it } from 'vitest';
import { screenChanged, screenSignature } from './screenSignature';

it('counts a change only past the threshold, and unknown as changed', () => {
  const still = new Uint8Array([10, 200, 30]);
  expect(screenChanged(still, new Uint8Array([14, 195, 30]))).toBe(false);
  expect(screenChanged(still, new Uint8Array([10, 200, 60]))).toBe(true);
  expect(screenChanged(null, still)).toBe(true);
  expect(screenChanged(still, null)).toBe(true);
  expect(screenChanged(still, new Uint8Array([10, 200]))).toBe(true);
});

it('gives no fingerprint where the picture cannot be decoded', async () => {
  expect(await screenSignature('not base64 !')).toBeNull();
});
