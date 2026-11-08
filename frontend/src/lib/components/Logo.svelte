<script lang="ts" module>
  /**
   * The Genzo glyph (`docs/UI/logo/svg/genzo-glyph-currentcolor.svg`, the
   * brand's small mark): one 35 mm frame, its perforations and its image
   * window cut out. The SVG draws it with a mask; here it is one even-odd
   * path in the same coordinates, so instances share no mask id.
   */
  type R = [x: number, y: number, w: number, h: number, r: number];

  /** A rounded rectangle as a closed subpath. */
  function rect([x, y, w, h, r]: R): string {
    return (
      `M${x + r},${y}h${w - 2 * r}a${r},${r} 0 0 1 ${r},${r}v${h - 2 * r}a${r},${r} 0 0 1 ${-r},${r}` +
      `h${-(w - 2 * r)}a${r},${r} 0 0 1 ${-r},${-r}v${-(h - 2 * r)}a${r},${r} 0 0 1 ${r},${-r}z`
    );
  }

  const PERFORATIONS = [19.15, 27.46, 35.77, 44.08, 52.39, 60.7, 69.01, 77.32].flatMap((x): R[] => [
    [x, 21.7, 3.5, 4.9, 0.9],
    [x, 73.4, 3.5, 4.9, 0.9],
  ]);
  const FRAME: R = [16.75, 19.5, 66.5, 61, 3];
  const WINDOW: R = [18.5, 29, 63, 42, 1.2];

  export const GLYPH_PATH = [FRAME, ...PERFORATIONS, WINDOW].map(rect).join('');
</script>

<script lang="ts">
  interface Props {
    /** Width in CSS pixels; the glyph is 72 × 66. */
    size?: number;
  }
  let { size = 18 }: Props = $props();
</script>

<!-- The color comes from the text around it (brand.md: the top bar's glyph takes currentColor). -->
<svg
  class="logo"
  width={size}
  height={(size * 66) / 72}
  viewBox="14 17 72 66"
  aria-hidden="true"
  fill="currentColor"
  fill-rule="evenodd"
>
  <path d={GLYPH_PATH} />
</svg>

<style>
  .logo {
    display: block;
    flex: none;
  }
</style>
