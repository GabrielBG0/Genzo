import type { Edit, Size } from '$contract/genzo';

/**
 * The sizes an edit reply names (`PhotoEdit.oriented`, `PhotoEdit.shown`),
 * as `PhotoEdits::sizes` computes them: the stored size in the edit's
 * orientation, and the crop's size in whole pixels, at least 1. The fake's
 * files are all stored upright.
 */
export function sizes(edit: Edit, stored: [number, number]): { oriented: Size; shown: Size } {
  const [w, h] = edit.quarter_turns % 2 === 1 ? [stored[1], stored[0]] : stored;
  const c = edit.crop;
  const full = c.cx === 0.5 && c.cy === 0.5 && c.w === 1 && c.h === 1 && c.angle === 0;
  return {
    oriented: { width: w, height: h },
    shown: full
      ? { width: w, height: h }
      : { width: Math.max(1, Math.round(c.w * w)), height: Math.max(1, Math.round(c.h * h)) },
  };
}
