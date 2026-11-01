import { describe, expect, it } from 'vitest';
import type { Command } from '$contract/genzo';
import { FakeEngine } from '../engine/fake';
import { DevelopSession } from './session.svelte';

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
  const session = new DevelopSession(engine);
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
});
