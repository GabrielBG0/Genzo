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

  it('renders a test pattern as tiles covering the image', async () => {
    const engine = new FakeEngine();
    const finished = new Promise<Event>((resolve) => engine.onEvent(resolve));
    await engine.execute({ type: 'renderTestPattern', width: 600, height: 300, phase: 0.5, format: 'rgba8Srgb' });
    const event = await finished;
    if (event.type !== 'renderFinished') throw new Error(`unexpected ${event.type}`);
    expect(event.tileCount).toBe(6);
    const tiles = decodeFrame(await engine.fetchFrame(event.generation));
    const area = tiles.reduce((n, t) => n + t.header.rect.width * t.header.rect.height, 0);
    expect(area).toBe(600 * 300);
  });

  it('fits like the Rust pipeline', () => {
    expect(fit([8064, 6048], [2560, 2560])).toEqual([2560, 1920]);
    expect(fit([100, 50], [2560, 1440])).toEqual([100, 50]);
  });

  it('renders the spike image fitted to the viewport', async () => {
    const engine = new FakeEngine();
    const events: Event[] = [];
    engine.onEvent((e) => events.push(e));
    await expect(
      engine.execute({ type: 'renderSpike', width: 10, height: 10, exposureEv: 0, wb: [1, 1, 1], format: 'rgba8Srgb', interactive: true, proxy: false }),
    ).rejects.toThrow('no image loaded');
    await engine.execute({ type: 'loadSpikeImage', source: { type: 'synthetic', width: 1201, height: 800 } });
    await engine.execute({ type: 'renderSpike', width: 600, height: 600, exposureEv: 1, wb: [1, 1, 1], format: 'rgba8Srgb', interactive: true, proxy: false });
    await new Promise((r) => setTimeout(r));
    expect(events[0]).toEqual({ type: 'spikeImageLoaded', width: 1200, height: 800 });
    const done = events[1];
    if (done?.type !== 'renderFinished') throw new Error('no render');
    expect(done.gpuMs).toBeNull();
    const tiles = decodeFrame(await engine.fetchFrame(done.generation));
    expect(tiles[0]?.header.imageWidth).toBe(600);
    expect(tiles[0]?.header.imageHeight).toBe(400);
  });
});
