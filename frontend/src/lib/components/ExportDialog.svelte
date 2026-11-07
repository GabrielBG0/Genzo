<script lang="ts">
  import { untrack } from 'svelte';
  import { MIN_LONG_EDGE, type Exports, nameOf, withJpg } from '../develop/export.svelte';
  import type { Engine } from '../engine/types';

  interface Props {
    engine: Engine;
    exports: Exports;
    /** The photo open in Develop. */
    photo: number;
    fileName: string;
    /** The folder the photo was opened from. */
    photoFolder: string;
    onclose: () => void;
  }
  let { engine, exports, photo, fileName, photoFolder, onclose }: Props = $props();

  // The destination, and whether it is the save dialog's own path (which
  // asked before replacing a file there).
  let path = $state(untrack(() => exports.suggested(fileName, photoFolder)));
  let fromDialog = $state(false);
  let error: string | null = $state(null);
  let confirming = $state(false);
  let working = $state(false);
  // The session's values: the dialog edits them in place.
  const settings = untrack(() => exports.settings);
  let dialog: HTMLElement | undefined = $state();

  // Keys go to the dialog while it is open.
  $effect(() => dialog?.focus());

  const longEdgeValid = $derived(
    settings.size === 'full' || (Number.isInteger(settings.longEdge) && settings.longEdge >= MIN_LONG_EDGE),
  );

  async function choose() {
    const picked = await engine.chooseSavePath(path);
    if (picked === null) return;
    const normalized = withJpg(picked);
    // A name the user typed without .jpg is not the path the dialog checked.
    fromDialog = normalized === picked;
    path = normalized;
    error = null;
    confirming = false;
  }

  async function run(replace: boolean) {
    if (working || !longEdgeValid) return;
    working = true;
    const started = await exports.start(photo, path, replace);
    working = false;
    if (started.ok) {
      onclose();
    } else if (started.exists) {
      confirming = true;
      error = null;
    } else {
      error = started.reason;
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (confirming) confirming = false;
      else onclose();
    } else if (e.key === 'Enter' && !(e.target instanceof HTMLButtonElement)) {
      e.preventDefault();
      e.stopPropagation();
      void run(confirming || fromDialog);
    }
  }
</script>

<div class="scrim" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="export-title"
    tabindex="-1"
    bind:this={dialog}
    {onkeydown}
  >
    <h2 id="export-title">Export</h2>

    {#if exports.busy}
      <p class="busy" role="status">
        {exports.status}
        <button class="link" onclick={() => void exports.cancel()}>Cancel export</button>
      </p>
    {/if}

    <section aria-label="Location">
      <span class="label">File</span>
      <div class="file">
        <span class="path mono" title={path}>{path}</span>
        <button class="lens" onclick={() => void choose()}>Choose…</button>
      </div>
    </section>

    <section aria-label="File settings">
      <span class="label">Format</span><span>JPEG</span>
      <label class="label" for="export-quality">Quality</label>
      <span class="quality">
        <input id="export-quality" type="range" min="1" max="100" bind:value={settings.quality} />
        <span class="mono">{settings.quality}</span>
      </span>
      <span class="label">Color space</span><span>sRGB</span>
    </section>

    <section aria-label="Image sizing">
      <span class="label">Size</span>
      <span class="sizes">
        <label><input type="radio" name="export-size" value="full" bind:group={settings.size} /> Full size</label>
        <label>
          <input type="radio" name="export-size" value="long" bind:group={settings.size} /> Long edge
          <input
            class="edge mono"
            type="number"
            min={MIN_LONG_EDGE}
            aria-label="Long edge in pixels"
            bind:value={settings.longEdge}
            disabled={settings.size !== 'long'}
          /> px
        </label>
      </span>
    </section>

    <p class="note">All EXIF metadata, including location. XMP and IPTC are not included.</p>

    {#if !longEdgeValid}
      <p class="error" role="alert">The long edge is {MIN_LONG_EDGE} px at least.</p>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    {#if confirming}
      <div class="actions" role="group" aria-label="Replace">
        <span class="ask">{nameOf(path)} already exists. Replace it?</span>
        <button class="lens" onclick={() => (confirming = false)}>Cancel</button>
        <button class="lens primary" onclick={() => void run(true)}>Replace</button>
      </div>
    {:else}
      <div class="actions">
        <button class="lens" onclick={onclose}>Cancel</button>
        <button class="lens primary" disabled={working || !longEdgeValid} onclick={() => void run(fromDialog)}>
          {exports.busy ? 'Export after this one' : 'Export'}
        </button>
      </div>
    {/if}
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
    width: min(460px, calc(100% - 48px));
    max-height: calc(100% - 48px);
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px 20px;
    border-radius: var(--r-dialog);
    background: var(--dialog);
    border: var(--pane-border);
    box-shadow: var(--pane-shadow);
    color: var(--text-body);
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-strong);
  }

  section {
    display: grid;
    grid-template-columns: 96px 1fr;
    align-items: center;
    gap: 8px 12px;
  }

  .label {
    color: var(--text-secondary);
  }

  .file {
    display: flex;
    gap: 8px;
    min-width: 0;
    align-items: center;
  }

  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-small);
    padding: 5px 8px;
    border-radius: 6px;
    background: var(--field);
  }

  .quality {
    display: flex;
    gap: 10px;
    align-items: center;
  }

  .quality input {
    flex: 1;
    accent-color: var(--accent);
  }

  .sizes {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .sizes input[type='radio'] {
    accent-color: var(--accent);
  }

  .edge {
    width: 72px;
    padding: 3px 6px;
    border-radius: 6px;
    border: none;
    background: var(--field);
    color: var(--text-body);
  }

  .note {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--text-small);
  }

  .error {
    margin: 0;
    color: var(--danger);
  }

  .busy {
    margin: 0;
    color: var(--text-secondary);
    display: flex;
    gap: 10px;
    align-items: center;
  }

  .link {
    color: var(--accent-soft);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
  }

  .ask {
    flex: 1;
    color: var(--text-strong);
  }

  .actions button,
  .file button {
    padding: 5px 14px;
    border-radius: 6px;
  }

  .primary {
    color: var(--text-strong);
    font-weight: 600;
  }

  button:disabled {
    opacity: 0.5;
  }
</style>
