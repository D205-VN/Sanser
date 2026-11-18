import { describe, expect, it } from 'vitest';
import { CHAT_BYTES, COORD_MAX, denormalize, normalize, POINTS_PER_SECOND, STROKE_FADE_MS, STROKE_HOLD_MS, STROKE_POINTS, StrokeBuffer, strokeAlpha, strokeId, validChat, videoFit } from './collab';

describe('chat validation', () => {
  it('accepts bounded plain text and rejects empty, oversized or control text', () => {
    expect(validChat('Xin chào 👋\nsecond line')).toBe(true);
    expect(validChat('<b>tags are just text</b>')).toBe(true);
    expect(validChat('   ')).toBe(false);
    expect(validChat('a'.repeat(CHAT_BYTES))).toBe(true);
    expect(validChat('a'.repeat(CHAT_BYTES + 1))).toBe(false);
    expect(validChat('é'.repeat(CHAT_BYTES / 2 + 1))).toBe(false); // UTF-8 bytes, not characters.
    expect(validChat('bell\u0007')).toBe(false);
  });
});

describe('annotation geometry', () => {
  it('maps pointer positions in an aspect-fit video to fixed-point coordinates and back', () => {
    const fit = videoFit(1000, 1000, 1920, 1080);
    expect(fit.width).toBeCloseTo(1000);
    expect(fit.y).toBeCloseTo((1000 - 562.5) / 2);
    expect(normalize(500, 500, fit)).toEqual([5000, 5000]);
    expect(normalize(0, fit.y, fit)).toEqual([0, 0]);
    expect(normalize(500, 10, fit)).toBeNull(); // In the letterbox, outside the video.
    const [x, y] = denormalize(COORD_MAX, COORD_MAX, fit);
    expect(x).toBeCloseTo(1000);
    expect(y).toBeCloseTo(fit.y + fit.height);
    expect(videoFit(800, 600, 0, 0)).toEqual({ x: 0, y: 0, width: 800, height: 600 });
  });
  it('holds strokes, then fades them out', () => {
    expect(strokeAlpha(0)).toBe(1);
    expect(strokeAlpha(STROKE_HOLD_MS)).toBe(1);
    expect(strokeAlpha(STROKE_HOLD_MS + STROKE_FADE_MS / 2)).toBeCloseTo(0.5);
    expect(strokeAlpha(STROKE_HOLD_MS + STROKE_FADE_MS)).toBe(0);
  });
});

describe('stroke buffer', () => {
  it('sends short segments without exceeding the per-second point budget', () => {
    const buffer = new StrokeBuffer(strokeId(), 0, 0);
    for (let index = 0; index < 400; index += 1) buffer.add([index * 10, 100]);
    let sent = 0;
    for (let index = 0; index < 20; index += 1) {
      const segment = buffer.take(0);
      if (!segment) break;
      expect(segment.points.length / 2).toBeLessThanOrEqual(STROKE_POINTS);
      sent += segment.points.length / 2;
    }
    expect(sent).toBe(POINTS_PER_SECOND);
    expect(buffer.take(0)).toBeNull();
    // Budget refills with time; the end flag is set once the stroke drains.
    let ended = false;
    for (let at = 1000; at < 10_000 && !ended; at += 1000) {
      for (let index = 0; index < 8; index += 1) { const segment = buffer.take(at, true); if (segment?.end) ended = true; }
    }
    expect(ended).toBe(true);
    expect(buffer.waiting).toBe(false);
  });
  it('drops points closer than the minimum distance', () => {
    const buffer = new StrokeBuffer('s', 0, 20);
    buffer.add([0, 0]); buffer.add([5, 5]); buffer.add([30, 0]);
    expect(buffer.take(0)?.points).toEqual([0, 0, 30, 0]);
    expect(strokeId()).toMatch(/^[0-9a-f]{16}$/);
  });
});
