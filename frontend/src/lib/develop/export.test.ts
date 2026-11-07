import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ExportState } from '$contract/genzo';
import { FakeEngine, ORIGINAL } from '../engine/fake';
import { DONE_FOR_MS, Exports, exportName, folderOf, join, nameOf, withJpg } from './export.svelte';

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function opened() {
  const engine = new FakeEngine();
  const r = await engine.execute({ type: 'openFolder', path: 'demo' });
  if (r.type !== 'folder') throw new Error(r.type);
  const exports = new Exports(engine);
  return { engine, exports, photo: r.strip.current ?? 0 };
}

afterEach(() => vi.useRealTimers());

describe('export paths (OUT-01)', () => {
  it('names an export after its photo, in the folder last used', () => {
    expect(exportName('DSC_0412.NEF')).toBe('DSC_0412.jpg');
    expect(exportName('archive.tar.gz')).toBe('archive.tar.jpg');
    expect(exportName('noext')).toBe('noext.jpg');
    expect(join('C:\\Photos\\Trip', 'a.jpg')).toBe('C:\\Photos\\Trip\\a.jpg');
    expect(join('/home/me/Trip/', 'a.jpg')).toBe('/home/me/Trip/a.jpg');
    expect(join('', 'a.jpg')).toBe('a.jpg');
    expect(folderOf('C:\\Photos\\a.jpg')).toBe('C:\\Photos');
    expect(nameOf('/x/y/a.jpg')).toBe('a.jpg');
  });

  it('adds .jpg to a name without .jpg or .jpeg', () => {
    expect(withJpg('/x/holiday')).toBe('/x/holiday.jpg');
    expect(withJpg('/x/holiday.JPEG')).toBe('/x/holiday.JPEG');
    expect(withJpg('/x/holiday.png')).toBe('/x/holiday.png.jpg');
  });
});

describe('Exports (OUT-01)', () => {
  it('exports, follows its progress, and remembers the folder', async () => {
    const { exports, photo } = await opened();
    expect(exports.suggested('DSC_0412.NEF', '/photos/trip')).toBe('/photos/trip/DSC_0412.jpg');
    const started = await exports.start(photo, '/out/DSC_0412.jpg', false);
    expect(started).toEqual({ ok: true });
    await tick();
    expect(exports.status).toBe('Exported DSC_0412.jpg');
    expect(exports.busy).toBe(false);
    // The next suggestion is in the folder exported to.
    expect(exports.suggested('IMG_2201.DNG', '/photos/trip')).toBe('/out/IMG_2201.jpg');
  });

  it('a file that exists is asked about; replace writes it', async () => {
    const { exports, photo } = await opened();
    await exports.start(photo, '/out/a.jpg', false);
    const again = await exports.start(photo, '/out/a.jpg', false);
    expect(again).toEqual({ ok: false, exists: true, reason: 'a.jpg already exists' });
    expect(await exports.start(photo, '/out/a.jpg', true)).toEqual({ ok: true });
  });

  it('an original or a bad size is refused with the engine reason', async () => {
    const { exports, photo } = await opened();
    expect(await exports.start(photo, 'demo/DSC_0412.NEF', true)).toEqual({ ok: false, exists: false, reason: ORIGINAL });
    exports.settings.size = 'long';
    exports.settings.longEdge = 20;
    const small = await exports.start(photo, '/out/b.jpg', false);
    expect(small.ok).toBe(false);
    exports.settings.longEdge = 400;
    expect(await exports.start(photo, '/out/b.jpg', false)).toEqual({ ok: true });
  });

  it('sends the dialog values', async () => {
    const { engine, exports, photo } = await opened();
    exports.settings.quality = 92;
    exports.settings.size = 'long';
    exports.settings.longEdge = 1600;
    await exports.start(photo, '/out/c.jpg', true);
    expect(engine.sent.at(-1)).toEqual({
      type: 'exportJpeg',
      photo,
      path: '/out/c.jpg',
      longEdge: 1600,
      quality: 92,
      replace: true,
    });
  });

  it('shows each state, and clears "Exported" after a while', async () => {
    vi.useFakeTimers();
    const { engine, exports } = await opened();
    // Events sent by hand, as the engine would.
    const emit = (job: number, rowsDone: number, state: ExportState) =>
      engine['emit']({ type: 'exportProgress', job, photo: 1, path: '/o/x.jpg', rowsDone, rowsTotal: 200, state });
    emit(5, 0, { type: 'queued' });
    await vi.advanceTimersByTimeAsync(0);
    expect(exports.status).toBe('Waiting to export x.jpg');
    expect(exports.busy).toBe(true);
    emit(5, 81, { type: 'running' });
    await vi.advanceTimersByTimeAsync(0);
    expect(exports.status).toBe('Exporting x.jpg, 40 %');
    emit(5, 200, { type: 'finished', notes: ["The camera's maker notes were too large to keep"] });
    await vi.advanceTimersByTimeAsync(0);
    expect(exports.status).toBe("Exported x.jpg. The camera's maker notes were too large to keep");
    await vi.advanceTimersByTimeAsync(DONE_FOR_MS);
    expect(exports.status).toBeNull();
    emit(6, 0, { type: 'failed', reason: 'The disk is full' });
    await vi.advanceTimersByTimeAsync(0);
    expect(exports.status).toBe('Export failed: The disk is full');
    // An older job's events do not take the status bar back.
    emit(5, 200, { type: 'cancelled' });
    await vi.advanceTimersByTimeAsync(0);
    expect(exports.status).toBe('Export failed: The disk is full');
  });
});
