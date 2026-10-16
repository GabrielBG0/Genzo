/**
 * M0 spike 0.E: slider input to pixels on screen. One request is in flight at
 * a time; newer input while it runs replaces the queued value (latest wins).
 */
export class LatencyProbe<T> {
  private samples: number[] = [];
  private sendToDraw: number[] = [];
  private inFlightSince: number | null = null;
  private sentAt = 0;
  private queued: { value: T; at: number } | null = null;

  constructor(
    private readonly send: (value: T) => Promise<void>,
    private readonly keep = 200,
    private readonly now: () => number = () => performance.now(),
    /** A request with no draw after this long is treated as lost. */
    private readonly lostAfterMs = 2000,
  ) {}

  /** Records an input. Sends it now, or queues it behind the request in flight. */
  input(value: T): void {
    const at = this.now();
    if (this.inFlightSince !== null && at - this.inFlightSince > this.lostAfterMs) {
      // Its frame never arrived (a missed event or a failed fetch). Do not count it.
      this.inFlightSince = null;
      this.queued = null;
    }
    if (this.inFlightSince !== null) {
      // Keep the time of the oldest unserved input: that is what the user waits on.
      this.queued = { value, at: this.queued?.at ?? at };
      return;
    }
    this.dispatch(value, at);
  }

  /** Call when the pixels of the request in flight are on screen. */
  drawn(at: number = this.now()): void {
    if (this.inFlightSince === null) return;
    this.samples.push(at - this.inFlightSince);
    this.sendToDraw.push(at - this.sentAt);
    if (this.samples.length > this.keep) this.samples.shift();
    if (this.sendToDraw.length > this.keep) this.sendToDraw.shift();
    this.inFlightSince = null;
    const next = this.queued;
    this.queued = null;
    if (next) this.dispatch(next.value, next.at);
  }

  private dispatch(value: T, at: number) {
    this.inFlightSince = at;
    this.sentAt = this.now();
    this.send(value).catch(() => {
      this.inFlightSince = null;
    });
  }

  get count(): number {
    return this.samples.length;
  }

  /** A copy of the kept samples, oldest first, in ms. */
  get all(): number[] {
    return [...this.samples];
  }

  /**
   * Like {@link all}, but timed from when each request was sent rather than
   * from the oldest input waiting for it: one request's own cost.
   */
  get sendToDrawMs(): number[] {
    return [...this.sendToDraw];
  }

  percentile(p: number): number | null {
    if (this.samples.length === 0) return null;
    const sorted = [...this.samples].sort((a, b) => a - b);
    return sorted[Math.round((sorted.length - 1) * p)] ?? null;
  }

  reset(): void {
    this.samples = [];
    this.sendToDraw = [];
  }

  /**
   * Drops the request in flight and any queued input without recording a
   * sample. Call before sending a render the probe must not time (the settled
   * render at the end of a drag), whose frame would otherwise close the sample.
   */
  cancel(): void {
    this.inFlightSince = null;
    this.queued = null;
  }
}
