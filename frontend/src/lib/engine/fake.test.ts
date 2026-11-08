import { describe, expect, it } from 'vitest';
import { CONTRACT_VERSION, type Event } from '$contract/genzo';
import { FakeEngine, fit, isCompatible } from './fake';
import { decodeFrame } from './tiles';

describe('FakeEngine', () => {
  it('follows the Rust compatibility rule', () => {
    expect(isCompatible('0.1.0', '0.1.9')).toBe(true);
    expect(isCompatible('0.1.0', '0.2.0')).toBe(false);
    expect(isCompatible('1.0.0', '1.3.0')).toBe(true);
    expect(isCompatible('1.0.0', '2.0.0')).toBe(false);
    expect(isCompatible('nope', '0.1.0')).toBe(false);
  });

  it('accepts the current contract and refuses another', async () => {
    const engine = new FakeEngine();
    expect((await engine.handshake({ contractVersion: CONTRACT_VERSION })).compatible).toBe(true);
    expect((await new FakeEngine('9.0.0').handshake({ contractVersion: CONTRACT_VERSION })).compatible).toBe(false);
  });

  it('fits like the Rust pipeline', () => {
    expect(fit([8064, 6048], [2560, 2560])).toEqual([2560, 1920]);
    expect(fit([100, 50], [2560, 1440])).toEqual([100, 50]);
  });

  it('opens a synthetic RAW as a folder of one, rendered to the view (the autobench)', async () => {
    const engine = new FakeEngine();
    const events: Event[] = [];
    engine.onEvent((e) => events.push(e));
    await engine.execute({ type: 'setView', width: 600, height: 600, view: { type: 'fit' }, space: 'srgb' });
    const r = await engine.execute({ type: 'openSynthetic', width: 1200, height: 800 });
    if (r.type !== 'folder') throw new Error(`answered ${r.type}`);
    expect(r.strip.photos.map((p) => p.fileName)).toEqual(['Synthetic 1200 × 800']);
    expect(r.edit?.shown).toEqual({ width: 1200, height: 800 });
    const done = events.find((e) => e.type === 'renderFinished');
    if (done?.type !== 'renderFinished') throw new Error('no render');
    expect(done.gpuMs).toBeNull();
    const tiles = decodeFrame(await engine.fetchFrame(done.generation));
    expect(tiles[0]?.header.imageWidth).toBe(600);
    expect(tiles[0]?.header.imageHeight).toBe(400);
  });

  it('answers before a folder is open as the engine does', async () => {
    const engine = new FakeEngine();
    expect(await engine.query({ type: 'listFolderPhotos' })).toEqual({
      type: 'folderPhotos',
      strip: { photos: [], current: null, direction: 'next' },
    });
    expect(await engine.query({ type: 'getEdit', photo: 1 })).toEqual({ type: 'edit', edit: null });
    expect(await engine.query({ type: 'getHistogram', photo: 1 })).toEqual({ type: 'histogram', counted: null });
    await expect(engine.execute({ type: 'undo', photo: 1 })).rejects.toThrow('photo 1 was not opened this session');
  });
});
