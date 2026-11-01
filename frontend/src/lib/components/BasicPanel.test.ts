import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { DevelopSession } from '../develop/session.svelte';
import { MIRED_SCALE, presetName } from '../develop/whiteBalance';
import { defaults } from '../engine/edit';
import { FakeEngine, PRESETS } from '../engine/fake';
import BasicPanel from './BasicPanel.svelte';

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function panel(photo = 1) {
  const engine = new FakeEngine();
  const session = new DevelopSession(engine);
  await session.openFolder('demo');
  if (photo !== 1) await session.select(photo);
  const view = render(BasicPanel, { session, presets: PRESETS });
  return { engine, session, view };
}

/** Lets every command sent answer, and the panel update. */
async function settle() {
  await wait(20);
}

const slider = (name: string) => screen.getByRole('slider', { name });
const row = (name: string) => slider(name);

describe('BasicPanel (DEV-02)', () => {
  it('places RAW Temp linearly in mireds, 5000 K at 62.5 %', () => {
    expect(MIRED_SCALE.toT(2000)).toBe(0);
    expect(MIRED_SCALE.toT(50000)).toBeCloseTo(1, 12);
    expect(MIRED_SCALE.toT(5000)).toBeCloseTo(0.625, 12);
    for (let k = 2000; k <= 50000; k += 250) expect(Math.abs(MIRED_SCALE.fromT(MIRED_SCALE.toT(k)) - k)).toBeLessThan(1);
  });

  it('names a preset when the values match it, Custom when not, As Shot in as-shot mode', () => {
    const e = defaults({ raw: true, asShotTemp: 5400, asShotTint: 4 });
    expect(presetName(e, PRESETS)).toBe('As Shot');
    const at = (temp: number, tint: number) => ({ ...e, white_balance: { mode: 'Custom' as const, temp, tint } });
    expect(presetName(at(6503.2, 9.6), PRESETS)).toBe('Cloudy');
    expect(presetName(at(2856, 0), PRESETS)).toBe('Tungsten');
    expect(presetName(at(2858, 0), PRESETS)).toBe('Custom');
    // No two presets round to the same shown values.
    const shown = new Set(PRESETS.map((p) => `${Math.round(p.temp)} ${Math.round(p.tint)}`));
    expect(shown.size).toBe(PRESETS.length);
  });

  it('shows changed sliders bright and the header dot; Temp and Tint only when custom', async () => {
    const { session, view } = await panel();
    const dot = () => view.container.querySelector('.header .dot');
    expect(dot()).toBeNull();
    expect(row('Exposure')).not.toHaveClass('changed');
    expect(row('Temp')).not.toHaveClass('changed');
    expect(slider('Temp')).toHaveAttribute('aria-valuetext', '5400');
    session.change([{ Exposure: 0.35 }]);
    await settle();
    expect(row('Exposure')).toHaveClass('changed');
    expect(slider('Exposure')).toHaveAttribute('aria-valuetext', '+0.35');
    expect(dot()).not.toBeNull();
    session.change([{ WhiteBalance: { mode: 'Custom', temp: 5400, tint: 4 } }]);
    await settle();
    // Custom at the as-shot values is still an edit.
    expect(row('Temp')).toHaveClass('changed');
    expect(row('Tint')).toHaveClass('changed');
  });

  it('makes a held arrow key one step, and a double-click on the label a reset', async () => {
    const { engine, session } = await panel();
    const contrast = slider('Contrast');
    await fireEvent.keyDown(contrast, { key: 'ArrowRight' });
    await fireEvent.keyDown(contrast, { key: 'ArrowRight', repeat: true });
    await fireEvent.keyDown(contrast, { key: 'ArrowRight', shiftKey: true, repeat: true });
    await fireEvent.keyUp(contrast, { key: 'ArrowRight' });
    await settle();
    // A start and an end: the updates within the frame gave way to the end.
    expect(session.edit).toMatchObject({ undo: 'Contrast', revision: 2, edit: { contrast: 12 } });
    await fireEvent.dblClick(screen.getByText('Contrast'));
    await settle();
    expect(session.edit).toMatchObject({ undo: 'Reset Contrast', edit: { contrast: 0 } });
    expect(engine.sent.at(-1)).toEqual({ type: 'resetParam', photo: 1, param: 'Contrast' });
  });

  it('starts Temp at the as-shot value and sets Custom; its reset is As Shot', async () => {
    const { engine, session } = await panel();
    await fireEvent.keyDown(slider('Temp'), { key: 'ArrowRight' });
    await fireEvent.keyUp(slider('Temp'), { key: 'ArrowRight' });
    await settle();
    expect(engine.sent.find((c) => c.type === 'applyEditDelta')).toMatchObject({
      phase: 'Start',
      changes: [{ WhiteBalance: { mode: 'Custom', temp: 5450, tint: 4 } }],
    });
    expect(screen.getByRole('combobox', { name: 'White balance preset' })).toHaveValue('Custom');
    await fireEvent.dblClick(screen.getByText('Tint'));
    await settle();
    expect(session.edit?.edit.white_balance.mode).toBe('AsShot');
    expect(screen.getByRole('combobox', { name: 'White balance preset' })).toHaveValue('As Shot');
  });

  it('chooses a preset as one step and names it', async () => {
    const { session } = await panel();
    const preset = screen.getByRole('combobox', { name: 'White balance preset' });
    await fireEvent.change(preset, { target: { value: 'Tungsten' } });
    await settle();
    expect(session.edit?.undo).toBe('White balance');
    expect(slider('Temp')).toHaveAttribute('aria-valuetext', '2856');
    expect(preset).toHaveValue('Tungsten');
  });

  it('hides Vibrance and Saturation in B&W and keeps their values for Color', async () => {
    const { session } = await panel();
    session.change([{ Vibrance: 18 }]);
    await settle();
    const treatment = screen.getByRole('radiogroup', { name: 'Treatment' });
    await fireEvent.click(within(treatment).getByRole('radio', { name: /B&W/ }));
    await settle();
    expect(session.edit?.edit.treatment).toBe('Monochrome');
    expect(screen.queryByRole('slider', { name: 'Vibrance' })).toBeNull();
    await fireEvent.click(within(treatment).getByRole('radio', { name: 'Color' }));
    await settle();
    expect(slider('Vibrance')).toHaveAttribute('aria-valuetext', '+18');
  });

  it('shows a non-RAW photo as Embedded, with relative Temp and two white balance entries', async () => {
    await panel(4);
    const profile = screen.getByRole('combobox', { name: 'Profile' });
    expect(profile).toBeDisabled();
    expect(profile).toHaveValue('Embedded');
    const entries = within(screen.getByRole('combobox', { name: 'White balance preset' })).getAllByRole('option');
    expect(entries.map((o) => o.textContent)).toEqual(['As Shot', 'Custom']);
    expect(slider('Temp')).toHaveAttribute('aria-valuemin', '-100');
    expect(slider('Temp')).toHaveAttribute('aria-valuetext', '0');
  });

  it('disables every control, values blank, for a photo that does not render', async () => {
    await panel(5);
    for (const s of screen.getAllByRole('slider')) {
      expect(s).toHaveAttribute('aria-disabled', 'true');
      expect(s).toHaveAttribute('aria-valuetext', '');
    }
    expect(screen.getByRole('combobox', { name: 'Profile' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'White balance picker' })).toBeDisabled();
  });
});
