<script lang="ts" module>
  /** Decoded thumbnail pixels as delivered by the engine (RGBA8). */
  export interface RgbaImage {
    width: number;
    height: number;
    data: Uint8ClampedArray<ArrayBuffer>;
  }
</script>

<script lang="ts">
  interface Props {
    /** RGBA pixels, or a decoded JPEG (the engine's thumbnails). */
    image?: RgbaImage | ImageBitmap | null;
    /** Its codes are Display P3, not sRGB. */
    p3?: boolean;
    alt?: string;
  }
  let { image = null, p3 = false, alt = '' }: Props = $props();
  let canvas: HTMLCanvasElement | undefined = $state();

  $effect(() => {
    if (!canvas || !image) return;
    canvas.width = image.width;
    canvas.height = image.height;
    const ctx = canvas.getContext('2d', { colorSpace: p3 ? 'display-p3' : 'srgb' });
    if (!ctx) return;
    if ('data' in image) {
      ctx.putImageData(new ImageData(image.data, image.width, image.height, { colorSpace: p3 ? 'display-p3' : 'srgb' }), 0, 0);
    } else {
      ctx.drawImage(image, 0, 0);
    }
  });
</script>

<div class="thumb" role="img" aria-label={alt}>
  <!-- A new canvas per color space: a context keeps the one it was made with. -->
  {#if image}{#key p3}<canvas bind:this={canvas}></canvas>{/key}{/if}
</div>

<style>
  .thumb {
    width: 100%;
    height: 100%;
    background: var(--photo-backdrop);
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>
