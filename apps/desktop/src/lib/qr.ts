/**
 * Minimal QR Code encoder (ISO/IEC 18004): byte mode, error correction level M.
 * Used to show the two-step verification otpauth:// URI without sending the
 * secret to any external QR service. Structure follows Project Nayuki's
 * reference implementation.
 */

const ECC_CODEWORDS_PER_BLOCK_M = [-1, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28];
const ERROR_CORRECTION_BLOCKS_M = [-1, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23, 25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49];
/** Format-information bits for level M. */
const ECL_M_FORMAT_BITS = 0;

export interface QrCode {
  version: number;
  size: number;
  mask: number;
  /** modules[y][x] is true for a dark module. */
  modules: boolean[][];
}

const bit = (value: number, index: number): boolean => ((value >>> index) & 1) !== 0;

function rawDataModules(version: number): number {
  let result = (16 * version + 128) * version + 64;
  if (version >= 2) {
    const alignments = Math.floor(version / 7) + 2;
    result -= (25 * alignments - 10) * alignments - 55;
    if (version >= 7) result -= 36;
  }
  return result;
}

function dataCodewords(version: number): number {
  return Math.floor(rawDataModules(version) / 8) - (ECC_CODEWORDS_PER_BLOCK_M[version] ?? 0) * (ERROR_CORRECTION_BLOCKS_M[version] ?? 0);
}

export function gfMultiply(x: number, y: number): number {
  let z = 0;
  for (let i = 7; i >= 0; i--) {
    z = (z << 1) ^ ((z >>> 7) * 0x11d);
    z ^= ((y >>> i) & 1) * x;
  }
  return z & 0xff;
}

function reedSolomonDivisor(degree: number): number[] {
  const result: number[] = new Array<number>(degree).fill(0);
  result[degree - 1] = 1;
  let root = 1;
  for (let i = 0; i < degree; i++) {
    for (let j = 0; j < result.length; j++) {
      result[j] = gfMultiply(result[j] ?? 0, root);
      if (j + 1 < result.length) result[j] = (result[j] ?? 0) ^ (result[j + 1] ?? 0);
    }
    root = gfMultiply(root, 0x02);
  }
  return result;
}

export function reedSolomonRemainder(data: readonly number[], degree: number): number[] {
  const divisor = reedSolomonDivisor(degree);
  const result: number[] = new Array<number>(degree).fill(0);
  for (const value of data) {
    const factor = value ^ (result.shift() ?? 0);
    result.push(0);
    divisor.forEach((coefficient, index) => { result[index] = (result[index] ?? 0) ^ gfMultiply(coefficient, factor); });
  }
  return result;
}

/** Data codewords (mode, count, payload, terminator, padding) for byte mode. */
export function encodeDataCodewords(bytes: Uint8Array, version: number): number[] {
  const bits: number[] = [];
  const append = (value: number, length: number) => { for (let i = length - 1; i >= 0; i--) bits.push((value >>> i) & 1); };
  append(0b0100, 4);
  append(bytes.length, version <= 9 ? 8 : 16);
  for (const byte of bytes) append(byte, 8);
  const capacity = dataCodewords(version) * 8;
  append(0, Math.min(4, capacity - bits.length));
  append(0, (8 - (bits.length % 8)) % 8);
  for (let pad = 0xec; bits.length < capacity; pad ^= 0xec ^ 0x11) append(pad, 8);
  const codewords: number[] = [];
  for (let i = 0; i < bits.length; i += 8) codewords.push(bits.slice(i, i + 8).reduce((value, next) => (value << 1) | next, 0));
  return codewords;
}

function addEccAndInterleave(data: readonly number[], version: number): number[] {
  const blocks = ERROR_CORRECTION_BLOCKS_M[version] ?? 1;
  const eccLength = ECC_CODEWORDS_PER_BLOCK_M[version] ?? 0;
  const raw = Math.floor(rawDataModules(version) / 8);
  const shortBlocks = blocks - (raw % blocks);
  const shortLength = Math.floor(raw / blocks);
  const all: number[][] = [];
  for (let i = 0, offset = 0; i < blocks; i++) {
    const length = shortLength - eccLength + (i < shortBlocks ? 0 : 1);
    const block = data.slice(offset, offset + length);
    offset += length;
    const ecc = reedSolomonRemainder(block, eccLength);
    if (i < shortBlocks) block.push(0);
    all.push([...block, ...ecc]);
  }
  const result: number[] = [];
  for (let i = 0; i < (all[0]?.length ?? 0); i++) {
    all.forEach((block, j) => { if (i !== shortLength - eccLength || j >= shortBlocks) result.push(block[i] ?? 0); });
  }
  return result;
}

export function formatBits(mask: number): number {
  const data = (ECL_M_FORMAT_BITS << 3) | mask;
  let remainder = data;
  for (let i = 0; i < 10; i++) remainder = (remainder << 1) ^ ((remainder >>> 9) * 0x537);
  return ((data << 10) | remainder) ^ 0x5412;
}

class Matrix {
  readonly size: number;
  readonly modules: boolean[][];
  readonly isFunction: boolean[][];

  constructor(readonly version: number) {
    this.size = version * 4 + 17;
    this.modules = Array.from({ length: this.size }, () => new Array<boolean>(this.size).fill(false));
    this.isFunction = Array.from({ length: this.size }, () => new Array<boolean>(this.size).fill(false));
  }

  set(x: number, y: number, dark: boolean): void {
    const row = this.modules[y];
    const functionRow = this.isFunction[y];
    if (!row || !functionRow) return;
    row[x] = dark;
    functionRow[x] = true;
  }

  drawFunctionPatterns(): void {
    for (let i = 0; i < this.size; i++) { this.set(6, i, i % 2 === 0); this.set(i, 6, i % 2 === 0); }
    this.finder(3, 3); this.finder(this.size - 4, 3); this.finder(3, this.size - 4);
    const positions = this.alignmentPositions();
    const count = positions.length;
    for (let i = 0; i < count; i++) {
      for (let j = 0; j < count; j++) {
        if ((i === 0 && j === 0) || (i === 0 && j === count - 1) || (i === count - 1 && j === 0)) continue;
        this.alignment(positions[i] ?? 0, positions[j] ?? 0);
      }
    }
    this.drawFormat(0);
    this.drawVersion();
  }

  private finder(x: number, y: number): void {
    for (let dy = -4; dy <= 4; dy++) {
      for (let dx = -4; dx <= 4; dx++) {
        const distance = Math.max(Math.abs(dx), Math.abs(dy));
        const xx = x + dx;
        const yy = y + dy;
        if (xx >= 0 && xx < this.size && yy >= 0 && yy < this.size) this.set(xx, yy, distance !== 2 && distance !== 4);
      }
    }
  }

  private alignment(x: number, y: number): void {
    for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) this.set(x + dx, y + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
  }

  private alignmentPositions(): number[] {
    if (this.version === 1) return [];
    const count = Math.floor(this.version / 7) + 2;
    const step = this.version === 32 ? 26 : Math.ceil((this.version * 4 + 4) / (count * 2 - 2)) * 2;
    const result = [6];
    for (let position = this.size - 7; result.length < count; position -= step) result.splice(1, 0, position);
    return result;
  }

  drawFormat(mask: number): void {
    const bits = formatBits(mask);
    for (let i = 0; i <= 5; i++) this.set(8, i, bit(bits, i));
    this.set(8, 7, bit(bits, 6));
    this.set(8, 8, bit(bits, 7));
    this.set(7, 8, bit(bits, 8));
    for (let i = 9; i < 15; i++) this.set(14 - i, 8, bit(bits, i));
    for (let i = 0; i < 8; i++) this.set(this.size - 1 - i, 8, bit(bits, i));
    for (let i = 8; i < 15; i++) this.set(8, this.size - 15 + i, bit(bits, i));
    this.set(8, this.size - 8, true);
  }

  private drawVersion(): void {
    if (this.version < 7) return;
    let remainder = this.version;
    for (let i = 0; i < 12; i++) remainder = (remainder << 1) ^ ((remainder >>> 11) * 0x1f25);
    const bits = (this.version << 12) | remainder;
    for (let i = 0; i < 18; i++) {
      const a = this.size - 11 + (i % 3);
      const b = Math.floor(i / 3);
      this.set(a, b, bit(bits, i));
      this.set(b, a, bit(bits, i));
    }
  }

  drawCodewords(data: readonly number[]): void {
    let index = 0;
    for (let right = this.size - 1; right >= 1; right -= 2) {
      if (right === 6) right = 5;
      for (let vertical = 0; vertical < this.size; vertical++) {
        for (let j = 0; j < 2; j++) {
          const x = right - j;
          const upward = ((right + 1) & 2) === 0;
          const y = upward ? this.size - 1 - vertical : vertical;
          const row = this.modules[y];
          if (row && !this.isFunction[y]?.[x] && index < data.length * 8) {
            row[x] = bit(data[index >>> 3] ?? 0, 7 - (index & 7));
            index++;
          }
        }
      }
    }
  }

  applyMask(mask: number): void {
    for (let y = 0; y < this.size; y++) {
      const row = this.modules[y];
      if (!row) continue;
      for (let x = 0; x < this.size; x++) {
        if (this.isFunction[y]?.[x]) continue;
        const invert = [
          (x + y) % 2 === 0,
          y % 2 === 0,
          x % 3 === 0,
          (x + y) % 3 === 0,
          (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0,
          ((x * y) % 2) + ((x * y) % 3) === 0,
          (((x * y) % 2) + ((x * y) % 3)) % 2 === 0,
          (((x + y) % 2) + ((x * y) % 3)) % 2 === 0
        ][mask];
        if (invert) row[x] = !row[x];
      }
    }
  }

  penalty(): number {
    let score = 0;
    const lines: string[] = [];
    for (let y = 0; y < this.size; y++) lines.push(this.modules[y]?.map((dark) => (dark ? '1' : '0')).join('') ?? '');
    for (let x = 0; x < this.size; x++) lines.push(this.modules.map((row) => (row[x] ? '1' : '0')).join(''));
    for (const line of lines) {
      for (const run of line.match(/0{5,}|1{5,}/g) ?? []) score += run.length - 2;
      const padded = `0000${line}0000`;
      for (let i = 0; i + 11 <= padded.length; i++) {
        const window = padded.slice(i, i + 11);
        if (window === '10111010000' || window === '00001011101') score += 40;
      }
    }
    let dark = 0;
    for (let y = 0; y < this.size; y++) {
      for (let x = 0; x < this.size; x++) {
        const color = this.modules[y]?.[x];
        if (color) dark++;
        if (x + 1 < this.size && y + 1 < this.size && color === this.modules[y]?.[x + 1] &&
            color === this.modules[y + 1]?.[x] && color === this.modules[y + 1]?.[x + 1]) score += 3;
      }
    }
    const total = this.size * this.size;
    score += (Math.ceil(Math.abs(dark * 20 - total * 10) / total) - 1) * 10;
    return score;
  }
}

/** Encodes `text` (UTF-8) at level M with the smallest version that fits. */
export function encodeQr(text: string): QrCode {
  const bytes = new TextEncoder().encode(text);
  let version = 1;
  while (version <= 40 && 4 + (version <= 9 ? 8 : 16) + bytes.length * 8 > dataCodewords(version) * 8) version++;
  if (version > 40) throw new Error('Text is too long for a QR code');
  const codewords = addEccAndInterleave(encodeDataCodewords(bytes, version), version);
  let best: Matrix | null = null;
  let bestMask = 0;
  let bestScore = Infinity;
  for (let mask = 0; mask < 8; mask++) {
    const matrix = new Matrix(version);
    matrix.drawFunctionPatterns();
    matrix.drawCodewords(codewords);
    matrix.applyMask(mask);
    matrix.drawFormat(mask);
    const score = matrix.penalty();
    if (score < bestScore) { best = matrix; bestMask = mask; bestScore = score; }
  }
  if (!best) throw new Error('Unable to build the QR code');
  return { version, size: best.size, mask: bestMask, modules: best.modules };
}

/** SVG path data (one unit per module, quiet zone included in the viewBox). */
export function qrPath(code: QrCode, border = 4): string {
  const parts: string[] = [];
  code.modules.forEach((row, y) => row.forEach((dark, x) => { if (dark) parts.push(`M${x + border},${y + border}h1v1h-1z`); }));
  return parts.join('');
}
