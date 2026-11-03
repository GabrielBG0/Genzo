export type Rgba8 = [number, number, number, number];

/**
 * A color token's value as RGBA8, for the colors the engine draws with
 * (the J overlays, DEV-03), so none is hard-coded outside the token set.
 * Reads hex colors and the rgb and rgba functions.
 */
export function tokenRgba(name: string, root: Element = document.documentElement): Rgba8 {
  const value = getComputedStyle(root).getPropertyValue(name).trim();
  const rgba = parseColor(value);
  if (!rgba) throw new Error(`the token ${name} is not a color: "${value}"`);
  return rgba;
}

export function parseColor(value: string): Rgba8 | null {
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(value)?.[1];
  if (hex) {
    const full = hex.length === 3 ? [...hex].map((c) => c + c).join('') : hex;
    const byte = (i: number) => parseInt(full.slice(2 * i, 2 * i + 2), 16);
    return [byte(0), byte(1), byte(2), full.length === 8 ? byte(3) : 255];
  }
  const fn = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/i.exec(value);
  if (!fn) return null;
  const channel = (s: string | undefined) => Math.min(255, Math.max(0, Math.round(Number(s))));
  const alpha = fn[4] === undefined ? 1 : fn[4].endsWith('%') ? Number(fn[4].slice(0, -1)) / 100 : Number(fn[4]);
  return [channel(fn[1]), channel(fn[2]), channel(fn[3]), Math.min(255, Math.max(0, Math.round(alpha * 255)))];
}
