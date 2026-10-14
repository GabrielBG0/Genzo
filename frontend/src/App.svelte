<script lang="ts">
  import type { EngineInfo } from '$contract/genzo';
  import AppShell from './lib/AppShell.svelte';
  import Gallery from './lib/gallery/Gallery.svelte';
  import { connectEngine, IncompatibleEngineError, type Engine } from './lib/engine';

  let route = $state(location.hash);
  let status = $state<
    { kind: 'connecting' } | { kind: 'ready'; engine: Engine; info: EngineInfo } | { kind: 'failed'; message: string }
  >({ kind: 'connecting' });

  $effect(() => {
    connectEngine()
      .then(async (engine) => {
        const result = await engine.query({ type: 'getEngineInfo' });
        status = { kind: 'ready', engine, info: result.info };
      })
      .catch((e: unknown) => {
        status = {
          kind: 'failed',
          message:
            e instanceof IncompatibleEngineError
              ? `${e.message} Reinstall Genzo so both parts match.`
              : `Could not start the engine: ${e instanceof Error ? e.message : String(e)}`,
        };
      });
  });
</script>

<svelte:window onhashchange={() => (route = location.hash)} />

{#if route === '#/gallery'}
  <Gallery />
{:else if status.kind === 'ready'}
  <AppShell engine={status.engine} info={status.info} />
{:else if status.kind === 'failed'}
  <div class="blocked" role="alert">{status.message}</div>
{/if}

<style>
  .blocked {
    height: 100%;
    display: grid;
    place-items: center;
    padding: 24px;
    color: var(--danger);
    text-align: center;
  }
</style>
