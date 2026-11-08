import type { Attachment } from 'svelte/attachments';

/**
 * The one lens indicator of a segmented group or tab bar (APP-01, the
 * handoff's *Selection indicators*). It slides to the chosen item, left and
 * width over 180 ms with no overshoot, and squeezes to (.95, .9) while an
 * item of the group is pressed.
 *
 * The items are the group's `[data-segment]` elements, in order; `index`
 * names the chosen one, or null for none (the indicator fades out). The
 * first placement fades in without sliding. A menu's highlight (`kind`
 * 'menu', APP-02's Decided style) is the same lens, moving in 55 ms.
 */
export function slidingLens(index: () => number | null, kind: 'segment' | 'menu' = 'segment'): Attachment<HTMLElement> {
  return (group) => {
    const lens = document.createElement('span');
    // A menu's highlight moves faster, and fades in where it first lights.
    lens.className = kind === 'menu' ? 'lens sliding-lens menu-lens' : 'lens sliding-lens';
    lens.setAttribute('aria-hidden', 'true');
    lens.style.opacity = '0';
    group.prepend(lens);
    if (getComputedStyle(group).position === 'static') group.style.position = 'relative';

    let placed = false;
    let chosen: number | null = null;

    function place() {
      const items = group.querySelectorAll<HTMLElement>('[data-segment]');
      const item = chosen === null ? undefined : items[chosen];
      if (!item) {
        lens.style.opacity = '0';
        placed = false;
        return;
      }
      if (!placed) lens.style.transitionProperty = 'opacity';
      lens.style.left = `${item.offsetLeft}px`;
      lens.style.top = `${item.offsetTop}px`;
      lens.style.width = `${item.offsetWidth}px`;
      lens.style.height = `${item.offsetHeight}px`;
      lens.style.borderRadius = getComputedStyle(item).borderRadius;
      lens.style.opacity = '1';
      if (!placed) {
        // Applies the position before the transition comes back.
        void lens.offsetWidth;
        lens.style.transitionProperty = '';
      }
      placed = true;
    }

    $effect(() => {
      chosen = index();
      place();
    });

    const resize = new ResizeObserver(place);
    resize.observe(group);
    for (const item of group.querySelectorAll('[data-segment]')) resize.observe(item);

    const press = (e: PointerEvent) => {
      if (e.target instanceof Element && e.target.closest('[data-segment]:not(:disabled)')) {
        lens.classList.add('pressed');
      }
    };
    const release = () => lens.classList.remove('pressed');
    group.addEventListener('pointerdown', press);
    window.addEventListener('pointerup', release);
    window.addEventListener('pointercancel', release);

    return () => {
      resize.disconnect();
      group.removeEventListener('pointerdown', press);
      window.removeEventListener('pointerup', release);
      window.removeEventListener('pointercancel', release);
      lens.remove();
    };
  };
}
