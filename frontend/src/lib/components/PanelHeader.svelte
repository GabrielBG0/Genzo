<script lang="ts">
  import Icon from './Icon.svelte';

  interface Props {
    heading: string;
    meta?: string;
    /** Something in the panel differs from its default (DEV-13). */
    edited?: boolean;
    open?: boolean;
    /** Its side is in solo mode (APP-08): the chevron shows it. */
    solo?: boolean;
    /** A click, with its modifiers: Alt for solo mode, Ctrl for all (APP-08). */
    ontoggle?: (e: MouseEvent) => void;
  }
  let { heading, meta = '', edited = false, open = true, solo = false, ontoggle }: Props = $props();
</script>

<button class="header" aria-expanded={open} onclick={(e) => ontoggle?.(e)}>
  <span class={['chevron', solo && 'solo']} title={solo ? 'Solo mode: one panel open at a time (Alt-click)' : undefined}>
    <Icon name={open ? 'chevronDown' : 'chevronRight'} size={14} stroke={2} />
  </span>
  <span class="heading">{heading}</span>
  {#if edited}<span class="dot" title="Edited" aria-label="Edited">●</span>{/if}
  <span class="spacer"></span>
  <span class="meta">{meta}</span>
</button>

<style>
  .header {
    width: 100%;
    height: var(--row-header);
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 12px 0 8px;
    border-top: 1px solid var(--divider);
    color: var(--text-body);
    font-size: var(--text-caption);
    font-weight: 600;
    letter-spacing: var(--header-tracking);
    text-transform: uppercase;
  }

  .chevron {
    display: flex;
  }

  .chevron.solo {
    color: var(--accent-soft);
  }

  .dot {
    color: var(--accent);
    font-size: 9px;
  }

  .spacer {
    flex: 1;
  }

  .meta {
    font-weight: 400;
    letter-spacing: 0;
    text-transform: none;
    color: var(--text-muted);
  }
</style>
