import { describe, expect, it } from 'vitest';
import { CONTRACT_VERSION, type Event } from '$contract/genzo';
import { FakeEngine, isCompatible } from './fake';
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
});
