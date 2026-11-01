import type { Edit, WhiteBalancePreset } from '$contract/genzo';
import type { SliderScale } from '../components/SliderRow.svelte';
import { RAW_TEMP } from '../engine/edit';

/**
 * RAW Temp's place on its track, linear in mireds (DEV-02):
 * `p = (1/2000 − 1/K) / (1/2000 − 1/50000)`, so 5000 K sits at 62.5 %.
 */
export const MIRED_SCALE: SliderScale = {
  toT: (k) => (1 / RAW_TEMP[0] - 1 / k) / (1 / RAW_TEMP[0] - 1 / RAW_TEMP[1]),
  fromT: (t) => 1 / (1 / RAW_TEMP[0] - t * (1 / RAW_TEMP[0] - 1 / RAW_TEMP[1])),
};

/**
 * What the white balance dropdown shows (DEV-02): "As Shot" in as-shot
 * mode; in custom mode, the preset whose Temp and Tint equal the edit's as
 * the sliders show them (1 K, 1 tint), else "Custom".
 */
export function presetName(edit: Edit, presets: WhiteBalancePreset[]): string {
  const wb = edit.white_balance;
  if (wb.mode === 'AsShot') return 'As Shot';
  const match = presets.find((p) => round(p.temp) === round(wb.temp) && round(p.tint) === round(wb.tint));
  return match?.name ?? 'Custom';
}

/** Rounds half away from zero, as Rust's `f64::round` (`Preset::matching`). */
export function round(v: number): number {
  return Math.sign(v) * Math.round(Math.abs(v));
}
