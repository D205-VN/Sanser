export function validUserText(text: string, clipboard = false): boolean {
  if ((!clipboard && (text.length === 0 || text.length > 256)) || new TextEncoder().encode(text).length > (clipboard ? 8192 : 1024)) return false;
  for (const character of text) {
    const code = character.codePointAt(0);
    if (code === undefined) return false;
    if ((code >= 0xd800 && code <= 0xdfff) || (code < 32 && code !== 9 && code !== 10 && !(clipboard && code === 13)) || (code >= 127 && code <= 159)) return false;
  }
  return true;
}
