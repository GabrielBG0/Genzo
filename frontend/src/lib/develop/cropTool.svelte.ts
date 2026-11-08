import type { Change, Crop, Phase } from '$contract/genzo';
import {
  ASPECTS,
  clampAngle,
  cropOf,
  frameOf,
  oriented,
  OVERLAYS,
  ratioOf,
  withRatio,
  type Frame,
  type Overlay,
  type Size,
} from './crop';
import type { DevelopSession } from './session.svelte';

/** How close a frame's ratio must be to a preset's to be named after it. */
const RATIO_MATCH = 0.005;

/**
 * The crop tool (DEV-12): whether it is open, its aspect lock, overlay and
 * straighten tool, kept for the session, and the commands its panel,
 * keys and frame send. The crop itself is the edit's.
 */
export class CropTool {
  open = $state(false);
  /** Handles keep the frame's ratio. */
  locked = $state(true);
  overlay: Overlay = $state('thirds');
  /** The straighten tool is on: a drag on the photo draws its line. */
  straightening = $state(false);

  constructor(private readonly session: DevelopSession) {}

  /** The oriented photo's size in pixels; zero before an edit reply. */
  get size(): Size {
    const o = this.session.edit?.oriented;
    return { width: o?.width ?? 0, height: o?.height ?? 0 };
  }

  /** The crop shown: the optimistic one during a drag. */
  get crop(): Crop | null {
    return this.session.shown?.crop ?? null;
  }

  get frame(): Frame | null {
    const c = this.crop;
    const s = this.size;
    return c && s.width > 0 ? frameOf(c, s) : null;
  }

  /** The preset the frame's ratio matches, "Free" when unlocked, or "Custom". */
  get aspect(): string {
    if (!this.locked) return 'Free';
    const f = this.frame;
    if (!f) return 'Original';
    const r = ratioOf(f);
    const s = this.size;
    for (const a of ASPECTS) {
      if (a.ratio === 'free') continue;
      const want = a.ratio === 'original' ? oriented(s.width / s.height, f) : oriented(a.ratio, f);
      if (Math.abs(r / want - 1) < RATIO_MATCH) return a.label;
    }
    return 'Custom';
  }

  toggle(): void {
    if (this.open) this.close();
    else if (this.session.editable) this.open = true;
  }

  close(): void {
    this.open = false;
    this.straightening = false;
  }

  /** A crop from the frame, as a gesture's step. */
  sendFrame(f: Frame, phase: Phase): void {
    this.session.change([{ Crop: cropOf(f, this.size) }], phase);
  }

  /** The aspect select: a preset fits about the center and locks; Free unlocks. */
  choose(label: string): void {
    const a = ASPECTS.find((x) => x.label === label);
    const f = this.frame;
    if (!a || !f) return;
    if (a.ratio === 'free') {
      this.locked = false;
      return;
    }
    const s = this.size;
    const ratio = a.ratio === 'original' ? oriented(s.width / s.height, f) : oriented(a.ratio, f);
    this.locked = true;
    this.sendFrame(withRatio(f, ratio, s), 'End');
  }

  /** X: the frame turns portrait or landscape about its center, as large as fits. */
  swap(): void {
    const f = this.frame;
    if (f) this.sendFrame(withRatio(f, 1 / ratioOf(f), this.size), 'End');
  }

  /** The angle slider, in degrees. */
  setAngle(deg: number, phase: Phase): void {
    const c = this.crop;
    if (c) this.session.change([{ Crop: { ...c, angle: clampAngle(deg) } }], phase);
  }

  /** Ctrl [ and Ctrl ]: a quarter turn; the crop follows the image (DEV-01). */
  turn(clockwise: boolean): void {
    this.session.change([{ Turn: { clockwise } }]);
  }

  flipHorizontal(): void {
    this.session.change(['Flip']);
  }

  /** A vertical flip is a horizontal one and a half turn, one step. */
  flipVertical(): void {
    const half: Change[] = [{ Turn: { clockwise: true } }, { Turn: { clockwise: true } }];
    this.session.change(['Flip', ...half]);
  }

  /** Reset: the crop and the orientation, as `Reset(Orientation)` does both (DEV-01). */
  reset(): void {
    this.session.reset('Orientation');
  }

  /** O: thirds, golden, grid, none. */
  cycleOverlay(): void {
    this.overlay = OVERLAYS[(OVERLAYS.indexOf(this.overlay) + 1) % OVERLAYS.length] ?? 'thirds';
  }
}
