import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import SliderRow, { parseTyped } from './SliderRow.svelte';

describe('SliderRow', () => {
  it('shows unchanged values dim and changed values bright', () => {
    const { container, rerender } = render(SliderRow, { label: 'Exposure', value: 0 });
    const row = container.querySelector('.row');
    expect(row).not.toHaveClass('changed');
    rerender({ label: 'Exposure', value: 12 });
    expect(row).toHaveClass('changed');
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuetext', '+12');
  });

  it('steps with the arrow keys and resets on double-click', async () => {
    const onchange = vi.fn();
    render(SliderRow, { label: 'Contrast', value: 0, onchange });
    const slider = screen.getByRole('slider');
    await fireEvent.keyDown(slider, { key: 'ArrowRight' });
    await fireEvent.keyUp(slider, { key: 'ArrowRight' });
    expect(onchange).toHaveBeenLastCalledWith(1);
    await fireEvent.keyDown(slider, { key: 'ArrowLeft', shiftKey: true });
    await fireEvent.keyUp(slider, { key: 'ArrowLeft' });
    expect(onchange).toHaveBeenLastCalledWith(-9);
    await fireEvent.dblClick(screen.getByText('Contrast'));
    expect(onchange).toHaveBeenLastCalledWith(0);
    expect(slider).toHaveAttribute('aria-valuenow', '0');
  });

  it('makes a held arrow key one gesture', async () => {
    const onstart = vi.fn();
    const oninput = vi.fn();
    const onchange = vi.fn();
    render(SliderRow, { label: 'Exposure', value: 0, min: -5, max: 5, step: 0.05, shiftStep: 0.25, onstart, oninput, onchange });
    const slider = screen.getByRole('slider');
    for (let i = 0; i < 4; i++) await fireEvent.keyDown(slider, { key: 'ArrowRight', repeat: i > 0 });
    await fireEvent.keyDown(slider, { key: 'ArrowRight', shiftKey: true, repeat: true });
    expect(onchange).not.toHaveBeenCalled();
    await fireEvent.keyUp(slider, { key: 'ArrowRight' });
    expect(onstart.mock.calls).toEqual([[0.05]]);
    expect(oninput.mock.calls).toEqual([[0.1], [0.15], [0.2], [0.45]]);
    expect(onchange.mock.calls).toEqual([[0.45]]);
  });

  it('takes a typed value as one change, and refuses what does not parse', async () => {
    const onstart = vi.fn();
    const onchange = vi.fn();
    render(SliderRow, { label: 'Exposure', value: 0, min: -5, max: 5, step: 0.05, onstart, onchange });
    await fireEvent.click(screen.getByText('0.00'));
    const field = screen.getByRole('textbox', { name: 'Exposure value' });
    await fireEvent.input(field, { target: { value: '+0,35' } });
    await fireEvent.keyDown(field, { key: 'Enter' });
    expect(onchange.mock.calls).toEqual([[0.35]]);
    await fireEvent.click(screen.getByText('+0.35'));
    await fireEvent.input(screen.getByRole('textbox'), { target: { value: 'bright' } });
    await fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    await fireEvent.click(screen.getByText('+0.35'));
    await fireEvent.input(screen.getByRole('textbox'), { target: { value: '9' } });
    await fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' });
    expect(onchange.mock.calls).toEqual([[0.35]]);
    expect(onstart).not.toHaveBeenCalled();
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '0.35');
  });

  it('reads typed values as DEV-02 says', () => {
    expect(parseTyped('5400K')).toBe(5400);
    expect(parseTyped('+0,35')).toBe(0.35);
    expect(parseTyped('−12')).toBe(-12);
    expect(parseTyped(' 7 ')).toBe(7);
    expect(parseTyped('12abc3')).toBeNull();
    expect(parseTyped('')).toBeNull();
  });

  it('clamps to its range', async () => {
    const onchange = vi.fn();
    render(SliderRow, { label: 'Whites', value: 100, onchange });
    await fireEvent.keyDown(screen.getByRole('slider'), { key: 'ArrowUp' });
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '100');
  });
});


/** A track 200 px wide at x 0, and pointer capture, which jsdom lacks. */
function layout(container: HTMLElement): HTMLElement {
  const track = container.querySelector('.track') as HTMLElement;
  track.getBoundingClientRect = () => ({ left: 0, right: 200, width: 200, top: 0, bottom: 4, height: 4, x: 0, y: 0, toJSON: () => ({}) });
  track.setPointerCapture = () => {};
  return track;
}

describe('SliderRow: detent, fine adjust and reset (DEV-02 criterion 11)', () => {
  it('catches at 0 within 1.5 % of the track, lights the tick, and not with Shift', async () => {
    const oninput = vi.fn();
    const { container } = render(SliderRow, { label: 'Contrast', value: 0, oninput });
    const track = layout(container);
    // 0 sits at 100 px; 1.5 % of 200 px is 3 px.
    await fireEvent.pointerDown(track, { button: 0, clientX: 150 });
    await fireEvent.pointerMove(track, { clientX: 102 });
    expect(oninput).toHaveBeenLastCalledWith(0);
    expect(container.querySelector('.detent')).toHaveClass('caught');
    await fireEvent.pointerMove(track, { clientX: 110 });
    expect(oninput).toHaveBeenLastCalledWith(10);
    expect(container.querySelector('.detent')).not.toHaveClass('caught');
    await fireEvent.pointerUp(track);
    // With Shift, 102 px is 2.
    await fireEvent.pointerDown(track, { button: 0, clientX: 102, shiftKey: true });
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '2');
    await fireEvent.pointerUp(track);
  });

  it('has no tick on a slider that does not span 0', () => {
    const { container } = render(SliderRow, { label: 'Temp', value: 5000, min: 2000, max: 50000, step: 50 });
    expect(container.querySelector('.detent')).toBeNull();
  });

  it('moves at a tenth of the pointer with Shift, without a jump either way', async () => {
    const oninput = vi.fn();
    const { container } = render(SliderRow, { label: 'Contrast', value: 0, oninput });
    const track = layout(container);
    await fireEvent.pointerDown(track, { button: 0, clientX: 150 });
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '50');
    // Shift goes down at 150: 100 px more is 10 px, so 50 → 60.
    await fireEvent.pointerMove(track, { clientX: 150, shiftKey: true });
    expect(container.querySelector('.row')).toHaveClass('fine');
    await fireEvent.pointerMove(track, { clientX: 250, shiftKey: true });
    expect(oninput).toHaveBeenLastCalledWith(60);
    // Shift up at 250: full speed again from 60, so 10 px more is 70.
    await fireEvent.pointerMove(track, { clientX: 250 });
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '60');
    await fireEvent.pointerMove(track, { clientX: 260 });
    expect(oninput).toHaveBeenLastCalledWith(70);
    await fireEvent.pointerUp(track);
  });

  it('animates the thumb home after a reset, and never during a drag', async () => {
    vi.useFakeTimers();
    const onreset = vi.fn();
    const { container } = render(SliderRow, { label: 'Contrast', value: 30, onreset });
    const row = container.querySelector('.row') as HTMLElement;
    await fireEvent.dblClick(screen.getByText('Contrast'));
    expect(onreset).toHaveBeenCalledOnce();
    expect(row).toHaveClass('resetting');
    vi.advanceTimersByTime(200);
    await Promise.resolve();
    expect(row).not.toHaveClass('resetting');
    const track = layout(container);
    await fireEvent.pointerDown(track, { button: 0, clientX: 50 });
    expect(row).not.toHaveClass('resetting');
    expect(row).toHaveClass('held');
    vi.useRealTimers();
  });

  it('is an opaque pill with no detent as an aux slider', () => {
    const { container } = render(SliderRow, { label: 'Quality', value: 80, min: 0, max: 100, kind: 'aux' });
    expect(container.querySelector('.row')).toHaveClass('aux');
    expect(container.querySelector('.line')).toBeNull();
    expect(container.querySelector('.detent')).toBeNull();
  });
});
