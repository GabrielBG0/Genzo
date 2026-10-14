import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import SliderRow from './SliderRow.svelte';

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
    expect(onchange).toHaveBeenLastCalledWith(1);
    await fireEvent.keyDown(slider, { key: 'ArrowLeft', shiftKey: true });
    expect(onchange).toHaveBeenLastCalledWith(-9);
    await fireEvent.dblClick(slider);
    expect(onchange).toHaveBeenLastCalledWith(0);
    expect(slider).toHaveAttribute('aria-valuenow', '0');
  });

  it('clamps to its range', async () => {
    const onchange = vi.fn();
    render(SliderRow, { label: 'Whites', value: 100, onchange });
    await fireEvent.keyDown(screen.getByRole('slider'), { key: 'ArrowUp' });
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '100');
  });
});
