<script lang="ts">
  import { CONTRACT_VERSION, type EngineInfo } from '$contract/genzo';
  import { APP_VERSION } from '../version';
  import Logo from './Logo.svelte';

  interface Props {
    info: EngineInfo;
    onclose: () => void;
  }
  let { info, onclose }: Props = $props();
  let dialog: HTMLElement | undefined = $state();

  // Keys go to the dialog while it is open.
  $effect(() => dialog?.focus());

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' || e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }
</script>

<div class="scrim" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="about-title"
    tabindex="-1"
    bind:this={dialog}
    {onkeydown}
  >
    <h2 id="about-title"><span class="mark"><Logo size={22} /></span>Genzo</h2>
    <dl>
      <dt>Version</dt>
      <dd class="mono">{APP_VERSION}</dd>
      <dt>Engine contract</dt>
      <dd class="mono">{CONTRACT_VERSION}</dd>
      <dt>GPU</dt>
      <dd>{info.adapter.name}</dd>
      {#if info.adapter.kind !== 'hardware' && info.adapter.fallbackReason}
        <dt></dt>
        <dd class="notice">{info.adapter.fallbackReason}</dd>
      {/if}
    </dl>
    <p>Free software under the GNU GPL v3.</p>
    <div class="actions">
      <button class="lens" onclick={onclose}>Close</button>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: var(--scrim);
    z-index: 10;
  }

  .dialog {
    width: min(360px, calc(100% - 48px));
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px;
    border-radius: var(--r-dialog);
    background: var(--dialog);
    border: var(--pane-border);
    box-shadow: var(--pane-shadow);
    color: var(--text-body);
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-strong);
  }

  /* The glyph takes the text's color; the frame is never recolored (brand.md). */
  .mark {
    color: var(--text-strong);
  }

  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 14px;
    margin: 0;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
  }

  .notice {
    color: var(--rating);
  }

  p {
    margin: 0;
    color: var(--text-secondary);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }

  .actions button {
    padding: 5px 14px;
    border-radius: 6px;
  }
</style>
