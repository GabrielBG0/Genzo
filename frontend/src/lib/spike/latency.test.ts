import { describe, expect, it } from 'vitest';
import { LatencyProbe } from './latency';

describe('LatencyProbe', () => {
  it('keeps one request in flight and coalesces newer input', () => {
    let t = 0;
    const sent: number[] = [];
    const probe = new LatencyProbe<number>(
      async (v) => {
        sent.push(v);
      },
      10,
      () => t,
    );
    probe.input(1);
    t = 5;
    probe.input(2);
    t = 6;
    probe.input(3);
    expect(sent).toEqual([1]);
    t = 20;
    probe.drawn();
    // The queued request goes out with the time of the oldest waiting input.
    expect(sent).toEqual([1, 3]);
    t = 45;
    probe.drawn();
    expect(probe.count).toBe(2);
    expect(probe.percentile(0)).toBe(20);
    expect(probe.percentile(1)).toBe(40);
    // The second request went out at 20, so it cost 25 on its own.
    expect(probe.sendToDrawMs).toEqual([20, 25]);
  });

  it('ignores draws with nothing in flight', () => {
    const probe = new LatencyProbe<number>(async () => {});
    probe.drawn();
    expect(probe.count).toBe(0);
    expect(probe.percentile(0.95)).toBeNull();
  });

  it('gives up on a request whose frame never arrives', () => {
    let t = 0;
    const sent: number[] = [];
    const probe = new LatencyProbe<number>(async (v) => void sent.push(v), 10, () => t, 100);
    probe.input(1);
    t = 500;
    probe.input(2);
    expect(sent).toEqual([1, 2]);
    probe.drawn();
    expect(probe.percentile(0)).toBe(0);
  });

  it('cancel drops the request in flight without a sample', () => {
    let t = 0;
    const sent: number[] = [];
    const probe = new LatencyProbe<number>(async (v) => void sent.push(v), 10, () => t);
    probe.input(1);
    probe.input(2);
    probe.cancel();
    t = 30;
    probe.drawn();
    expect(probe.count).toBe(0);
    // Nothing queued survives the cancel.
    expect(sent).toEqual([1]);
  });
});
