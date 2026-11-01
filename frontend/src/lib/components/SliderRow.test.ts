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
