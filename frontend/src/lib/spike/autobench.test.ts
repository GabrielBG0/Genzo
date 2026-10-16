import { describe, expect, it } from 'vitest';
import type { EngineInfo } from '$contract/genzo';
import { CASES, PROTOCOL, percentile, runAutobench, type AutobenchDriver, type AutobenchSlider } from './autobench';

const INFO: EngineInfo = {
  engineVersion: '0.0.0-test',
  contractVersion: '0.0.0',
  adapter: { kind: 'none', name: 'test', backend: 'none', fallbackReason: null },
  autobench: true,
};

const FAST = { ...PROTOCOL, drags: 2, steps: 4, stepMs: 0, pauseMs: 0, loadWaitMs: 0, prepareTimeoutMs: 50 };

describe('runAutobench', () => {
  it('drags each slider end to end and back, settling after every drag', async () => {
    const log: string[] = [];
    let samples: number[] = [];
    const driver: AutobenchDriver = {
      prepare: async (c) => void log.push(`prepare ${c.slider}`),
      input: (slider: AutobenchSlider, t: number) => {
        log.push(`${slider} ${t}`);
        samples.push(10 + t);
      },
      settle: () => void log.push('settle'),
      reset: () => {
        samples = [];
      },
      samples: () => samples,
      engineMs: () => [4, 6],
      transport: () => ({ executeMs: [9], fetchMs: [2, 3], fetchBytes: [2048, 4096] }),
    };
    const report = await runAutobench(driver, INFO, CASES.slice(0, 1), FAST);
    expect(log).toEqual([
      'prepare Exposure',
      ...[0, 0.25, 0.5, 0.75, 1].map((t) => `Exposure ${t}`),
      'settle',
      ...[1, 0.75, 0.5, 0.25, 0].map((t) => `Exposure ${t}`),
      'settle',
    ]);
    const [c] = report.cases;
    expect(c?.n).toBe(10);
    expect(c?.p95).toBe(11);
    expect(c?.engine).toEqual({ frames: 2, p50: 6, p95: 6 });
    expect(c?.transport.fetchKB).toEqual({ n: 2, p50: 4, p95: 4 });
    expect(report.engine).toBe(INFO);
  });

  it('records a failing case and runs the next', async () => {
    const driver: AutobenchDriver = {
      prepare: (c) => (c.source.includes('48') ? Promise.reject(new Error('out of memory')) : Promise.resolve()),
      input: () => {},
      settle: () => {},
      reset: () => {},
      samples: () => [20],
      engineMs: () => [],
      transport: () => ({ executeMs: [], fetchMs: [], fetchBytes: [] }),
    };
    const report = await runAutobench(driver, INFO, CASES.filter((_, i) => i === 3 || i === 0), FAST);
    expect(report.cases.map((c) => c.error ?? c.p95)).toEqual([20, 'out of memory']);
  });

  it('times out a case whose image never loads', async () => {
    const driver: AutobenchDriver = {
      prepare: () => new Promise(() => {}),
      input: () => {},
      settle: () => {},
      reset: () => {},
      samples: () => [],
      engineMs: () => [],
      transport: () => ({ executeMs: [], fetchMs: [], fetchBytes: [] }),
    };
    const report = await runAutobench(driver, INFO, CASES.slice(0, 1), FAST);
    expect(report.cases[0]?.error).toMatch(/took over 50 ms/);
  });
});

describe('percentile', () => {
  it('matches the probe', () => {
    expect(percentile([], 0.95)).toBeNull();
    expect(percentile([5, 1, 3], 0.5)).toBe(3);
  });
});
