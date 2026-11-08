/**
 * Fades a closing element out over `ms` (a menu's 90 ms close, APP-02's
 * Decided style) without keeping it: a copy takes its place, inert, above
 * the page, and goes when the fade ends. So a menu opened again at once is
 * a new one, focused as usual, and nothing under the copy loses a click.
 */
export function fadeOut(el: HTMLElement | undefined, ms = 90): void {
  if (!el?.isConnected || typeof el.animate !== 'function') return;
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const r = el.getBoundingClientRect();
  const ghost = el.cloneNode(true) as HTMLElement;
  ghost.setAttribute('aria-hidden', 'true');
  ghost.inert = true;
  Object.assign(ghost.style, {
    position: 'fixed',
    left: `${r.left}px`,
    top: `${r.top}px`,
    width: `${r.width}px`,
    height: `${r.height}px`,
    margin: '0',
    zIndex: '40',
    pointerEvents: 'none',
    animation: 'none',
  });
  document.body.append(ghost);
  ghost.animate([{ opacity: 1 }, { opacity: 0 }], { duration: ms, easing: 'ease-out' }).onfinish = () => ghost.remove();
}
