import { describe, expect, it, vi } from 'vitest';
import type { CommandResult, Event } from '$contract/genzo';
import { FakeEngine } from '../engine/fake';
import { encodeTile } from '../engine/tiles';
import type { Engine } from '../engine/types';
import { Thumbnails, type JpegDecoder, type ThumbImage } from './thumbnails.svelte';

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** Waits until `ok` holds, or fails after a second. */
async function until(ok: () => boolean) {
  for (let i = 0; i < 200 && !ok(); i++) await wait(5);
  expect(ok()).toBe(true);
}

async function opened() {
  const engine = new FakeEngine();
  await engine.execute({ type: 'openFolder', path: 'demo' });
  const thumbnails = new Thumbnails(engine);
  return { engine, thumbnails, ids: [1, 2, 3, 4, 5] };
}

describe('Thumbnails', () => {
  it('asks for the visible cells first, then those after, then those before', async () => {
    const { engine, thumbnails, ids } = await opened();
    thumbnails.show(ids, 2, 3);
    expect(engine.thumbnailRequests).toEqual([[3, 4, 5, 1, 2]]);
  });

  it('shows each thumbnail once made, and the name for a file without one', async () => {
    const { thumbnails, ids } = await opened();
    thumbnails.show(ids, 0, 4);
    await until(() => ids.every((id) => thumbnails.get(id) !== 'pending'));
    const first = thumbnails.get(1) as ThumbImage;
    expect(first.p3).toBe(false);
    // 6048 x 4032 fitted in 320 px.
    expect([first.image.width, first.image.height]).toEqual([320, 213]);
    expect((thumbnails.get(3) as ThumbImage).image.height).toBe(320);
    // The unsupported file has none in the fake.
    expect(thumbnails.get(5)).toBe('none');
  });

  it('asks only for those not known, and not again for the same order', async () => {
    const { engine, thumbnails, ids } = await opened();
    const held: (() => void)[] = [];
    engine.thumbnailLatency = (photo) =>
      photo === 1 ? Promise.resolve() : new Promise<void>((resolve) => held.push(resolve));
    thumbnails.show(ids, 0, 0);
    await until(() => thumbnails.get(1) !== 'pending');
    thumbnails.show(ids, 0, 0);
    expect(engine.thumbnailRequests).toEqual([[1, 2, 3, 4, 5]]);
    // A scroll to the end puts those cells first, without photo 1.
    thumbnails.show(ids, 3, 4);
    expect(engine.thumbnailRequests[1]).toEqual([4, 5, 2, 3]);
    held.forEach((open) => open());
    thumbnails.dispose();
  });

  it('a scroll moves the newly visible cells to the front of the engine queue', async () => {
    const { engine, thumbnails, ids } = await opened();
    const made: number[] = [];
    let release = () => {};
    engine.thumbnailLatency = (photo) => {
      made.push(photo);
      return photo === 1 ? new Promise<void>((resolve) => (release = resolve)) : Promise.resolve();
    };
    thumbnails.show(ids, 0, 1);
    await until(() => made.length === 1);
    thumbnails.show(ids, 4, 4);
    release();
    await until(() => ids.every((id) => thumbnails.get(id) !== 'pending'));
    expect(made).toEqual([1, 5, 2, 3, 4]);
  });

  it('decodes the engine JPEG tiles, and knows a Display P3 one', async () => {
    const jpeg = new Uint8Array([0xff, 0xd8, 1, 2, 3, 0xff, 0xd9]);
    const message = encodeTile(
      {
        generation: 0,
        rect: { x: 0, y: 0, width: 320, height: 240 },
        mipLevel: 0,
        format: 'jpegDisplayP3',
        imageWidth: 320,
        imageHeight: 240,
      },
      jpeg,
    );
    let listener: (e: Event) => void = () => {};
    const engine: Engine = {
      kind: 'tauri',
      handshake: () => Promise.reject(new Error('unused')),
      query: () => Promise.reject(new Error('unused')),
      execute: () => Promise.resolve<CommandResult>({ type: 'done' }),
      onEvent: (l) => ((listener = l), () => {}),
      fetchFrame: () => Promise.reject(new Error('unused')),
      fetchThumbnail: () => Promise.resolve(message.slice().buffer),
    };
    const bitmap = { width: 320, height: 240 } as ImageBitmap;
    const decode = vi.fn<JpegDecoder>(() => Promise.resolve(bitmap));
    const thumbnails = new Thumbnails(engine, decode);
    listener({ type: 'thumbnailReady', photo: 7, found: true });
    await until(() => thumbnails.get(7) !== 'pending');
    expect(thumbnails.get(7)).toEqual({ image: bitmap, p3: true });
    expect([...(decode.mock.lastCall?.[0] ?? [])]).toEqual([...jpeg]);
  });

  it('a thumbnail that cannot be fetched shows the name', async () => {
    const { engine, thumbnails } = await opened();
    engine.fetchThumbnail = () => Promise.reject(new Error('gone'));
    thumbnails.show([1], 0, 0);
    await until(() => thumbnails.get(1) !== 'pending');
    expect(thumbnails.get(1)).toBe('none');
  });
});

describe('Thumbnails, answered twice', () => {
  it('fetches a thumbnail once when a second answer comes during its fetch', async () => {
    const { engine, thumbnails } = await opened();
    const fetch = engine.fetchThumbnail.bind(engine);
    let calls = 0;
    engine.fetchThumbnail = async (photo) => {
      calls += 1;
      await wait(20);
      return fetch(photo);
    };
    thumbnails.show([1], 0, 0);
    await until(() => calls === 1);
    // Asked again (another order) while it is fetched: answered at once.
    thumbnails.show([2, 1], 0, 0);
    await until(() => thumbnails.get(1) !== 'pending' && thumbnails.get(2) !== 'pending');
    expect(calls).toBe(2);
  });
});

describe('Thumbnails, bounded', () => {
  it('keeps at most the cap decoded, never a visible one, and fetches a dropped one again when shown', async () => {
    const engine = new FakeEngine();
    await engine.execute({ type: 'openFolder', path: 'demo' });
    const fetched: number[] = [];
    const fetch = engine.fetchThumbnail.bind(engine);
    engine.fetchThumbnail = (photo) => (fetched.push(photo), fetch(photo));
    const thumbnails = new Thumbnails(engine, undefined, 2);
    const ids = [1, 2, 3, 4, 5];
    thumbnails.show(ids, 0, 0);
    await until(() => fetched.length === 4 && thumbnails.decodedCount === 2);
    await wait(10);
    // 1 is visible; 4 came last. 2 and 3 were dropped.
    expect(typeof thumbnails.get(1)).toBe('object');
    expect(typeof thumbnails.get(4)).toBe('object');
    expect(thumbnails.get(2)).toBe('pending');
    expect(thumbnails.get(3)).toBe('pending');
    expect(thumbnails.get(5)).toBe('none');
    // Shown again: fetched from the engine, not asked to be made.
    thumbnails.show(ids, 1, 2);
    await until(() => typeof thumbnails.get(2) === 'object' && typeof thumbnails.get(3) === 'object');
    expect(fetched).toEqual([1, 2, 3, 4, 2, 3]);
    expect(engine.thumbnailRequests).toHaveLength(1);
    expect(thumbnails.decodedCount).toBe(2);
    expect(thumbnails.get(1)).toBe('pending');
  });

  it('closes the bitmaps it drops', async () => {
    let listener: (e: Event) => void = () => {};
    const tile = encodeTile(
      {
        generation: 0,
        rect: { x: 0, y: 0, width: 4, height: 3 },
        mipLevel: 0,
        format: 'jpegSrgb',
        imageWidth: 4,
        imageHeight: 3,
      },
      new Uint8Array([0xff, 0xd8, 0xff, 0xd9]),
    );
    const engine: Engine = {
      kind: 'tauri',
      handshake: () => Promise.reject(new Error('unused')),
      query: () => Promise.reject(new Error('unused')),
      execute: () => Promise.resolve<CommandResult>({ type: 'done' }),
      onEvent: (l) => ((listener = l), () => {}),
      fetchFrame: () => Promise.reject(new Error('unused')),
      fetchThumbnail: () => Promise.resolve(tile.slice().buffer),
    };
    const bitmaps: { close: ReturnType<typeof vi.fn> }[] = [];
    const decode: JpegDecoder = () => {
      const b = { width: 4, height: 3, close: vi.fn() };
      bitmaps.push(b);
      return Promise.resolve(b as unknown as ImageBitmap);
    };
    const thumbnails = new Thumbnails(engine, decode, 1);
    listener({ type: 'thumbnailReady', photo: 1, found: true });
    await until(() => thumbnails.decodedCount === 1);
    listener({ type: 'thumbnailReady', photo: 2, found: true });
    await until(() => bitmaps.length === 2 && thumbnails.get(1) === 'pending');
    expect(bitmaps[0]?.close).toHaveBeenCalledOnce();
    expect(bitmaps[1]?.close).not.toHaveBeenCalled();
  });
});
