<script lang="ts">
  import type { Change, Edit, Param, Profile, WhiteBalancePreset } from '$contract/genzo';
  import type { DevelopSession } from '../develop/session.svelte';
  import { MIRED_SCALE, presetName } from '../develop/whiteBalance';
  import { EXPOSURE, PARAM_OF, RAW_TEMP, RAW_TINT, SLIDER, type ValueField } from '../engine/edit';
  import { useRegistry } from '../shortcuts/context';
  import Icon from './Icon.svelte';
  import PanelHeader from './PanelHeader.svelte';
  import SliderRow from './SliderRow.svelte';

  interface Props {
    session: DevelopSession;
    /** The RAW presets, from the engine's info. */
    presets: WhiteBalancePreset[];
  }
  let { session, presets }: Props = $props();
  const registry = useRegistry();

  let open = $state(true);

  /** The edit shown, or null with every control disabled and blank. */
  const edit: Edit | null = $derived(session.editable ? session.shown : null);
  const raw = $derived(edit?.profile !== 'Embedded');
  const custom = $derived(edit?.white_balance.mode === 'Custom');
  const mono = $derived(edit?.treatment === 'Monochrome');

  const TONE: ValueField[] = ['exposure', 'contrast', 'highlights', 'shadows', 'whites', 'blacks'];
  const PRESENCE: ValueField[] = ['vibrance', 'saturation'];
  const LABEL: Record<ValueField, string> = {
    exposure: 'Exposure',
    contrast: 'Contrast',
    highlights: 'Highlights',
    shadows: 'Shadows',
    whites: 'Whites',
    blacks: 'Blacks',
    vibrance: 'Vibrance',
    saturation: 'Saturation',
  };

  /** Anything in the panel differs from its default: the header's dot (DEV-13). */
  const edited = $derived(
    edit !== null &&
      (edit.profile !== (raw ? 'GenzoStandard' : 'Embedded') ||
        edit.treatment !== 'Color' ||
        custom ||
        [...TONE, ...PRESENCE].some((f) => edit[f] !== 0)),
  );

  const temp = $derived(edit?.white_balance.temp ?? 0);
  const tint = $derived(edit?.white_balance.tint ?? 0);

  const presetShown = $derived(edit ? presetName(edit, raw ? presets : []) : '');

  function valueChange(f: ValueField, v: number): Change {
    return { [PARAM_OF[f]]: v } as Change;
  }

  function white(temp: number, tint: number): Change {
    return { WhiteBalance: { mode: 'Custom', temp, tint } };
  }

  function choosePreset(name: string) {
    if (!edit) return;
    if (name === 'As Shot') {
      session.reset('WhiteBalance');
      return;
    }
    const p = presets.find((q) => q.name === name);
    if (p) session.change([white(p.temp, p.tint)]);
  }

  /** A double-click on a group label resets the group: one step. */
  function resetGroup(fields: ValueField[]) {
    session.change(fields.map((f): Change => ({ Reset: PARAM_OF[f] as Param })));
  }

  const formatExposure = (v: number) => v.toFixed(2);
  const formatWhole = (v: number) => String(Math.round(v));
  const blank = () => '';
</script>

<section class="basic" aria-label="Basic">
  <PanelHeader heading="Basic" {edited} {open} ontoggle={() => (open = !open)} />
  {#if open}
    <div class="profile">
      <span class="choice pill">
        <select
          aria-label="Profile"
          disabled={!edit || !raw}
          value={edit?.profile ?? ''}
          onchange={(e) => session.change([{ Profile: e.currentTarget.value as Profile }])}
        >
          {#if raw}
            <option value="GenzoStandard">Genzo Standard</option>
            <option value="GenzoLinear">Genzo Linear</option>
          {:else}
            <option value="Embedded">Embedded</option>
          {/if}
        </select>
        <span class="chevron" aria-hidden="true">⌄</span>
      </span>
      <div class="toggle" role="radiogroup" aria-label="Treatment" title={registry.title('develop.treatment')}>
        <button
          role="radio"
          aria-checked={!mono}
          class={[!mono && edit && 'lens']}
          disabled={!edit}
          onclick={() => mono && session.change([{ Treatment: 'Color' }])}>Color</button
        >
        <button
          role="radio"
          aria-checked={mono}
          class={[mono && 'lens']}
          disabled={!edit}
          onclick={() => !mono && session.change([{ Treatment: 'Monochrome' }])}
          >B&amp;W <span class="key mono">{registry.shortcut('develop.treatment')}</span></button
        >
      </div>
    </div>

    <div class="group">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <span ondblclick={() => edit && session.reset('WhiteBalance')}>White balance</span>
      <span class="wb">
        <span class="choice preset">
          <select
            aria-label="White balance preset"
            disabled={!edit}
            value={presetShown}
            onchange={(e) => choosePreset(e.currentTarget.value)}
          >
            <option>As Shot</option>
            {#each raw ? presets : [] as p (p.name)}<option>{p.name}</option>{/each}
            <!-- Custom is shown, not chosen. -->
            <option disabled>Custom</option>
          </select>
          <span class="chevron" aria-hidden="true">⌄</span>
        </span>
        <button
          class={['picker', session.picking && 'lens']}
          aria-label="White balance picker"
          aria-pressed={session.picking}
          title={registry.title('develop.picker')}
          disabled={!edit}
          onclick={() => session.togglePicker()}><Icon name="picker" size={13} /></button
        >
      </span>
    </div>
    <SliderRow
      label="Temp"
      value={edit?.white_balance.temp ?? (raw ? 5000 : 0)}
      min={raw ? RAW_TEMP[0] : SLIDER[0]}
      max={raw ? RAW_TEMP[1] : SLIDER[1]}
      step={raw ? 50 : 1}
      shiftStep={raw ? 500 : 10}
      from={raw ? 'left' : 'center'}
      scale={raw ? MIRED_SCALE : undefined}
      track="temp"
      dim={!custom}
      disabled={!edit}
      format={edit ? formatWhole : blank}
      onstart={(v) => session.change([white(v, tint)], 'Start')}
      oninput={(v) => session.change([white(v, tint)], 'Update')}
      onchange={(v) => session.change([white(v, tint)], 'End')}
      onreset={() => session.reset('WhiteBalance')}
    />
    <SliderRow
      label="Tint"
      value={edit?.white_balance.tint ?? 0}
      min={raw ? RAW_TINT[0] : SLIDER[0]}
      max={raw ? RAW_TINT[1] : SLIDER[1]}
      track="tint"
      dim={!custom}
      disabled={!edit}
      format={edit ? formatWhole : blank}
      onstart={(v) => session.change([white(temp, v)], 'Start')}
      oninput={(v) => session.change([white(temp, v)], 'Update')}
      onchange={(v) => session.change([white(temp, v)], 'End')}
      onreset={() => session.reset('WhiteBalance')}
    />

    {#snippet group(name: string, fields: ValueField[])}
      <div class="group">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <span ondblclick={() => edit && resetGroup(fields)}>{name}</span>
      </div>
      {#each fields as f (f)}
        <SliderRow
          label={LABEL[f]}
          value={edit?.[f] ?? 0}
          min={f === 'exposure' ? EXPOSURE[0] : SLIDER[0]}
          max={f === 'exposure' ? EXPOSURE[1] : SLIDER[1]}
          step={f === 'exposure' ? 0.05 : 1}
          shiftStep={f === 'exposure' ? 0.25 : 10}
          disabled={!edit}
          format={!edit ? blank : f === 'exposure' ? formatExposure : formatWhole}
          onstart={(v) => session.change([valueChange(f, v)], 'Start')}
          oninput={(v) => session.change([valueChange(f, v)], 'Update')}
          onchange={(v) => session.change([valueChange(f, v)], 'End')}
          onreset={() => session.reset(PARAM_OF[f])}
        />
      {/each}
    {/snippet}
    {@render group('Tone', TONE)}
    <!-- B&W hides Vibrance and Saturation; their values stay for Color. -->
    {#if !mono}{@render group('Presence', PRESENCE)}{/if}
  {/if}
</section>

<style>
  .profile {
    display: flex;
    gap: 6px;
    padding: 0 10px 4px 26px;
  }

  .choice {
    position: relative;
    display: flex;
    align-items: center;
  }

  select {
    appearance: none;
    width: 100%;
    font: inherit;
    color: var(--text-body);
    background: none;
    border: 0;
    cursor: pointer;
  }

  select:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .chevron {
    position: absolute;
    right: 10px;
    top: 50%;
    transform: translateY(-65%);
    color: var(--text-muted);
    pointer-events: none;
  }

  .pill {
    flex: 1;
    min-width: 0;
    height: 24px;
    border: 1px solid var(--field);
    border-radius: var(--r-pill);
    background: var(--trough);
  }

  .pill select {
    height: 100%;
    padding: 0 24px 0 10px;
  }

  .toggle {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r-pill);
    background: var(--trough);
  }

  .toggle button {
    padding: 1px 8px;
    border-radius: var(--r-pill);
    border: 1px solid transparent;
  }

  .toggle button:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .key {
    font-size: 10px;
    color: var(--text-muted);
  }

  .group {
    height: 20px;
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    padding: 0 12px 0 26px;
    font-size: var(--text-caption);
    color: var(--text-muted);
  }

  .wb {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .preset select {
    padding-right: 12px;
    color: var(--text-muted);
    font-size: var(--text-caption);
    text-align: right;
  }

  .preset .chevron {
    right: 0;
  }

  .picker {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: var(--r-pill);
    border: 1px solid transparent;
    color: var(--text-muted);
  }

  .picker:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
