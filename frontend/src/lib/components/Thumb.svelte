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
    image?: RgbaImage | null;
    alt?: string;
  }
  let { image = null, alt = '' }: Props = $props();
  let canvas: HTMLCanvasElement | undefined = $state();

  $effect(() => {
    if (!canvas || !image) return;
    canvas.width = image.width;
    canvas.height = image.height;
    canvas.getContext('2d')?.putImageData(new ImageData(image.data, image.width, image.height), 0, 0);
  });
</script>

<div class="thumb" role="img" aria-label={alt}>
  {#if image}<canvas bind:this={canvas}></canvas>{/if}
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
