import { SvelteMap, SvelteSet } from 'svelte/reactivity';
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

const browserJpeg: JpegDecoder = (jpeg) => createImageBitmap(new Blob([jpeg], { type: 'image/jpeg' }));

/**
 * The strip's thumbnails (LIB-09) on the app side.
 *
 * - **Visible first.** `show` sends `requestThumbnails` with the visible
 *   cells first, then the cells after them, then those before, leaving out
 *   the ones known already. The engine replaces its order with each
 *   request, so a scroll moves the newly visible cells to the front.
 * - Each `thumbnailReady` is fetched by photo id and decoded once: a JPEG
 *   tile (the engine's) to an `ImageBitmap`, an RGBA one (the fake's) as
 *   is. A photo with none shows its file name.
 * - Kept by photo id for the session, as the engine keeps ids, so a folder
 *   opened again shows them at once.
 */
export class Thumbnails {
  private readonly states = new SvelteMap<number, ThumbState>();
  /**
   * The last order sent. One that is the same but for the thumbnails
   * that came since is not sent again: the engine holds it already.
   */
  private lastOrder: number[] = [];
  /** Thumbnails being fetched: an answer to a later request is not fetched twice. */
  private readonly fetching = new SvelteSet<number>();
  private readonly unsubscribe: () => void;

  constructor(
    private readonly engine: Engine,
    private readonly decodeJpeg: JpegDecoder = browserJpeg,
  ) {
    this.unsubscribe = engine.onEvent((e) => this.onEvent(e));
  }

  dispose(): void {
    this.unsubscribe();
  }

  /** What photo `id`'s cell shows. */
  get(id: number): ThumbState {
    return this.states.get(id) ?? 'pending';
  }

  /**
   * Asks for the thumbnails of `ids` (the strip, in order) not known yet,
   * those from `first` to `last` (the visible cells) first.
   */
  show(ids: number[], first: number, last: number): void {
    const from = Math.max(0, first);
    const to = Math.min(ids.length - 1, last);
    const order = [...ids.slice(from, to + 1), ...ids.slice(to + 1), ...ids.slice(0, from)].filter(
      (id) => !this.states.has(id),
    );
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
    } catch {
      // Drawn as a photo without one: its name, not an endless gray cell.
      this.states.set(photo, 'none');
    } finally {
      this.fetching.delete(photo);
    }
  }
}
