<script lang="ts">
  import { t } from '../lib/i18n';
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { denormalize, normalize, PEN_COLORS, STROKE_FADE_MS, STROKE_HOLD_MS, StrokeBuffer, strokeAlpha, strokeId, videoFit, type CollabEvent, type Fit } from '../lib/collab';

  // Transparent, click-through and excluded from screen capture by the shell.
  // It only polls strokes and sends its own pen input.
  interface Poll { kind: 'host' | 'client'; pen: boolean; color: number; annotations: boolean; videoWidth: number; videoHeight: number; cursor: number; events: CollabEvent[] }
  interface Stroke { key: string; color: number; points: number[]; updated: number }
  let { poll = (after: number) => invoke<Poll>('annotation_overlay_poll', { after }),
    send = (strokeId: string, points: number[], end: boolean, clear = false) => invoke<number>('annotation_overlay_stroke', { strokeId, points, end, clear }),
    penOff = () => invoke<void>('annotation_overlay_pen_off') }:
    { poll?: (after: number) => Promise<Poll>; send?: (strokeId: string, points: number[], end: boolean, clear?: boolean) => Promise<number>; penOff?: () => Promise<void> } = $props();

  let canvas = $state<HTMLCanvasElement | null>(null);
  let pen = $state(false);
  let color = $state(0);
  let cursor = 0;
  let video = { width: 0, height: 0 };
  const strokes = new Map<string, Stroke>();
  let drawing: { buffer: StrokeBuffer; local: Stroke } | null = null;
  let sending = false;

  function fit(): Fit {
    const width = canvas?.clientWidth ?? 0, height = canvas?.clientHeight ?? 0;
    return videoFit(width, height, video.width, video.height);
  }
  function absorb(events: CollabEvent[], at: number): void {
    for (const event of events) {
      if (event.kind === 'clear') { strokes.clear(); continue; }
      if (event.kind !== 'stroke' || !event.points || !event.strokeId || event.own) continue;
      const key = `${event.from}:${event.strokeId}`;
      const current = strokes.get(key);
      if (current) { current.points.push(...event.points); current.updated = at; }
      else strokes.set(key, { key, color: event.color ?? 0, points: [...event.points], updated: at });
    }
  }
  function render(at: number): void {
    const element = canvas; if (!element) return;
    const scale = window.devicePixelRatio || 1;
    const width = Math.round(element.clientWidth * scale), height = Math.round(element.clientHeight * scale);
    if (element.width !== width || element.height !== height) { element.width = width; element.height = height; }
    const context = element.getContext('2d'); if (!context) return;
    context.setTransform(1, 0, 0, 1, 0, 0); context.clearRect(0, 0, width, height); context.setTransform(scale, 0, 0, scale, 0, 0);
    const area = fit();
    context.lineCap = 'round'; context.lineJoin = 'round'; context.lineWidth = 4;
    for (const stroke of strokes.values()) {
      const alpha = stroke === drawing?.local ? 1 : strokeAlpha(at - stroke.updated);
      if (alpha <= 0) { strokes.delete(stroke.key); continue; }
      context.globalAlpha = alpha;
      context.strokeStyle = PEN_COLORS[stroke.color]?.value ?? PEN_COLORS[0].value;
      context.shadowColor = 'rgba(0,0,0,0.45)'; context.shadowBlur = 3;
      context.beginPath();
      for (let index = 0; index + 1 < stroke.points.length; index += 2) {
        const [x, y] = denormalize(stroke.points[index] ?? 0, stroke.points[index + 1] ?? 0, area);
        if (index === 0) context.moveTo(x, y); else context.lineTo(x, y);
      }
      if (stroke.points.length === 2) { const [x, y] = denormalize(stroke.points[0] ?? 0, stroke.points[1] ?? 0, area); context.lineTo(x + 0.1, y); }
      context.stroke();
    }
    context.globalAlpha = 1;
  }
  // Finished strokes drain in order at the allowed point rate.
  const queue: { buffer: StrokeBuffer; ended: boolean }[] = [];
  async function flush(): Promise<void> {
    const head = queue[0]; if (!head || sending) return;
    const segment = head.buffer.take(performance.now(), head.ended);
    if (segment) {
      sending = true;
      try { await send(head.buffer.strokeId, segment.points, segment.end); }
      catch { /* Strokes are ephemeral; a rate-limited or dropped segment is skipped. */ }
      finally { sending = false; }
    }
    if (head.ended && !head.buffer.waiting) queue.shift();
  }
  function point(event: PointerEvent): [number, number] | null {
    const box = canvas?.getBoundingClientRect(); if (!box) return null;
    return normalize(event.clientX - box.left, event.clientY - box.top, fit());
  }
  function down(event: PointerEvent): void {
    if (!pen || event.button !== 0) return;
    const at = point(event); if (!at) return;
    canvas?.setPointerCapture?.(event.pointerId);
    if (drawing) up();
    const id = strokeId();
    const local: Stroke = { key: `self:${id}`, color, points: [...at], updated: Date.now() };
    drawing = { buffer: new StrokeBuffer(id, performance.now()), local };
    drawing.buffer.add(at);
    queue.push({ buffer: drawing.buffer, ended: false });
    strokes.set(local.key, local);
  }
  function move(event: PointerEvent): void {
    if (!drawing) return;
    const at = point(event); if (!at) return;
    drawing.buffer.add(at); drawing.local.points.push(...at); drawing.local.updated = Date.now();
  }
  function up(): void {
    if (!drawing) return;
    drawing.local.updated = Date.now();
    const finished = queue.find((item) => item.buffer === drawing?.buffer);
    if (finished) finished.ended = true;
    drawing = null;
  }
  function done(): void { up(); pen = false; void penOff().catch(() => undefined); }

  onMount(() => {
    let alive = true, busy = false;
    const tick = async () => {
      if (busy) return; busy = true;
      try {
        const next = await poll(cursor);
        if (!alive) return;
        if (next.cursor < cursor) strokes.clear();
        cursor = next.cursor; pen = next.pen; color = next.color;
        video = { width: next.videoWidth, height: next.videoHeight };
        if (!next.annotations) strokes.clear();
        absorb(next.events, Date.now());
      } catch { /* The shell closes this window when the session ends. */ }
      finally { busy = false; }
    };
    const timer = window.setInterval(() => { void tick(); void flush(); }, 80);
    let frame = 0;
    const loop = () => { render(Date.now()); frame = requestAnimationFrame(loop); };
    frame = requestAnimationFrame(loop);
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape' && pen) done(); };
    window.addEventListener('keydown', key);
    void tick();
    return () => { alive = false; window.clearInterval(timer); cancelAnimationFrame(frame); window.removeEventListener('keydown', key); };
  });
  const hold = STROKE_HOLD_MS + STROKE_FADE_MS;
</script>

<div class="overlay" class:pen data-fade-ms={hold}>
  <canvas bind:this={canvas} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} aria-label={$t('Annotations')}></canvas>
  {#if pen}
    <div class="toolbar" role="toolbar" aria-label={$t('Pen')}>
      <span class="dot" style={`--swatch:${PEN_COLORS[color]?.value ?? PEN_COLORS[0].value}`}></span>
      <span>{$t('Drawing · strokes fade after a few seconds')}</span>
      <button type="button" onclick={() => { void send(strokeId(), [], true, true).catch(() => undefined); strokes.clear(); }}>{$t('Clear')}</button>
      <button type="button" class="primary" onclick={done}>{$t('Done (Esc)')}</button>
    </div>
  {/if}
</div>

<style>
  :global(html), :global(body) { margin: 0; height: 100%; background: transparent !important; overflow: hidden; }
  .overlay { position: fixed; inset: 0; }
  .overlay.pen { cursor: crosshair; background: rgba(0, 0, 0, 0.01); box-shadow: inset 0 0 0 3px rgba(10, 132, 255, 0.85); }
  canvas { width: 100%; height: 100%; display: block; touch-action: none; }
  .toolbar { position: absolute; top: 12px; left: 50%; transform: translateX(-50%); display: flex; align-items: center; gap: 10px; padding: 8px 12px; border-radius: 999px; background: rgba(20, 20, 24, 0.88); color: #fff; font: 13px system-ui, sans-serif; box-shadow: 0 4px 18px rgba(0, 0, 0, 0.35); }
  .toolbar button { border: 1px solid rgba(255, 255, 255, 0.35); border-radius: 999px; background: transparent; color: #fff; padding: 4px 12px; font: inherit; cursor: pointer; }
  .toolbar button.primary { background: #0a84ff; border-color: #0a84ff; }
  .dot { width: 12px; height: 12px; border-radius: 50%; background: var(--swatch); }
</style>
