import { describe, expect, it } from 'vitest';
import { encodeDataCodewords, encodeQr, formatBits, qrPath, reedSolomonRemainder } from './qr';

describe('QR encoder', () => {
  it('computes Reed-Solomon error correction like the published 1-M example', () => {
    const data = [32, 91, 11, 120, 209, 114, 220, 77, 67, 64, 236, 17, 236, 17, 236, 17];
    expect(reedSolomonRemainder(data, 10)).toEqual([196, 35, 39, 119, 235, 215, 231, 226, 93, 23]);
  });

  it('produces the standard level-M format information', () => {
    expect(formatBits(0)).toBe(0b101010000010010);
    expect(formatBits(5)).toBe(0b100000011001110);
  });

  it('encodes byte mode with terminator and alternating padding', () => {
    expect(encodeDataCodewords(new TextEncoder().encode('A'), 1)).toEqual([
      0x40, 0x14, 0x10, 0xec, 0x11, 0xec, 0x11, 0xec, 0x11, 0xec, 0x11, 0xec, 0x11, 0xec, 0x11, 0xec
    ]);
  });

  it('draws finder, timing and dark-module patterns for an otpauth URI', () => {
    const uri = 'otpauth://totp/Sanser:someone%40example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Sanser&algorithm=SHA1&digits=6&period=30';
    const code = encodeQr(uri);
    expect(code.size).toBe(code.version * 4 + 17);
    expect(code.version).toBeGreaterThanOrEqual(6);
    const at = (x: number, y: number) => code.modules[y]?.[x];
    for (const [x, y] of [[0, 0], [code.size - 7, 0], [0, code.size - 7]] as const) {
      expect(at(x, y)).toBe(true);
      expect(at(x + 1, y + 1)).toBe(false);
      expect(at(x + 3, y + 3)).toBe(true);
    }
    for (let i = 8; i < code.size - 8; i++) expect(at(i, 6)).toBe(i % 2 === 0);
    expect(at(8, code.size - 8)).toBe(true);
    expect(qrPath(code)).toContain('M4,4h1v1h-1z');
  });
});
