import { describe, expect, it } from 'vitest';
import { chooseNotice, type NoticeParts } from './status';

const all: NoticeParts = {
  exporting: 'Exporting a.jpg, 40 %',
  refusal: "Can't set white balance here: the area is clipped",
  reason: 'Nikon High Efficiency RAW is not supported yet',
  notes: ['Lens corrections from the file are not applied yet', "The file's tone curve can't be read; using the default"],
  adapter: 'No GPU: rendering on the CPU',
};

describe('chooseNotice (APP-07)', () => {
  it('shows the most urgent, and the next as each goes', () => {
    let p = { ...all };
    expect(chooseNotice(p)?.text).toBe(all.exporting);
    p = { ...p, exporting: null };
    expect(chooseNotice(p)?.text).toBe(all.refusal);
    p = { ...p, refusal: null };
    expect(chooseNotice(p)?.text).toBe(all.reason);
    p = { ...p, reason: null };
    expect(chooseNotice(p)).toEqual({
      text: 'Lens corrections from the file are not applied yet +1',
      title: all.notes.join('\n'),
    });
    p = { ...p, notes: [] };
    expect(chooseNotice(p)?.text).toBe(all.adapter);
    p = { ...p, adapter: null };
    expect(chooseNotice(p)).toBeNull();
  });

  it('a single note shows without a count', () => {
    expect(chooseNotice({ ...all, exporting: null, refusal: null, reason: null, notes: ['One'] })?.text).toBe('One');
  });
});
