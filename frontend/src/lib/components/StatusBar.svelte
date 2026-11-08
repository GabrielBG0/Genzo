<script lang="ts">
  import Menu, { type MenuItem } from './Menu.svelte';

  interface Props {
    /** The mode pill, e.g. the active module. */
    mode: string;
    /** Left-side info, after the pill. The last item is shown brighter. */
    info?: string[];
    /** Right-side facts: preview status, color space, bit depth, adapter; each with a tooltip. */
    facts?: { text: string; title?: string }[];
    /** The one message (APP-07): export progress, a refusal, a reason, notes, the adapter notice. */
    notice?: string | null;
    /** The notice's tooltip: its whole text. */
    noticeTitle?: string | undefined;
    /**
     * Set, the notice is a button offering these (an export's progress
     * offers "Cancel export", OUT-01).
     */
    noticeMenu?: MenuItem[] | null;
  }
  let { mode, info = [], facts = [], notice = null, noticeTitle, noticeMenu = null }: Props = $props();

  /** Where the open menu's bottom right corner is, in the window. */
  let menuAt: { right: number; bottom: number } | null = $state(null);
  // The menu goes with its notice.
  $effect(() => {
    if (!noticeMenu) menuAt = null;
  });

  function toggle(e: MouseEvent) {
    if (menuAt) {
      menuAt = null;
      return;
    }
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menuAt = { right: window.innerWidth - r.right, bottom: window.innerHeight - r.top + 4 };
  }

  function onwindowkey(e: KeyboardEvent) {
    if (menuAt && e.key === 'Escape') {
      e.stopPropagation();
      menuAt = null;
    }
  }

  function onwindowdown(e: PointerEvent) {
    if (menuAt && !(e.target instanceof Element && e.target.closest('.notice-menu, .notice'))) menuAt = null;
  }
</script>

<svelte:window onkeydowncapture={onwindowkey} onpointerdown={onwindowdown} />


<footer class="status">
  <span class="pill lens">{mode}</span>
  {#each info as item, i (i)}
    <span class={[i === info.length - 1 && i > 0 && 'bright']}>{item}</span>
  {/each}
  <span class="spacer"></span>
  {#if notice && noticeMenu}
    <span role="status" class="notice-slot">
      <button class="notice" title={noticeTitle} aria-haspopup="menu" aria-expanded={menuAt !== null} onclick={toggle}
        >{notice}</button
      >
    </span>
  {:else if notice}<span class="notice notice-slot" role="status" title={noticeTitle}>{notice}</span>{/if}
  {#each facts as fact, i (i)}
    <span class="fact" title={fact.title}>{fact.text}</span>
  {/each}
</footer>
{#if menuAt && noticeMenu}
  <div class="notice-menu" style:right="{menuAt.right}px" style:bottom="{menuAt.bottom}px">
    <Menu items={noticeMenu} label="Export" width={200} onclose={() => (menuAt = null)} />
  </div>
{/if}

<style>
  .status {
    height: var(--statusbar-h);
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 12px 0 4px;
    background: var(--bg);
    color: var(--text-secondary);
    font-size: var(--text-small);
    white-space: nowrap;
    overflow: hidden;
  }

  .pill {
    height: 18px;
    display: flex;
    align-items: center;
    padding: 0 10px;
    border-radius: var(--r-pill);
    font-weight: 500;
    box-shadow: var(--lens-shadow-flat);
  }

  .bright {
    color: var(--text-body);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .notice-slot {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .fact,
  .pill {
    flex: none;
  }

  .notice {
    color: var(--rating);
  }

  button.notice {
    height: 18px;
    padding: 0 6px;
    border-radius: var(--r-pill);
    border: 1px solid transparent;
    font: inherit;
  }

  button.notice:hover {
    background: var(--hover);
  }

  .notice-menu {
    position: fixed;
    z-index: 20;
  }

  .spacer {
    flex: 1;
  }
</style>
