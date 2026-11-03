import type { Rational, Shooting } from '$contract/genzo';

/**
 * The histogram's EXIF row (DEV-03): ISO, focal length, aperture and
 * exposure time, each left out when the file does not carry it.
 */
export function exifRow(s: Shooting): string[] {
  const row: string[] = [];
  if (s.iso !== null) row.push(`ISO ${s.iso}`);
  const mm = value(s.focalLength);
  if (mm !== null && mm > 0) row.push(`${mm < 10 ? mm.toFixed(1) : Math.round(mm)} mm`);
  const f = value(s.fNumber);
  if (f !== null && f > 0) row.push(`f/${trimmed(f.toFixed(1))}`);
  const t = value(s.exposureTime);
  if (t !== null && t > 0) row.push(`${seconds(t)} s`);
  return row;
}

/** `1/250` under a second, the denominator rounded; `1.3` and `30` otherwise. */
function seconds(t: number): string {
  if (t < 1) return `1/${Math.round(1 / t)}`;
  return t < 10 ? t.toFixed(1) : String(Math.round(t));
}

function value(r: Rational | null): number | null {
  return r && r.den !== 0 ? r.num / r.den : null;
}

/** `8.0` reads `8`. */
function trimmed(s: string): string {
  return s.endsWith('.0') ? s.slice(0, -2) : s;
}
