<script lang="ts">
  import { ASPECTS, pixelSize } from '../develop/crop';
  import type { CropTool } from '../develop/cropTool.svelte';
  import type { DevelopSession } from '../develop/session.svelte';
  import { useRegistry } from '../shortcuts/context';
  import Icon from './Icon.svelte';
  import PanelHeader from './PanelHeader.svelte';
  import SliderRow from './SliderRow.svelte';

  interface Props {
    session: DevelopSession;
    tool: CropTool;
  }
  let { session, tool }: Props = $props();
  const registry = useRegistry();

  const edit = $derived(session.editable ? session.shown : null);
  /** The crop or the orientation differs from the defaults (DEV-13's dot). */
  const edited = $derived.by(() => {
    if (!edit) return false;
    const c = edit.crop;
    const whole = c.cx === 0.5 && c.cy === 0.5 && c.w === 1 && c.h === 1 && c.angle === 0;
    return !whole || edit.quarter_turns !== 0 || edit.flip_horizontal;
  });
  const size = $derived(tool.crop && tool.size.width > 0 ? pixelSize(tool.crop, tool.size) : null);
  const aspect = $derived(tool.aspect);

  function run(id: string) {
    const cmd = registry.get(id);
    if (cmd.enabled()) cmd.run();
  }

  const formatAngle = (v: number) => `${v > 0 ? '+' : ''}${v.toFixed(1)}°`;
</script>

<section class="crop" aria-label="Crop and rotate">
  <PanelHeader heading="Crop and rotate" {edited} />
  <div class="body">
    <div class="row">
      <span class="label">Aspect</span>
      <span class="choice pill">
        <select
          aria-label="Aspect"
          disabled={!edit}
          value={aspect}
          onchange={(e) => tool.choose(e.currentTarget.value)}
        >
          {#each ASPECTS as a (a.label)}<option>{a.label}</option>{/each}
          <!-- Custom is shown, not chosen: a ratio no preset matches. -->
          <option disabled>Custom</option>
        </select>
        <span class="chevron" aria-hidden="true">⌄</span>
      </span>
      <button
        class={['icon', tool.locked && 'on']}
        aria-label="Lock the aspect"
        aria-pressed={tool.locked}
        title="Lock the aspect"
        disabled={!edit}
        onclick={() => (tool.locked = !tool.locked)}
      >
        <Icon name={tool.locked ? 'lock' : 'unlock'} size={13} />
      </button>
      <button class="text" title={registry.title('crop.swap')} disabled={!edit} onclick={() => run('crop.swap')}>
        Swap <kbd class="mono">{registry.shortcut('crop.swap')}</kbd>
      </button>
    </div>

    <SliderRow
      label="Angle"
      value={edit?.crop.angle ?? 0}
      min={-45}
      max={45}
      step={0.1}
      shiftStep={1}
      disabled={!edit}
      format={edit ? formatAngle : () => ''}
      dim={(edit?.crop.angle ?? 0) === 0}
      onstart={(v) => tool.setAngle(v, 'Start')}
      oninput={(v) => tool.setAngle(v, 'Update')}
      onchange={(v) => tool.setAngle(v, 'End')}
      onreset={() => tool.setAngle(0, 'End')}
    />

    <div class="row">
      <button
        class={['text', tool.straightening && 'on']}
        aria-pressed={tool.straightening}
        title="Straighten: draw a line along the horizon"
        disabled={!edit}
        onclick={() => run('crop.straighten')}
      >
        <Icon name="straighten" size={13} /> Straighten
      </button>
    </div>

    <div class="row buttons">
      <button class="text" title={registry.title('photo.rotateLeft')} disabled={!edit} onclick={() => run('photo.rotateLeft')}>
        ↺ 90°
      </button>
      <button class="text" title={registry.title('photo.rotateRight')} disabled={!edit} onclick={() => run('photo.rotateRight')}>
        ↻ 90°
      </button>
      <button class="text" title={registry.title('photo.flipHorizontal')} disabled={!edit} onclick={() => run('photo.flipHorizontal')}>
        Flip H
      </button>
      <button class="text" title={registry.title('photo.flipVertical')} disabled={!edit} onclick={() => run('photo.flipVertical')}>
        Flip V
      </button>
    </div>

    <div class="row size mono" aria-label="Crop size">
      {#if size}<span>W {size.width}</span><span>H {size.height}</span>{/if}
      <span class="overlay-name" title={registry.title('crop.overlay')}>
        Overlay: {tool.overlay === 'none' ? 'None' : tool.overlay === 'golden' ? 'Golden' : tool.overlay === 'grid' ? 'Grid' : 'Thirds'}
      </span>
    </div>

    <div class="row actions">
      <button class="text" disabled={!edit || !edited} onclick={() => tool.reset()}>Reset</button>
      <button class="text close" title={registry.title('crop.done')} onclick={() => tool.close()}>Close</button>
    </div>
  </div>
</section>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 0 12px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 12px 0 26px;
  }

  .label {
    width: 52px;
    flex: none;
    color: var(--text-muted);
    font-size: var(--text-caption);
  }

  .choice {
    position: relative;
    display: flex;
    align-items: center;
  }

  .pill {
    flex: 1;
    min-width: 0;
    height: 24px;
    border: 1px solid var(--field);
    border-radius: var(--r-pill);
    background: var(--trough);
  }

  select {
    appearance: none;
    width: 100%;
    height: 100%;
    padding: 0 26px 0 10px;
    font: inherit;
    color: var(--text-body);
    background: none;
    border: 0;
  }

  .chevron {
    position: absolute;
    right: 10px;
    top: 50%;
    transform: translateY(-65%);
    color: var(--text-muted);
    pointer-events: none;
  }

  .icon,
  .text {
    height: 24px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border-radius: 6px;
    color: var(--text-body);
  }

  .icon {
    width: 26px;
    justify-content: center;
  }

  .text {
    padding: 0 8px;
  }

  .icon:hover:not(:disabled),
  .text:hover:not(:disabled) {
    background: var(--hover);
  }

  .on {
    color: var(--accent-soft);
    background: var(--toggle-on-bg);
  }

  button:disabled {
    color: var(--text-dim);
    cursor: default;
  }

  kbd {
    font-size: var(--text-mono-small);
    color: var(--text-muted);
  }

  .buttons {
    flex-wrap: wrap;
  }

  .size {
    gap: 14px;
    font-size: var(--text-mono-small);
    color: var(--text-secondary);
  }

  .overlay-name {
    margin-left: auto;
    font-family: inherit;
    color: var(--text-muted);
  }

  .actions {
    justify-content: flex-end;
  }

  .close {
    border: 1px solid var(--field);
  }
</style>
