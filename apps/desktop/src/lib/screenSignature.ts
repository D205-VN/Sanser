/**
 * A tiny grayscale fingerprint of a JPEG screen, so the exam seat sends a
 * picture only when something on the screen moved. 64×36 cells: a caret
 * blinking or a word typed changes at least one cell.
 */
export const SIGNATURE_WIDTH = 64;
export const SIGNATURE_HEIGHT = 36;
/** A cell must move this much (0–255) to count as a change, so JPEG noise does not. */
export const SIGNATURE_THRESHOLD = 10;

/** `null` when this webview cannot decode the picture; treat that as changed. */
export async function screenSignature(jpegBase64: string): Promise<Uint8Array | null> {
  try {
    if (typeof createImageBitmap !== 'function' || typeof document === 'undefined') return null;
    const bytes = Uint8Array.from(atob(jpegBase64), (char) => char.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/jpeg' }));
    const canvas = document.createElement('canvas');
    canvas.width = SIGNATURE_WIDTH;
    canvas.height = SIGNATURE_HEIGHT;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) { bitmap.close(); return null; }
    context.imageSmoothingEnabled = true;
    context.imageSmoothingQuality = 'high';
    context.drawImage(bitmap, 0, 0, SIGNATURE_WIDTH, SIGNATURE_HEIGHT);
    bitmap.close();
    const pixels = context.getImageData(0, 0, SIGNATURE_WIDTH, SIGNATURE_HEIGHT).data;
    const signature = new Uint8Array(SIGNATURE_WIDTH * SIGNATURE_HEIGHT);
    for (let cell = 0; cell < signature.length; cell += 1) {
      const at = cell * 4;
      signature[cell] = Math.round(0.299 * (pixels[at] ?? 0) + 0.587 * (pixels[at + 1] ?? 0) + 0.114 * (pixels[at + 2] ?? 0));
    }
    return signature;
  } catch {
    return null;
  }
}

/** Unknown on either side counts as changed. */
export function screenChanged(previous: Uint8Array | null, next: Uint8Array | null, threshold = SIGNATURE_THRESHOLD): boolean {
  if (!previous || !next || previous.length !== next.length) return true;
  for (let cell = 0; cell < next.length; cell += 1) {
    if (Math.abs((next[cell] ?? 0) - (previous[cell] ?? 0)) > threshold) return true;
  }
  return false;
}
