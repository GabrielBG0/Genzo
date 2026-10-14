import { CONTRACT_VERSION } from '$contract/genzo';
import { isTauri, TauriEngine } from '../transport/tauri';
import { FakeEngine } from './fake';
import type { Engine } from './types';

export type { Engine } from './types';

export class IncompatibleEngineError extends Error {
  constructor(
    readonly appVersion: string,
    readonly engineVersion: string,
  ) {
    super(`Contract mismatch: the app speaks ${appVersion}, the engine ${engineVersion}.`);
  }
}

/**
 * Picks the real engine inside Tauri and the fake one in a plain browser
 * (`?engine=fake` forces the fake), then runs the contract handshake.
 */
export async function connectEngine(): Promise<Engine> {
  const forceFake = new URLSearchParams(location.search).get('engine') === 'fake';
  const engine: Engine = !forceFake && isTauri() ? new TauriEngine() : new FakeEngine();
  const result = await engine.handshake({ contractVersion: CONTRACT_VERSION });
  if (!result.compatible) {
    throw new IncompatibleEngineError(CONTRACT_VERSION, result.engineContractVersion);
  }
  return engine;
}
