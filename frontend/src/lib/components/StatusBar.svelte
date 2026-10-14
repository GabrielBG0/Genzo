<script lang="ts">
  interface Props {
    /** The mode pill, e.g. the active module. */
    mode: string;
    /** Left-side info, after the pill. The last item is shown brighter. */
    info?: string[];
    /** Right-side facts: color space, bit depth, preview status. */
    facts?: string[];
    /** Software-adapter or other warning (APP-07). */
    notice?: string | null;
  }
  let { mode, info = [], facts = [], notice = null }: Props = $props();
</script>

<footer class="status">
  <span class="pill lens">{mode}</span>
  {#each info as item, i (i)}
    <span class={[i === info.length - 1 && i > 0 && 'bright']}>{item}</span>
  {/each}
  <span class="spacer"></span>
  {#if notice}<span class="notice" role="status">{notice}</span>{/if}
  {#each facts as fact, i (i)}
    <span>{fact}</span>
  {/each}
</footer>

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
  }

  .notice {
    color: var(--rating);
  }

  .spacer {
    flex: 1;
  }
</style>
