import { describe, expect, it } from 'vitest';
import type { Command } from '$contract/genzo';
import { FakeEngine } from '../engine/fake';
import { DevelopSession, type ClipColors } from './session.svelte';

const COLORS: ClipColors = { shadow: [1, 2, 3, 255], highlight: [4, 5, 6, 255] };

/** A gate a test opens: replies held until then. */
function gate() {
  let open = () => {};
  const shut = new Promise<void>((resolve) => (open = resolve));
  return { shut, open };
}

const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function open(latency?: FakeEngine['latency']) {
  const engine = new FakeEngine();
  if (latency) engine.latency = latency;
  const session = new DevelopSession(engine, 'srgb', () => COLORS);
  await session.openFolder('demo');
  await session.setView(640, 480);
  engine.sent.length = 0;
  return { engine, session };
}

/** Waits until every command sent has answered. */
async function settled(session: DevelopSession) {
  for (let i = 0; i < 100 && (session['inFlight'] || session['queue'].length > 0); i++) await wait(5);
}

const deltas = (sent: Command[]) =>
  sent.flatMap((c) => (c.type === 'applyEditDelta' ? [[c.phase, c.changes] as const] : []));

describe('DevelopSession', () => {
  it('sends a drag as one start, its updates once per frame, and one end: one step', async () => {
    const { engine, session } = await open();
    session.change([{ Exposure: 0.1 }], 'Start');
    session.change([{ Exposure: 0.2 }], 'Update');
    session.change([{ Exposure: 0.3 }], 'Update');
    await nextFrame();
    session.change([{ Exposure: 0.4 }], 'Update');
    session.change([{ Exposure: 0.5 }], 'End');
    await settled(session);
    // 0.2 was replaced by 0.3 within a frame; 0.4 by the end.
    expect(deltas(engine.sent)).toEqual([
      ['Start', [{ Exposure: 0.1 }]],
      ['Update', [{ Exposure: 0.3 }]],
      ['End', [{ Exposure: 0.5 }]],
    ]);
    expect(session.edit).toMatchObject({ undo: 'Exposure', edited: true });
    expect(session.shown?.exposure).toBeCloseTo(0.5);
  });

  it('keeps a dragged value when an older reply arrives during the gesture', async () => {
    // The start's reply is held, then the update's: each answers after the
    // slider moved on.
    const start = gate();
    const update = gate();
    const { session } = await open((m) =>
      m.type === 'applyEditDelta' ? (m.phase === 'Start' ? start.shut : m.phase === 'Update' ? update.shut : 0) : 0,
    );
    session.change([{ Exposure: 0.1 }], 'Start');
    session.change([{ Exposure: 0.8 }], 'Update');
    expect(session.shown?.exposure).toBeCloseTo(0.8);
    await nextFrame();
    start.open();
    await wait(0);
    // The start has answered; the update is still in flight.
    expect(session.edit?.edit.exposure).toBeCloseTo(0.1);
    expect(session.shown?.exposure).toBeCloseTo(0.8);
    update.open();
    session.change([{ Exposure: 0.8 }], 'End');
    await settled(session);
    expect(session.shown?.exposure).toBeCloseTo(0.8);
    expect(session.edit?.undo).toBe('Exposure');
  });

  it('says why a switch failed', async () => {
    const { session } = await open();
    await session.select(99);
    expect(session.photo).toBe(99);
    expect(session.notice).toBe('photo 99 is not in the strip');
  });

  it('ends a gesture before a switch, so the photo it leaves keeps its step', async () => {
    const update = gate();
    const { engine, session } = await open((m) => (m.type === 'applyEditDelta' && m.phase === 'Update' ? update.shut : 0));
    session.change([{ Exposure: 0.1 }], 'Start');
    session.change([{ Exposure: 0.4 }], 'Update');
    await nextFrame();
    session.change([{ Exposure: 0.7 }], 'Update');
    // Released, then the next photo at once, while an update is in flight.
    const away = session.select(2);
    update.open();
    await away;
    await session.select(1);
    expect(session.edit).toMatchObject({ undo: 'Exposure', edit: { exposure: expect.closeTo(0.7, 5) } });
    expect(session.canUndo).toBe(true);
    // The end went before the switch.
    const order = engine.sent.map((c) => (c.type === 'applyEditDelta' ? c.phase : c.type));
    expect(order.indexOf('End')).toBeLessThan(order.indexOf('selectPhoto'));
  });

  it('takes an edit reply only when its revision is the newest seen', async () => {
    // A query that answers late, with the edit as it was when asked.
    const { session } = await open((m) => (m.type === 'getEdit' ? 30 : 0));
    const stale = session.refresh();
    session.change([{ Contrast: 25 }]);
    await settled(session);
    expect(session.edit).toMatchObject({ revision: 1, edit: { contrast: 25 } });
    await stale;
    expect(session.edit).toMatchObject({ revision: 1, edit: { contrast: 25 } });
  });

  it('shows the engine’s clamped value once every command has answered', async () => {
    const { session } = await open();
    session.change([{ WhiteBalance: { mode: 'Custom', temp: 90000, tint: 0 } }]);
    await settled(session);
    expect(session.shown?.white_balance.temp).toBe(50000);
  });

  it('keeps undo and redo off during a gesture, and acts on the photo shown', async () => {
    const { engine, session } = await open();
    session.change([{ Blacks: -10 }]);
    await settled(session);
    expect(session.canUndo).toBe(true);
    session.change([{ Blacks: -20 }], 'Start');
    expect(session.canUndo).toBe(false);
    session.undo();
    session.change([{ Blacks: -20 }], 'End');
    await settled(session);
    expect(engine.sent.filter((c) => c.type === 'undo')).toEqual([]);
    session.undo();
    await settled(session);
    expect(session.edit).toMatchObject({ undo: 'Blacks', redo: 'Blacks', edit: { blacks: -10 } });
    session.redo();
    await settled(session);
    expect(session.edit?.edit.blacks).toBe(-20);
  });

  it('ignores a switch that a later switch overtook', async () => {
    const { session } = await open((m) => (m.type === 'selectPhoto' && m.photo === 5 ? 30 : 0));
    const slow = session.select(5);
    await session.select(3);
    await slow;
    expect(session.photo).toBe(3);
    expect(session.edit?.photo).toBe(3);
    // Photo 5's reason is not photo 3's.
    expect(session.reason).toBeNull();
  });

  it('keeps a dragged value while its next update waits for the frame', async () => {
    const { session } = await open();
    session.change([{ Exposure: 0.1 }], 'Start');
    session.change([{ Exposure: 0.6 }], 'Update');
    // The start has answered and nothing is in flight; the update waits.
    await wait(0);
    expect(session.edit?.edit.exposure).toBeCloseTo(0.1);
    expect(session.shown?.exposure).toBeCloseTo(0.6);
  });

  it('turns the picker off after a pick, and keeps it on with the reason when refused', async () => {
    const { session } = await open();
    session.togglePicker();
    session.pick(0.5, 0.05);
    await settled(session);
    expect(session.picking).toBe(true);
    expect(session.notice).toBe("Can't set white balance here: the area is clipped");
    session.pick(0.5, 0.5);
    await settled(session);
    expect(session.picking).toBe(false);
    expect(session.notice).toBeNull();
    expect(session.edit?.undo).toBe('White balance: picked');
  });

  it('marks the strip from each reply, and gives the reason for a photo that does not render', async () => {
    const { session } = await open();
    session.change([{ Saturation: 30 }]);
    await settled(session);
    expect(session.strip.photos[0]?.edited).toBe(true);
    await session.select(5);
    expect(session.editable).toBe(false);
    expect(session.reason).toBe('Nikon High Efficiency RAW is not supported yet');
  });

  it('shows the newest counts of the open photo, and the shown photo’s after a switch', async () => {
    const { engine, session } = await open();
    expect(session.histogram?.pixels).toBe(640 * 427);
    session.change([{ Exposure: 2 }]);
    await settled(session);
    expect(session.histogram?.highlights.pixels).toBeGreaterThan(0);
    // Photo 4 is a JPEG fitted at 640 × 480 from 4000 × 3000.
    await session.select(4);
    expect(session.histogram?.pixels).toBe(640 * 480);
    expect(session.histogram?.highlights.pixels).toBe(0);
    // A photo that does not render has none.
    await session.select(5);
    expect(session.histogram).toBeNull();
    // Counts of another photo, or older ones, are not taken.
    await session.select(1);
    const shown = session.histogram;
    engine['emit']({ type: 'histogramReady', photo: 2, generation: 1e6, histogram: { ...shown, pixels: 1 } as never });
    engine['emit']({ type: 'histogramReady', photo: 1, generation: 0, histogram: { ...shown, pixels: 2 } as never });
    expect(session.histogram).toBe(shown);
  });

  it('asks for the counts sent before a folder’s reply named its photo', async () => {
    const engine = new FakeEngine();
    const session = new DevelopSession(engine, 'srgb', () => COLORS);
    await session.openFolder('demo');
    await session.setView(640, 480);
    // The frame of the view came after: its counts by event.
    expect(session.histogram?.pixels).toBe(640 * 427);
    // Opened again, the frame is sent before the reply names photo 1 as
    // open, while the session still shows it: the query is what keeps
    // the counts once the reply clears them.
    await session.openFolder('demo');
    expect(session.histogram?.pixels).toBe(640 * 427);
  });

  it('turns both overlays with J and one with its marker, and keeps them across switches', async () => {
    const { engine, session } = await open();
    const overlays = () => engine.sent.filter((c) => c.type === 'setClippingOverlay');
    session.toggleClipping();
    expect(session.overlay).toEqual({ shadows: true, highlights: true });
    session.toggleClipping('shadows');
    expect(session.overlay).toEqual({ shadows: false, highlights: true });
    // One on: J turns both on.
    session.toggleClipping();
    expect(session.overlay).toEqual({ shadows: true, highlights: true });
    await settled(session);
    expect(overlays().at(-1)).toEqual({
      type: 'setClippingOverlay',
      shadows: true,
      highlights: true,
      shadowColor: [1, 2, 3, 255],
      highlightColor: [4, 5, 6, 255],
    });
    await session.select(2);
    expect(session.overlay).toEqual({ shadows: true, highlights: true });
    // A photo that does not render: J does nothing.
    await session.select(5);
    session.toggleClipping();
    expect(session.overlay).toEqual({ shadows: true, highlights: true });
    // Leaving Develop turns both off.
    session.clearClipping();
    await settled(session);
    expect(session.overlay).toEqual({ shadows: false, highlights: false });
    expect(overlays()).toHaveLength(4);
  });

  it('sends the Alt preview after its gesture’s start, only during a gesture, and drops it at the end', async () => {
    const start = gate();
    const { engine, session } = await open((m) => (m.type === 'applyEditDelta' && m.phase === 'Start' ? start.shut : 0));
    const order = () => engine.sent.map((c) => (c.type === 'applyEditDelta' ? c.phase : c.type === 'setClippingPreview' ? c.preview : c.type));
    session.preview('highlights');
    session.change([{ Whites: 10 }], 'Start');
    session.preview('highlights');
    session.preview('highlights');
    start.open();
    await settled(session);
    session.preview('none');
    session.preview('highlights');
    session.change([{ Whites: 20 }], 'End');
    // The engine ended it with the gesture; Alt held for the next one sends it again.
    session.change([{ Whites: 30 }], 'Start');
    session.preview('highlights');
    session.change([{ Whites: 30 }], 'End');
    await settled(session);
    expect(order()).toEqual(['Start', 'highlights', 'none', 'highlights', 'End', 'Start', 'highlights', 'End']);
  });

  it('says why the overlay colors could not be read', async () => {
    const engine = new FakeEngine();
    const session = new DevelopSession(engine, 'srgb', () => {
      throw new Error('the token --clip-shadow is not a color: ""');
    });
    await session.openFolder('demo');
    session.toggleClipping();
    expect(session.overlay).toEqual({ shadows: false, highlights: false });
    expect(session.notice).toBe('the token --clip-shadow is not a color: ""');
  });
});

describe('DevelopSession, the strip (LIB-09)', () => {
  it('steps to the next and previous photo, and not past either end', async () => {
    const { engine, session } = await open();
    expect(session.folder).toBe('demo');
    await session.step('previous');
    expect(session.strip.current).toBe(1);
    expect(engine.sent.filter((c) => c.type === 'selectPhoto')).toEqual([]);
    await session.step('next');
    await session.step('next');
    expect(session.strip.current).toBe(3);
    await session.step('previous');
    expect(session.strip.current).toBe(2);
    for (let i = 0; i < 5; i++) await session.step('next');
    expect(session.strip.current).toBe(5);
    const selects = engine.sent.filter((c) => c.type === 'selectPhoto').map((c) => c.photo);
    expect(selects).toEqual([2, 3, 2, 3, 4, 5]);
  });
});
