import { SvelteMap } from 'svelte/reactivity';
import type { Event } from '$contract/genzo';
import type { RgbaImage } from '../components/Thumb.svelte';
import { decodeFrame, isJpeg } from '../engine/tiles';
import type { Engine } from '../engine/types';

/** A thumbnail ready to draw, and the color space of its codes. */
export interface ThumbImage {
  image: RgbaImage | ImageBitmap;
  p3: boolean;
}

/** What a strip cell shows: not made yet, none (the file name), or the image. */
export type ThumbState = 'pending' | 'none' | ThumbImage;

/** Decodes a JPEG payload; `createImageBitmap` in the app. */
export type JpegDecoder = (jpeg: Uint8Array<ArrayBuffer>) => Promise<ImageBitmap>;

/**
 * The codes as they are, as the viewport decodes its JPEG tiles (M0 hand-off,
 * item 6): the engine encodes them for the frames' target, without a profile.
 */
const browserJpeg: JpegDecoder = (jpeg) =>
  createImageBitmap(new Blob([jpeg], { type: 'image/jpeg' }), { colorSpaceConversion: 'none' });

/**
 * Decoded thumbnails kept at most: about 54 MB at 320 × 213, well beyond
 * the cells a strip draws (about 30 at 2560 px). The engine keeps every
 * thumbnail in its own tiers, so one dropped here is fetched again cheaply.
 */
export const DECODED_CAP = 200;

/** The engine has the photo's thumbnail; it is not decoded here now. */
const FOUND = 'found';

/**
 * The strip's thumbnails (LIB-09) on the app side.
 *
 * - **Visible first.** `show` sends `requestThumbnails` with the visible
 *   cells first, then the cells after them, then those before, leaving out
 *   the ones known already. The engine replaces its order with each
 *   request, so a scroll moves the newly visible cells to the front.
 * - Each `thumbnailReady` is fetched by photo id and decoded: a JPEG tile
 *   (the engine's) to an `ImageBitmap`, an RGBA one (the fake's) as is. A
 *   photo with none shows its file name.
 * - **Bounded.** At most `cap` thumbnails stay decoded, the least recently
 *   shown dropped first, never a visible one; a dropped bitmap is closed.
 *   A dropped thumbnail is fetched again when its cell shows. A folder of
 *   thousands would otherwise hold over a gigabyte of pixels.
 * - Known by photo id for the session, as the engine keeps ids, so a folder
 *   opened again asks for nothing new.
 */
export class Thumbnails {
  private readonly states = new SvelteMap<number, typeof FOUND | 'none' | ThumbImage>();
  /** Decoded thumbnails, least recently shown first. */
  private decoded: number[] = [];
  /** The cells visible at the last `show`. Not state, as `fetching`. */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  private visible = new Set<number>();
  /**
   * The last order sent. One that is the same but for the thumbnails
   * that came since is not sent again: the engine holds it already.
   */
  private lastOrder: number[] = [];
  /**
   * Thumbnails being fetched: an answer to a later request is not fetched
   * twice. Not state: `show` runs in an effect and both reads and adds to it.
   */
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  private readonly fetching = new Set<number>();
  private readonly unsubscribe: () => void;

  constructor(
    private readonly engine: Engine,
    private readonly decodeJpeg: JpegDecoder = browserJpeg,
    private readonly cap = DECODED_CAP,
  ) {
    this.unsubscribe = engine.onEvent((e) => this.onEvent(e));
  }

  dispose(): void {
    this.unsubscribe();
  }

  /** What photo `id`'s cell shows. */
  get(id: number): ThumbState {
    const state = this.states.get(id);
    return state === undefined || state === FOUND ? 'pending' : state;
  }

  /** How many thumbnails are decoded now. */
  get decodedCount(): number {
    return this.decoded.length;
  }

  /**
   * Asks for the thumbnails of `ids` (the strip, in order) not known yet,
   * those from `first` to `last` (the visible cells) first, and fetches
   * again the visible ones dropped from memory.
   */
  show(ids: number[], first: number, last: number): void {
    const from = Math.max(0, first);
    const to = Math.min(ids.length - 1, last);
    const shown = ids.slice(from, to + 1);
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    this.visible = new Set(shown);
    for (const id of shown) {
      if (this.states.get(id) === FOUND) void this.fetch(id);
      else if (this.decoded.includes(id)) this.touch(id);
    }
    const order = [...shown, ...ids.slice(to + 1), ...ids.slice(0, from)].filter((id) => !this.states.has(id));
    const pending = this.lastOrder.filter((id) => !this.states.has(id));
    if (order.length === pending.length && order.every((id, i) => id === pending[i])) return;
    this.lastOrder = order;
    if (order.length === 0) return;
    void this.engine.execute({ type: 'requestThumbnails', photos: order }).catch(() => {
      // Asked again on the next scroll or folder.
      this.lastOrder = [];
    });
  }

  private onEvent(e: Event) {
    if (e.type !== 'thumbnailReady' || this.states.has(e.photo) || this.fetching.has(e.photo)) return;
    if (!e.found) {
      this.states.set(e.photo, 'none');
      return;
    }
    void this.fetch(e.photo);
  }

  private async fetch(photo: number) {
    if (this.fetching.has(photo)) return;
    this.fetching.add(photo);
    try {
      const [tile] = decodeFrame(await this.engine.fetchThumbnail(photo));
      if (!tile) throw new Error('an empty thumbnail');
      const { format, rect } = tile.header;
      const p3 = format === 'rgba8DisplayP3' || format === 'jpegDisplayP3';
      const image = isJpeg(format)
        ? await this.decodeJpeg(new Uint8Array(tile.pixels))
        : { width: rect.width, height: rect.height, data: new Uint8ClampedArray(tile.pixels) };
      this.states.set(photo, { image, p3 });
      this.touch(photo);
      this.trim();
    } catch {
      // Drawn as a photo without one: its name, not an endless gray cell.
      this.states.set(photo, 'none');
    } finally {
      this.fetching.delete(photo);
    }
  }

  /** Marks `photo`'s decoded thumbnail as the most recently shown. */
  private touch(photo: number) {
    this.decoded = [...this.decoded.filter((id) => id !== photo), photo];
  }

  /** Drops the least recently shown thumbnails past the cap, never a visible one. */
  private trim() {
    let excess = this.decoded.length - this.cap;
    if (excess <= 0) return;
    const kept: number[] = [];
    for (const id of this.decoded) {
      const state = this.states.get(id);
      if (excess > 0 && !this.visible.has(id) && state && typeof state === 'object') {
        if ('close' in state.image) state.image.close();
        this.states.set(id, FOUND);
        excess -= 1;
      } else {
        kept.push(id);
      }
    }
    this.decoded = kept;
  }
}
