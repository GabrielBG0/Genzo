/** The status bar's notice (APP-07): one message, the most urgent first. */
export interface NoticeParts {
  /** An export's progress (OUT-01). */
  exporting: string | null;
  /** A refusal or an error just now. */
  refusal: string | null;
  /** Why the photo shows only its embedded preview. */
  reason: string | null;
  /** What the engine could not do as the photo's file asks. */
  notes: string[];
  /** Why the GPU is not used. */
  adapter: string | null;
}

export interface Notice {
  text: string;
  /** The tooltip: the whole text, every note. */
  title: string;
}

export function chooseNotice(p: NoticeParts): Notice | null {
  for (const text of [p.exporting, p.refusal, p.reason]) {
    if (text) return { text, title: text };
  }
  const [first, ...rest] = p.notes;
  if (first) return { text: rest.length > 0 ? `${first} +${rest.length}` : first, title: p.notes.join('\n') };
  return p.adapter ? { text: p.adapter, title: p.adapter } : null;
}
