import { invoke } from '@tauri-apps/api/core';
import { isTauriRuntime } from './platform';

// In-session chat and annotation. Messages travel over the native engine's
// authenticated SNV2 user-data lane; the host relays them to the other group
// viewers. History lives only in memory for the session. Chat is plain text:
// it is always rendered through Svelte text interpolation, never as HTML.

export type CollabKind = 'host' | 'client';
export interface CollabScope { kind: CollabKind; sessionId?: string }
export interface CollabEvent {
  seq: number;
  kind: 'chat' | 'stroke' | 'clear';
  /** 'self', 'host', '-' (the single remote peer) or a sharing viewer id. */
  from: string;
  name: string;
  own: boolean;
  text?: string;
  strokeId?: string;
  color?: number;
  points?: number[];
  end?: boolean;
  atMs: number;
}
export interface CollabSnapshot {
  available: boolean;
  host: boolean;
  peers: number;
  legacyPeers: number;
  annotations: boolean;
  cursor: number;
  events: CollabEvent[];
}
export type CollabOutgoing =
  | { type: 'chat'; name: string; text: string }
  | { type: 'stroke'; name: string; strokeId: string; color: number; points: number[]; end: boolean }
  | { type: 'clear'; name: string };

/** Must match the Rust hub (`collab.rs`). */
export const CHAT_BYTES = 1024;
export const STROKE_POINTS = 32;
export const COORD_MAX = 10_000;
export const POINTS_PER_SECOND = 120;
/** Strokes stay fully visible, then fade out. */
export const STROKE_HOLD_MS = 2500;
export const STROKE_FADE_MS = 1500;
export const PEN_COLORS = [
  { id: 0, label: 'Red', value: '#ff3b30' },
  { id: 1, label: 'Yellow', value: '#ffcc00' },
  { id: 2, label: 'Green', value: '#34c759' },
  { id: 3, label: 'Blue', value: '#0a84ff' },
  { id: 4, label: 'Purple', value: '#bf5af2' },
  { id: 5, label: 'White', value: '#ffffff' }
] as const;

const encoder = new TextEncoder();
export function utf8Bytes(text: string): number { return encoder.encode(text).length; }
/** Plain text, 1 KB, no control characters except newline and tab. */
export function validChat(text: string): boolean {
  // eslint-disable-next-line no-control-regex
  return text.trim().length > 0 && utf8Bytes(text) <= CHAT_BYTES && !/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(text);
}

export function collabSnapshot(scope: CollabScope, after: number): Promise<CollabSnapshot> {
  return invoke<CollabSnapshot>('collab_snapshot', { scope, after });
}
export function collabPost(scope: CollabScope, message: CollabOutgoing): Promise<number> {
  return invoke<number>('collab_post', { scope, message });
}
export function setGroupAnnotations(enabled: boolean): Promise<void> {
  return invoke('collab_set_annotations', { enabled });
}
export function openAnnotationOverlay(scope: CollabScope, locale: 'en' | 'vi'): Promise<void> {
  return invoke('open_annotation_overlay', { scope, locale });
}
export function closeAnnotationOverlay(): Promise<void> {
  return isTauriRuntime() ? invoke('close_annotation_overlay') : Promise.resolve();
}
export function setAnnotationPen(enabled: boolean, color: number, name: string): Promise<void> {
  return invoke('set_annotation_pen', { enabled, color, name });
}

/** Map a pointer position inside the overlay to fixed-point video coordinates. */
export interface Fit { x: number; y: number; width: number; height: number }
/** Aspect-fit video rectangle inside the overlay, matching the native presenters. */
export function videoFit(width: number, height: number, videoWidth: number, videoHeight: number): Fit {
  if (!videoWidth || !videoHeight || !width || !height) return { x: 0, y: 0, width, height };
  const scale = Math.min(width / videoWidth, height / videoHeight);
  const fitted = { width: videoWidth * scale, height: videoHeight * scale };
  return { x: (width - fitted.width) / 2, y: (height - fitted.height) / 2, ...fitted };
}
export function normalize(x: number, y: number, fit: Fit): [number, number] | null {
  if (fit.width <= 0 || fit.height <= 0) return null;
  const nx = (x - fit.x) / fit.width, ny = (y - fit.y) / fit.height;
  if (nx < -0.02 || ny < -0.02 || nx > 1.02 || ny > 1.02) return null;
  const clamp = (value: number) => Math.round(Math.min(1, Math.max(0, value)) * COORD_MAX);
  return [clamp(nx), clamp(ny)];
}
export function denormalize(x: number, y: number, fit: Fit): [number, number] {
  return [fit.x + (x / COORD_MAX) * fit.width, fit.y + (y / COORD_MAX) * fit.height];
}
export function strokeAlpha(ageMs: number): number {
  if (ageMs <= STROKE_HOLD_MS) return 1;
  return Math.max(0, 1 - (ageMs - STROKE_HOLD_MS) / STROKE_FADE_MS);
}

/**
 * Collects pen points, drops near-duplicates and flushes short segments
 * (at most STROKE_POINTS points) without exceeding POINTS_PER_SECOND.
 */
export class StrokeBuffer {
  private pending: number[] = [];
  private last: [number, number] | null = null;
  private tokens = POINTS_PER_SECOND;
  private at: number;
  constructor(readonly strokeId: string, now: number, private readonly minDistance = 20) { this.at = now; }
  add(point: [number, number]): void {
    if (this.last && Math.hypot(point[0] - this.last[0], point[1] - this.last[1]) < this.minDistance) return;
    this.last = point;
    if (this.pending.length / 2 >= STROKE_POINTS * 4) this.pending.splice(0, 2); // Bound memory while rate-limited.
    this.pending.push(point[0], point[1]);
  }
  /** Points to send now; `end` flushes what the rate allows and marks the stroke done. */
  take(now: number, end = false): { points: number[]; end: boolean } | null {
    this.tokens = Math.min(POINTS_PER_SECOND, this.tokens + ((now - this.at) / 1000) * POINTS_PER_SECOND);
    this.at = now;
    const allowed = Math.min(STROKE_POINTS, Math.floor(this.tokens), this.pending.length / 2);
    if (allowed <= 0) return null;
    // Receivers append segments of one stroke id, so segments join seamlessly.
    const points = this.pending.splice(0, allowed * 2);
    this.tokens -= allowed;
    return { points, end: end && this.pending.length === 0 };
  }
  get waiting(): boolean { return this.pending.length > 0; }
}

export function strokeId(): string {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}
