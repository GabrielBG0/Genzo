import { getContext, setContext } from 'svelte';
import { createRegistry } from './commands';
import type { ShortcutRegistry } from './registry';

const KEY = Symbol('shortcuts');

export function provideRegistry(registry: ShortcutRegistry = createRegistry()): ShortcutRegistry {
  setContext(KEY, registry);
  return registry;
}

/** The registry of the enclosing app, or a fresh default one (gallery, tests). */
export function useRegistry(): ShortcutRegistry {
  return getContext<ShortcutRegistry | undefined>(KEY) ?? createRegistry();
}
