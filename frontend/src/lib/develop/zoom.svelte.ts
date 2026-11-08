import type { View } from '$contract/genzo';

export type ZoomLevel = 'fit' | '1:1';

interface Size {
  width: number;
  height: number;
}

interface Point {
  x: number;
  y: number;
}

/**
 * The Develop viewport's zoom and pan (DEV-01 viewport spec): Fit or 1:1,
 * one photo pixel per device pixel. Positions are in device pixels from
 * the view's top left, or in shown-image pixels (`PhotoEdit.shown`).
 */
export class ZoomState {
  level: ZoomLevel = $state('fit');
  /** Space is held: release returns to Fit. */
  held = $state(false);
  /** The view's size in device pixels. */
  view: Size = $state({ width: 0, height: 0 });
  /** The shown image's size in pixels; zero before a photo's edit. */
  shown: Size = $state({ width: 0, height: 0 });
  /** Where the pointer is over the view, in device pixels; null off it (Z and Space zoom there). */
  pointer: Point | null = null;
  /** At 1:1, the photo point at the view's center, in shown pixels. */
  private center: Point = $state({ x: 0, y: 0 });

  /** Device pixels per shown pixel at Fit; 1 before a size is known. */
  get fitZoom(): number {
    const { view, shown } = this;
    if (shown.width <= 0 || shown.height <= 0 || view.width <= 0 || view.height <= 0) return 1;
    return Math.min(view.width / shown.width, view.height / shown.height);
  }

  get zoom(): number {
    return this.level === 'fit' ? this.fitZoom : 1;
  }

  /** The view's top left in shown pixels: whole pixels at 1:1, as the engine snaps it. */
  get origin(): Point {
    const z = this.zoom;
    if (this.level === 'fit') {
      return {
        x: (this.shown.width - this.view.width / z) / 2,
        y: (this.shown.height - this.view.height / z) / 2,
      };
    }
    return { x: axis(this.center.x, this.view.width, this.shown.width), y: axis(this.center.y, this.view.height, this.shown.height) };
  }

  /** What `SetView` says. */
  get request(): View {
    if (this.level === 'fit') return { type: 'fit' };
    const o = this.origin;
    return { type: 'region', zoom: 1, x: o.x, y: o.y };
  }

  /** "Fit 48%" or "1:1". */
  get readout(): string {
    return this.level === 'fit' ? `Fit ${Math.round(this.fitZoom * 100)}%` : '1:1';
  }

  /** The shown-image point under `p`, in shown pixels. */
  imageAt(p: Point): Point {
    const o = this.origin;
    const z = this.zoom;
    return { x: o.x + p.x / z, y: o.y + p.y / z };
  }

  /** The point under `p` normalized to the shown image, or null off it (the picker). */
  normalizedAt(p: Point): [number, number] | null {
    if (this.shown.width <= 0 || this.shown.height <= 0) return null;
    const i = this.imageAt(p);
    const u = i.x / this.shown.width;
    const v = i.y / this.shown.height;
    return u >= 0 && u <= 1 && v >= 0 && v <= 1 ? [u, v] : null;
  }

  /** 1:1 with the photo point under `p` staying there; the view's center when null. */
  zoomIn(p: Point | null): void {
    if (this.level === '1:1') return;
    const at = p ?? { x: this.view.width / 2, y: this.view.height / 2 };
    const i = this.imageAt(at);
    this.level = '1:1';
    // At 1:1 the point under `at` is origin + at, so the center is i - at + view / 2.
    this.setCenter({ x: i.x - at.x + this.view.width / 2, y: i.y - at.y + this.view.height / 2 });
  }

  fit(): void {
    this.level = 'fit';
    this.held = false;
  }

  /** Z, and a click: Fit ↔ 1:1. */
  toggle(p: Point | null): void {
    if (this.level === 'fit') this.zoomIn(p);
    else this.fit();
  }

  /** Space down: 1:1 until `release`. Nothing at 1:1 already. */
  hold(p: Point | null): void {
    if (this.level === '1:1') return;
    this.zoomIn(p);
    this.held = true;
  }

  /** Space up. */
  release(): void {
    if (this.held) this.fit();
  }

  /** A drag or a scroll of `dx`, `dy` device pixels: the photo moves with it. */
  pan(dx: number, dy: number): void {
    if (this.level !== '1:1') return;
    this.setCenter({ x: this.center.x - dx, y: this.center.y - dy });
  }

  /** The view's size changes; at 1:1 its center stays on the same photo point. */
  resize(width: number, height: number): void {
    this.view = { width, height };
    this.setCenter(this.center);
  }

  /** The shown image's size, from an edit reply; a new photo opens on its center. */
  setShown(width: number, height: number, newPhoto: boolean): void {
    const was = this.shown;
    this.shown = { width, height };
    if (newPhoto || was.width <= 0 || was.height <= 0) {
      this.center = { x: width / 2, y: height / 2 };
    } else if (was.width !== width || was.height !== height) {
      // A crop or turn: the same fraction of the photo stays centered.
      this.center = { x: (this.center.x / was.width) * width, y: (this.center.y / was.height) * height };
    }
    this.setCenter(this.center);
  }

  /** Clamps the center so the photo leaves no gap it could fill. */
  private setCenter(c: Point): void {
    const o = { x: axis(c.x, this.view.width, this.shown.width), y: axis(c.y, this.view.height, this.shown.height) };
    this.center = { x: o.x + this.view.width / 2, y: o.y + this.view.height / 2 };
  }
}

/**
 * One axis of the 1:1 origin for a center `c`: clamped to the photo's
 * edges when the photo is larger than the view, centered otherwise, and on
 * a whole pixel.
 */
function axis(c: number, view: number, shown: number): number {
  if (shown <= view) return Math.round((shown - view) / 2);
  return Math.round(Math.min(Math.max(c - view / 2, 0), shown - view));
}
