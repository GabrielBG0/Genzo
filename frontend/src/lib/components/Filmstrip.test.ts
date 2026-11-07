import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import Filmstrip, { visibleRange, type FilmstripItem } from './Filmstrip.svelte';

const item = (i: number, extra: Partial<FilmstripItem> = {}): FilmstripItem => ({
  id: String(i),
  name: `IMG_${i}.dng`,
  ...extra,
});

describe('Filmstrip (LIB-09)', () => {
  it('finds the visible cells: 108 px cells, 8 px gaps, 10 px padding', () => {
    // 116 px a cell: in 500 px from the start, cells 0 to 4 (4 ends at 582).
    expect(visibleRange(0, 500, 100)).toEqual({ first: 0, last: 4 });
    // Cell 0 ends at 10 + 108 = 118: its last pixel shows at 117, not at 118.
    expect(visibleRange(117, 500, 100)).toEqual({ first: 0, last: 5 });
    expect(visibleRange(118, 500, 100)).toEqual({ first: 1, last: 5 });
    // Cell 5 starts at 10 + 5 * 116 = 590: shown once the right edge passes it.
    expect(visibleRange(90, 500, 100)).toEqual({ first: 0, last: 4 });
    expect(visibleRange(91, 500, 100)).toEqual({ first: 0, last: 5 });
    // Clamped to the strip.
    expect(visibleRange(10_000, 500, 20)).toEqual({ first: 19, last: 19 });
    expect(visibleRange(0, 500, 0)).toEqual({ first: 0, last: -1 });
    // Not measured yet: everything counts as visible.
    expect(visibleRange(0, 0, 3)).toEqual({ first: 0, last: 2 });
  });

  it('rings the current photo, dots edited ones, names those without a thumbnail, dims missing ones', async () => {
    const onselect = vi.fn();
    render(Filmstrip, {
      items: [item(1, { edited: true }), item(2, { noThumb: true }), item(3, { missing: true })],
      selected: '2',
      onselect,
    });
    const frames = screen.getAllByRole('button');
    const [first, second, third] = frames as [HTMLElement, HTMLElement, HTMLElement];
    expect(frames.map((f) => f.getAttribute('aria-current'))).toEqual(['false', 'true', 'false']);
    expect(first.querySelector('[aria-label="Edited"]')).not.toBeNull();
    expect(second.querySelector('[aria-label="Edited"]')).toBeNull();
    // Only the cell without a thumbnail shows its name; the others wait gray.
    expect(screen.getAllByText(/IMG_\d\.dng/).map((n) => n.textContent)).toEqual(['IMG_2.dng']);
    expect(third.classList.contains('missing')).toBe(true);
    await fireEvent.click(third);
    expect(onselect).toHaveBeenCalledWith('3');
  });

  it('says what to do with no folder open, and with an empty one', () => {
    const { rerender } = render(Filmstrip, { items: [], message: 'Open a folder', shortcut: 'Ctrl O' });
    expect(screen.getByText('Open a folder')).toBeTruthy();
    expect(screen.getByText('Ctrl O').tagName).toBe('KBD');
    rerender({ items: [], message: 'No photos in this folder: Trip', shortcut: '' });
    expect(screen.getByText('No photos in this folder: Trip')).toBeTruthy();
    expect(screen.queryByText('Ctrl O')).toBeNull();
  });

  it('reports its visible cells', () => {
    const onvisible = vi.fn();
    render(Filmstrip, { items: [item(1), item(2), item(3)], onvisible });
    // Unmeasured in jsdom: every cell.
    expect(onvisible).toHaveBeenLastCalledWith(0, 2);
  });
});
