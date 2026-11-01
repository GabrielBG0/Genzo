import { describe, expect, it } from 'vitest';
import type { CommandResult, Event, PhotoEdit } from '$contract/genzo';
import { clamped, defaults, isEdited, type EditKind } from './edit';
import { FakeEngine } from './fake';
import { PhotoEdits } from './history';

const RAW: EditKind = { raw: true, asShotTemp: 5400, asShotTint: 4 };

describe('PhotoEdits, as engine::history keeps them (APP-10)', () => {
  it('records one step per finished gesture, labelled after its control', () => {
    const p = new PhotoEdits(1, RAW);
    expect(p.apply([{ Exposure: 0.1 }], 'Start')).toBe(false);
    for (let i = 2; i <= 50; i++) expect(p.apply([{ Exposure: i / 10 }], 'Update')).toBe(false);
    expect(p.undoLabel()).toBeNull();
    expect(p.apply([{ Exposure: 0.5 }], 'End')).toBe(true);
    expect(p.state()).toMatchObject({ undo: 'Exposure', redo: null, revision: 51, edited: true });
    expect(p.current().exposure).toBe(0.5);
  });

  it('records nothing for a change that leaves the edit as it was', () => {
    const p = new PhotoEdits(1, RAW);
    p.apply([{ Contrast: 20 }], 'Start');
    expect(p.apply([{ Contrast: 0 }], 'End')).toBe(false);
    p.apply([{ Whites: 100 }], 'End');
    // Clamped back to the current value.
    expect(p.apply([{ Whites: 140 }], 'End')).toBe(false);
    expect(p.undoLabel()).toBe('Whites');
    p.undo();
    expect(p.undoLabel()).toBeNull();
  });

  it('round-trips undo and redo, and a new step drops the redo steps', () => {
    const p = new PhotoEdits(1, RAW);
    p.apply([{ Exposure: 1 }], 'End');
    p.apply([{ Treatment: 'Monochrome' }], 'End');
    p.apply([{ WhiteBalance: { mode: 'Custom', temp: 3000, tint: 0 } }], 'End');
    const last = structuredClone(p.current());
    for (let i = 0; i < 3; i++) expect(p.undo()).toBe(true);
    expect(p.undo()).toBe(false);
    expect(p.current()).toEqual(defaults(RAW));
    for (let i = 0; i < 3; i++) expect(p.redo()).toBe(true);
    expect(p.current()).toEqual(last);
    p.undo();
    p.undo();
    expect(p.redoLabel()).toBe('Treatment');
    p.apply([{ Saturation: 10 }], 'End');
    expect(p.redoLabel()).toBeNull();
    expect(p.undoLabel()).toBe('Saturation');
  });

  it('does nothing on undo or redo during a gesture', () => {
    const p = new PhotoEdits(1, RAW);
    p.apply([{ Shadows: 10 }], 'End');
    p.apply([{ Shadows: 20 }], 'Start');
    expect(p.undo()).toBe(false);
    expect(p.state()).toMatchObject({ undo: null, redo: null });
  });

  it('labels resets, Reset all and mixed deltas as the engine does', () => {
    const p = new PhotoEdits(1, RAW);
    p.apply([{ Highlights: -30 }], 'End');
    p.apply([{ Reset: 'Highlights' }], 'End');
    expect(p.undoLabel()).toBe('Reset Highlights');
    p.apply([{ Whites: 5 }, { Blacks: -5 }], 'End');
    expect(p.undoLabel()).toBe('Edit');
    p.apply(['ResetAll'], 'End');
    expect(p.undoLabel()).toBe('Reset all');
    p.apply([{ WhiteBalance: { mode: 'Custom', temp: 6000, tint: 2 } }], 'End');
    expect(p.undoLabel()).toBe('White balance');
    p.apply([{ Turn: { clockwise: true } }], 'End');
    expect(p.undoLabel()).toBe('Rotate');
  });

  it('clamps as Edit::clamped and refills As Shot', () => {
    const e = clamped(
      { ...defaults(RAW), exposure: 9, white_balance: { mode: 'AsShot', temp: 9000, tint: 90 }, contrast: Number.NaN },
      RAW,
    );
    expect(e.exposure).toBe(5);
    expect(e.contrast).toBe(0);
    expect(e.white_balance).toEqual({ mode: 'AsShot', temp: 5400, tint: 4 });
    const custom = clamped({ ...defaults(RAW), white_balance: { mode: 'Custom', temp: 900, tint: -400 } }, RAW);
    expect(custom.white_balance).toEqual({ mode: 'Custom', temp: 2000, tint: -150 });
    // Temp and Tint count only when custom.
    expect(isEdited({ ...defaults(RAW), white_balance: { mode: 'AsShot', temp: 1, tint: 1 } }, defaults(RAW))).toBe(false);
  });
});

describe('FakeEngine, Develop', () => {
  async function opened() {
    const engine = new FakeEngine();
    const events: Event[] = [];
    engine.onEvent((e) => events.push(e));
    const r = await engine.execute({ type: 'openFolder', path: 'demo' });
    if (r.type !== 'folder') throw new Error(r.type);
    await engine.execute({ type: 'setView', width: 640, height: 480, view: { type: 'fit' }, space: 'srgb' });
    await Promise.resolve();
    return { engine, events, folder: r };
  }

  const edited = (r: CommandResult): PhotoEdit => {
    if (r.type !== 'edit') throw new Error(r.type);
    return r.edit;
  };

  it('opens the demo folder at its first photo', async () => {
    const { folder } = await opened();
    expect(folder.strip.current).toBe(1);
    expect(folder.strip.photos.map((p) => p.kind)).toEqual(['renders', 'renders', 'renders', 'renders', 'unsupported']);
    expect(folder.edit).toMatchObject({ photo: 1, edited: false, undo: null, revision: 0 });
  });

  it('sends each settled frame of the open photo, then its counts', async () => {
    const { engine, events } = await opened();
    events.length = 0;
    await engine.execute({ type: 'applyEditDelta', photo: 1, changes: [{ Exposure: 1 }], phase: 'Start' });
    await engine.execute({ type: 'applyEditDelta', photo: 1, changes: [{ Exposure: 1 }], phase: 'End' });
    await Promise.resolve();
    expect(events.map((e) => [e.type, 'photo' in e ? e.photo : null])).toEqual([
      ['renderFinished', 1],
      ['renderFinished', 1],
      ['histogramReady', 1],
    ]);
    const [, settled, counts] = events;
    if (settled?.type !== 'renderFinished' || counts?.type !== 'histogramReady') throw new Error('order');
    expect(counts.generation).toBe(settled.generation);
    expect(counts.histogram.pixels).toBe(640 * 427);
  });

  it('changes nothing for a photo not open, and keeps each photo its history', async () => {
    const { engine } = await opened();
    edited(await engine.execute({ type: 'applyEditDelta', photo: 1, changes: [{ Exposure: 1 }], phase: 'End' }));
    const selected = await engine.execute({ type: 'selectPhoto', photo: 2 });
    expect(selected).toMatchObject({ type: 'selected', photo: 2, reason: null });
    edited(await engine.execute({ type: 'applyEditDelta', photo: 2, changes: [{ Blacks: -10 }], phase: 'End' }));
    // Photo 1 is not open: its undo changes nothing.
    expect(edited(await engine.execute({ type: 'undo', photo: 1 }))).toMatchObject({ undo: 'Exposure', revision: 1 });
    await engine.execute({ type: 'selectPhoto', photo: 1 });
    expect(edited(await engine.execute({ type: 'undo', photo: 1 }))).toMatchObject({ undo: null, edited: false });
    const strip = await engine.query({ type: 'listFolderPhotos' });
    if (strip.type !== 'folderPhotos') throw new Error(strip.type);
    expect(strip.strip.photos.map((p) => p.edited)).toEqual([false, true, false, false, false]);
  });

  it('ends a gesture on a switch, and keeps edits when the folder opens again', async () => {
    const { engine } = await opened();
    await engine.execute({ type: 'applyEditDelta', photo: 1, changes: [{ Exposure: 0.4 }], phase: 'Start' });
    await engine.execute({ type: 'selectPhoto', photo: 2 });
    const reopened = await engine.execute({ type: 'openFolder', path: 'demo' });
    expect(reopened).toMatchObject({ type: 'folder', edit: { photo: 1, undo: 'Exposure', edited: true } });
    if (reopened.type !== 'folder') throw new Error(reopened.type);
    expect(reopened.strip.photos[0]?.edited).toBe(true);
  });

  it('answers a photo that does not render with its reason', async () => {
    const { engine } = await opened();
    expect(await engine.execute({ type: 'selectPhoto', photo: 5 })).toEqual({
      type: 'selected',
      photo: 5,
      edit: null,
      reason: 'Nikon High Efficiency RAW is not supported yet',
    });
  });

  it('picks a white, or refuses a clipped area with the reason', async () => {
    const { engine } = await opened();
    const refused = await engine.execute({ type: 'pickWhiteBalance', photo: 1, x: 0.5, y: 0.05 });
    expect(refused).toMatchObject({ recorded: false, refused: "Can't set white balance here: the area is clipped" });
    const picked = await engine.execute({ type: 'pickWhiteBalance', photo: 1, x: 0.5, y: 0.5 });
    expect(picked).toMatchObject({ recorded: true, refused: null, edit: { undo: 'White balance: picked' } });
    expect(edited(picked).edit.white_balance).toEqual({ mode: 'Custom', temp: 5500, tint: 0 });
  });
});
