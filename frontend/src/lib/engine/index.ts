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
 * (`?engine=fake` forces the fake; `?exportStep=MS` paces its exports), then runs the contract handshake.
 */
export async function connectEngine(): Promise<Engine> {
  const params = new URLSearchParams(location.search);
  const forceFake = params.get('engine') === 'fake';
  let engine: Engine;
  if (!forceFake && isTauri()) {
    engine = new TauriEngine();
  } else {
    const fake = new FakeEngine();
    // `?exportStep=MS`: the fake's exports take ten steps of MS (e2e tests).
    const step = Number(params.get('exportStep'));
    if (step > 0) fake.exportStepMs = step;
    engine = fake;
  }
  const result = await engine.handshake({ contractVersion: CONTRACT_VERSION });
  if (!result.compatible) {
    throw new IncompatibleEngineError(CONTRACT_VERSION, result.engineContractVersion);
  }
  return engine;
}
