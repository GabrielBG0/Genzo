import type { Change, Edit, Param, WhiteBalanceMode } from '$contract/genzo';

/**
 * The edit's rules as the engine applies them, for the fake engine and the
 * panel's optimistic values. Each mirrors its Rust original in
 * `genzo_model::edit` or `genzo_engine::history`; keep them in step.
 */

/** What a photo is, for its defaults: mirrors `genzo_model::edit::PhotoKind`. */
export type EditKind = { raw: true; asShotTemp: number; asShotTint: number } | { raw: false };

/** Mirrors `genzo_model::edit::RAW_TEMP`. */
export const RAW_TEMP: [number, number] = [2000, 50000];
/** Mirrors `genzo_model::edit::RAW_TINT`. */
export const RAW_TINT: [number, number] = [-150, 150];
/** Mirrors `genzo_model::edit::SLIDER`: the sliders, and a non-RAW Temp and Tint. */
export const SLIDER: [number, number] = [-100, 100];
/** Mirrors `genzo_model::edit::EXPOSURE`. */
export const EXPOSURE: [number, number] = [-5, 5];
/** Mirrors `genzo_model::edit::ANGLE`. */
export const ANGLE: [number, number] = [-45, 45];

/** The sliders whose values are plain numbers. */
export const VALUES = [
  'exposure',
  'contrast',
  'highlights',
  'shadows',
  'whites',
  'blacks',
  'vibrance',
  'saturation',
] as const;
export type ValueField = (typeof VALUES)[number];

/** Mirrors `Edit::defaults`. */
export function defaults(kind: EditKind): Edit {
  return {
    schema_version: 1,
    profile: kind.raw ? 'GenzoStandard' : 'Embedded',
    treatment: 'Color',
    white_balance: {
      mode: 'AsShot',
      temp: kind.raw ? kind.asShotTemp : 0,
      tint: kind.raw ? kind.asShotTint : 0,
    },
    exposure: 0,
    contrast: 0,
    highlights: 0,
    shadows: 0,
    whites: 0,
    blacks: 0,
    vibrance: 0,
    saturation: 0,
    crop: { cx: 0.5, cy: 0.5, w: 1, h: 1, angle: 0 },
    quarter_turns: 0,
    flip_horizontal: false,
  };
}

/** Mirrors `clamp` in `genzo_model::edit`, in f32 as Rust stores it. */
function clamp(v: number, [lo, hi]: [number, number], fallback: number): number {
  return Number.isNaN(v) ? fallback : Math.fround(Math.min(hi, Math.max(lo, v)));
}

/** Mirrors `Edit::clamped`. */
export function clamped(e: Edit, kind: EditKind): Edit {
  const d = defaults(kind);
  const out = copy(e);
  if (kind.raw === (out.profile === 'Embedded')) out.profile = d.profile;
  const wb = out.white_balance;
  if (wb.mode === 'AsShot') {
    wb.temp = d.white_balance.temp;
    wb.tint = d.white_balance.tint;
  }
  wb.temp = clamp(wb.temp, kind.raw ? RAW_TEMP : SLIDER, d.white_balance.temp);
  wb.tint = clamp(wb.tint, kind.raw ? RAW_TINT : SLIDER, d.white_balance.tint);
  out.exposure = clamp(out.exposure, EXPOSURE, 0);
  for (const f of VALUES) if (f !== 'exposure') out[f] = clamp(out[f], SLIDER, 0);
  const c = out.crop;
  c.cx = clamp(c.cx, [0, 1], 0.5);
  c.cy = clamp(c.cy, [0, 1], 0.5);
  c.w = clamp(c.w, [1.1754943508222875e-38, 1], 1);
  c.h = clamp(c.h, [1.1754943508222875e-38, 1], 1);
  c.angle = clamp(c.angle, ANGLE, 0);
  out.quarter_turns %= 4;
  return out;
}

/** Mirrors `Edit::is_edited`: Temp and Tint count only when custom. */
export function isEdited(e: Edit, d: Edit): boolean {
  return paramsEdited(e, d).length > 0;
}

/** The fields of `e` that differ from `d`, as `Edit::is_edited` compares them. */
export function paramsEdited(e: Edit, d: Edit): Param[] {
  const out: Param[] = [];
  if (e.profile !== d.profile) out.push('Profile');
  if (e.treatment !== d.treatment) out.push('Treatment');
  const wb = (x: Edit) =>
    x.white_balance.mode === 'AsShot' ? 'AsShot' : `Custom ${x.white_balance.temp} ${x.white_balance.tint}`;
  if (wb(e) !== wb(d)) out.push('WhiteBalance');
  for (const f of VALUES) if (e[f] !== d[f]) out.push(PARAM_OF[f]);
  const crop = (x: Edit) => [x.crop.cx, x.crop.cy, x.crop.w, x.crop.h, x.crop.angle].join();
  if (crop(e) !== crop(d)) out.push('Crop');
  if (e.quarter_turns !== d.quarter_turns || e.flip_horizontal !== d.flip_horizontal) out.push('Orientation');
  return out;
}

export const PARAM_OF: Record<ValueField, Param> = {
  exposure: 'Exposure',
  contrast: 'Contrast',
  highlights: 'Highlights',
  shadows: 'Shadows',
  whites: 'Whites',
  blacks: 'Blacks',
  vibrance: 'Vibrance',
  saturation: 'Saturation',
};

/** Mirrors `Param::label`. */
export function paramLabel(p: Param): string {
  switch (p) {
    case 'WhiteBalance':
      return 'White balance';
    case 'Orientation':
      return 'Rotate';
    default:
      return p;
  }
}

/** Mirrors `Change::param`: the field a change sets, null for Reset all. */
export function changeParam(c: Change): Param | null {
  if (c === 'ResetAll') return null;
  if (c === 'Flip') return 'Orientation';
  if ('Turn' in c) return 'Orientation';
  if ('Reset' in c) return c.Reset;
  return Object.keys(c)[0] as Param;
}

/** Mirrors `Change::label`. */
export function changeLabel(c: Change): string {
  if (c === 'ResetAll') return 'Reset all';
  if (typeof c === 'object' && 'Reset' in c) return `Reset ${paramLabel(c.Reset)}`;
  return paramLabel(changeParam(c) as Param);
}

/** The label of a delta's step, as `PhotoEdits::apply_labelled` picks it. */
export function deltaLabel(changes: Change[]): string {
  const [first] = changes;
  if (!first) return 'Edit';
  if (changes.length === 1) return changeLabel(first);
  return changes.every((c) => changeParam(c) === changeParam(first)) ? changeLabel(first) : 'Edit';
}

/**
 * One change applied, then clamped: mirrors `PhotoEdits::change`. The fake
 * engine does not fit the crop to the image (it accepts any clamped crop)
 * and turns and flips without mapping the crop: the Basic panel needs
 * neither, and the crop tool (DEV-12) brings them.
 */
export function applyChange(e: Edit, c: Change, kind: EditKind): Edit {
  const d = defaults(kind);
  const out = copy(e);
  if (c === 'ResetAll') return clamped(d, kind);
  if (c === 'Flip') {
    out.flip_horizontal = !out.flip_horizontal;
  } else if ('Turn' in c) {
    out.quarter_turns = (out.quarter_turns + (c.Turn.clockwise ? 1 : 3)) % 4;
  } else if ('Reset' in c) {
    resetParam(out, d, c.Reset);
  } else if ('Profile' in c) {
    out.profile = c.Profile;
  } else if ('Treatment' in c) {
    out.treatment = c.Treatment;
  } else if ('WhiteBalance' in c) {
    out.white_balance = { ...c.WhiteBalance };
  } else if ('Crop' in c) {
    out.crop = { ...c.Crop };
  } else {
    const [param, value] = Object.entries(c)[0] as [string, number];
    out[param.toLowerCase() as ValueField] = value;
  }
  return clamped(out, kind);
}

function resetParam(e: Edit, d: Edit, p: Param) {
  switch (p) {
    case 'Profile':
      e.profile = d.profile;
      break;
    case 'Treatment':
      e.treatment = d.treatment;
      break;
    case 'WhiteBalance':
      e.white_balance.mode = 'AsShot' satisfies WhiteBalanceMode;
      break;
    case 'Crop':
      e.crop = { ...d.crop };
      break;
    case 'Orientation':
      e.quarter_turns = d.quarter_turns;
      e.flip_horizontal = d.flip_horizontal;
      e.crop = { ...d.crop };
      break;
    default:
      e[p.toLowerCase() as ValueField] = d[p.toLowerCase() as ValueField];
  }
}

/**
 * A deep copy. JSON rather than `structuredClone`, which refuses the
 * app's reactive state proxies.
 */
export function copy<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

/** Whether two edits are the same, as Rust's `==` compares them. */
export function sameEdit(a: Edit, b: Edit): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
