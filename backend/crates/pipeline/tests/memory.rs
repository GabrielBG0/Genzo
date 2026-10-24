//! GPU memory of one 48 MP edit (DEV-01, Tiling) against the budget's caps
//! (`genzo_pipeline::budget`): the pyramid, one tile's scratch and its
//! upload, computed from the allocations and, where the backend reports
//! them (`generate_allocator_report`: DX12 and Vulkan), measured.
//!
//! A second photo of the same size replaces the first without the total
//! growing: the old pyramid is freed before the new one is made. The view's
//! caches with the frame and its staging, the level-0 tile cache and the
//! base layer each stay within their own line.

use genzo_color::encoding::InputEncoding;
use genzo_imgio::raw::{Levels, Rect};
use genzo_pipeline::budget;
use genzo_pipeline::camera_color::{camera_transform, encoded_transform};
use genzo_pipeline::gpu::raw_stages::{RawStages, TileConfig, pyramid_bytes};
use genzo_pipeline::gpu::{AdapterChoice, Gpu, GpuError};
use genzo_pipeline::raw::{Cfa, Linearization, RawSource};

/// One device at a time: WARP crashes with several in one process, and
/// each test measures the process's allocations.
static ONE_DEVICE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn allocated(gpu: &Gpu) -> Option<u64> {
    gpu.device
        .generate_allocator_report()
        .map(|r| r.total_allocated_bytes)
}

/// The live allocations per budget item, by the labels raw_stages gives
/// them: the pyramid, the raw upload tile, the other tile textures.
fn measured(gpu: &Gpu) -> Option<[u64; 3]> {
    let report = gpu.device.generate_allocator_report()?;
    let mut items = [0u64; 3];
    for a in &report.allocations {
        match a.name.as_str() {
            "pyramid" => items[0] += a.size,
            // The staging of the uploads in flight is freed by now; the
            // peak below includes it.
            "tile raw" => items[2] += a.size,
            "tile uniforms" | "tile readback" => {}
            n if n.starts_with("tile ") => items[1] += a.size,
            _ => {}
        }
    }
    Some(items)
}

const MB: f64 = 1e6;

/// An X-Trans sensor's worth of samples (the X-T5's pattern).
fn xtrans_source((w, h): (u32, u32)) -> RawSource {
    let mut src = linear_source((w, h));
    src.samples = 1;
    src.data.truncate((w * h) as usize);
    src.cfa = Some(Cfa::XTrans([
        [1, 1, 0, 1, 1, 2],
        [1, 1, 2, 1, 1, 0],
        [2, 0, 1, 0, 2, 1],
        [1, 1, 2, 1, 1, 0],
        [1, 1, 0, 1, 1, 2],
        [0, 2, 1, 2, 0, 1],
    ]));
    let levels = Levels {
        black: vec![512.0],
        black_repeat: (1, 1),
        black_delta_h: Vec::new(),
        black_delta_v: Vec::new(),
        white: vec![16383],
    };
    src.linearization = Linearization::new(&levels, 1, (w, h)).unwrap();
    src
}

/// A linear DNG's worth of samples, three per pixel: the largest upload.
fn linear_source((w, h): (u32, u32)) -> RawSource {
    let levels = Levels {
        black: vec![512.0; 3],
        black_repeat: (1, 1),
        black_delta_h: Vec::new(),
        black_delta_v: Vec::new(),
        white: vec![16383],
    };
    let data = (0..w * h * 3).map(|i| 600 + (i % 9000) as u16).collect();
    RawSource {
        stored: (w, h),
        samples: 3,
        data,
        cfa: None,
        active: Rect {
            top: 0,
            left: 0,
            bottom: h,
            right: w,
        },
        linearization: Linearization::new(&levels, 3, (w, h)).unwrap(),
        fix_bad_pixels: Vec::new(),
        gain_maps: Vec::new(),
        color: Default::default(),
        encoding: None,
    }
}

/// A non-RAW image: an sRGB JPEG's codes. Its curves add a 786 KB buffer.
fn encoded_source((w, h): (u32, u32)) -> RawSource {
    let rgba: Vec<u16> = (0..w * h * 4).map(|i| (i % 65536) as u16).collect();
    RawSource::from_encoded((w, h), &rgba, &InputEncoding::srgb()).unwrap()
}

#[test]
fn a_48mp_edit_stays_within_the_budget() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let gpu = match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => gpu,
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    println!(
        "adapter: {} ({}); allocator report: {}",
        gpu.info.name,
        gpu.info.backend,
        allocated(&gpu).is_some()
    );

    // The largest upload (linear, three samples per pixel, 48 MP), then the
    // largest working set (X-Trans, Markesteijn's 8 directional images). One
    // full X-Trans tile shows that; 48 MP of it would take minutes on a
    // software adapter.
    for (name, make, size) in [
        (
            "linear",
            linear_source as fn((u32, u32)) -> RawSource,
            (8064, 6048),
        ),
        ("x-trans", xtrans_source, (1200, 1200)),
        ("non-RAW", encoded_source, (8064, 6048)),
    ] {
        println!("{name}:");
        // What is already allocated (the self-test's and wgpu's own, the
        // previous case's freed memory) is not this edit's.
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let before = allocated(&gpu);
        within_budget(&gpu, before, make, size);
    }
}

fn within_budget(
    gpu: &Gpu,
    before: Option<u64>,
    make: fn((u32, u32)) -> RawSource,
    size: (u32, u32),
) {
    let src = make(size);
    let t = match &src.encoding {
        Some(e) => encoded_transform(e),
        None => camera_transform(&src.color, None),
    };
    let mut stages = RawStages::new(gpu, TileConfig::default());
    stages.sample_memory = true;
    let pyramid = stages.render_pyramid(gpu, &src, &t).unwrap().bytes();
    let (scratch, upload) = (stages.scratch_bytes(), stages.upload_bytes());
    println!(
        "computed: pyramid {:.1} MB, tile scratch {:.1} MB, upload {:.1} MB",
        pyramid as f64 / MB,
        scratch as f64 / MB,
        upload as f64 / MB
    );
    assert_eq!(pyramid, pyramid_bytes((size.0 / 2, size.1 / 2)));
    assert!(pyramid <= budget::PYRAMID, "pyramid {pyramid}");
    assert!(scratch <= budget::TILE_SCRATCH, "scratch {scratch}");
    assert!(upload <= budget::UPLOAD, "upload {upload}");
    let caps = budget::PYRAMID + budget::TILE_SCRATCH + budget::UPLOAD;

    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let first = allocated(gpu);
    if let Some(items) = measured(gpu) {
        let names = ["pyramid", "tile scratch", "upload"];
        let caps_each = [budget::PYRAMID, budget::TILE_SCRATCH, budget::UPLOAD];
        for ((name, got), cap) in names.iter().zip(items).zip(caps_each) {
            println!(
                "measured: {name} {:.1} MB (cap {:.0} MB)",
                got as f64 / MB,
                cap as f64 / MB
            );
            assert!(got <= cap, "{name}: {got} bytes, cap {cap}");
        }
    }
    // The peak, while tiles are in flight: their uploads' staging on top.
    if let (Some(before), Some(peak)) = (before, stages.peak_allocated) {
        let used = peak - before;
        println!(
            "measured: peak {:.1} MB during the render (caps {:.0} MB)",
            used as f64 / MB,
            caps as f64 / MB
        );
        assert!(used <= caps + 2_000_000, "peak {used} bytes for {caps}");
    }
    if let (Some(before), Some(after)) = (before, first) {
        let used = after - before;
        println!(
            "measured: {:.1} MB for the edit in all (caps {:.0} MB)",
            used as f64 / MB,
            caps as f64 / MB
        );
        // Everything else the edit holds is small: uniforms, black levels,
        // GainMaps, a non-RAW image's curves.
        assert!(used <= caps + 2_000_000, "{used} bytes for {caps}");
    }

    // A second photo replaces the first.
    let src2 = make(size);
    stages.render_pyramid(gpu, &src2, &t).unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    if let (Some(first), Some(second)) = (first, allocated(gpu)) {
        println!(
            "after a second photo: {:+.1} MB",
            (second as f64 - first as f64) / MB
        );
        assert!(
            second as f64 <= first as f64 + 2.0 * MB,
            "{first} then {second}"
        );
    }
}

/// A prefetched neighbor of 48 MP: its level 1 renders beside the held
/// pyramid within the prefetch line, and the switch to it, level 1
/// uploaded in bands, stays within the pyramid's and the upload's.
#[test]
fn a_prefetch_and_its_switch_stay_within_the_budget() {
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let gpu = match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => gpu,
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    // The largest tile, and one source for both photos to spare CPU memory:
    // the neighbor is the same image at another white.
    let src = linear_source((8064, 6048));
    let held = camera_transform(&src.color, None);
    let other = camera_transform(
        &src.color,
        Some(genzo_color::camera::xy_from_temperature(4300.0, 0.0)),
    );
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &held).unwrap();
    let caps = budget::PYRAMID + budget::TILE_SCRATCH + budget::UPLOAD;
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let before = allocated(&gpu);

    stages.sample_memory = true;
    stages.peak_allocated = None;
    let mut build = stages.start_level1(&gpu, &src, &other);
    let scratch = build.gpu_bytes();
    println!("computed: prefetch {:.1} MB", scratch as f64 / MB);
    assert!(scratch <= budget::PREFETCH, "prefetch {scratch}");
    assert!(
        stages
            .continue_level1(&gpu, &src, &mut build, || false)
            .unwrap()
    );
    if let (Some(before), Some(peak)) = (before, stages.peak_allocated) {
        let more = peak.saturating_sub(before);
        println!(
            "measured: {:.1} MB more at the peak of the neighbor's build",
            more as f64 / MB
        );
        // The uploads' staging, already in the caps, and the prefetch line.
        assert!(
            peak - before <= budget::UPLOAD + budget::PREFETCH,
            "{more} bytes more"
        );
    }
    let level1 = build.finish();
    assert_eq!(level1.bytes(), 4032 * 3024 * 8);

    stages.peak_allocated = None;
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let before = allocated(&gpu);
    stages.upload_pyramid(&gpu, &level1).unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    if let (Some(before), Some(after), Some(peak)) =
        (before, allocated(&gpu), stages.peak_allocated)
    {
        println!(
            "measured: switch {:+.1} MB after, {:+.1} MB at its peak (caps {:.0} MB)",
            (after as f64 - before as f64) / MB,
            (peak as f64 - before as f64) / MB,
            caps as f64 / MB
        );
        // The new pyramid replaces the old; its bands' staging comes and
        // goes within the upload line.
        assert!(
            after as f64 <= before as f64 + 2.0 * MB,
            "{before} then {after}"
        );
        assert!(
            peak <= before + budget::UPLOAD,
            "{before} then a peak of {peak}"
        );
    }
}

/// The view at the largest size DEV-01 budgets (2560 x 1920): its sampled
/// cache within the viewport caches' line, its frame and two staging buffers
/// within the output's, after a 1:1 read of tiles at a pan that clips them.
#[test]
fn the_view_stays_within_its_caps() {
    use genzo_color::output::Target;
    use genzo_pipeline::gpu::view::ViewStages;
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let gpu = match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => gpu,
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    let src = encoded_source((1600, 1200));
    let t = encoded_transform(src.encoding.as_ref().unwrap());
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    let p = stages.render_pyramid(&gpu, &src, &t).unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let before = allocated(&gpu);
    let mut view = ViewStages::new(&gpu);
    view.sample(&gpu, p, 1, (2560, 1920));
    // Stages 6 and 7 make their caches: three viewport caches in all.
    let look = genzo_pipeline::look::Look {
        exposure: 0.5,
        vibrance: 20.0,
        ..genzo_pipeline::look::Look::with_profile(genzo_pipeline::look::Profile::Embedded)
    };
    view.tone(&gpu, &look, None, None);
    view.color(&gpu, &look);
    view.output(&gpu, Target::Srgb, false, false);
    let tiles = genzo_pipeline::gpu::readback::visible_tiles((1, 1), (2560, 1920), (4000, 3000));
    let rects: Vec<_> = tiles.iter().map(|t| t.frame).collect();
    view.read_rects(&gpu, &rects).unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let (cache, frame) = (view.cache_bytes(), view.output_bytes());
    println!(
        "computed: viewport caches {:.1} MB (cap {:.0}), frame and staging {:.1} MB (cap {:.0})",
        cache as f64 / MB,
        budget::VIEWPORT_CACHES as f64 / MB,
        frame as f64 / MB,
        budget::OUTPUT as f64 / MB
    );
    assert!(cache <= budget::VIEWPORT_CACHES && frame <= budget::OUTPUT);
    let full = allocated(&gpu);
    if let (Some(before), Some(after)) = (before, full) {
        let used = after - before;
        println!("measured: {:.1} MB for the view", used as f64 / MB);
        // Plus uniforms and the driver's padding.
        assert!(
            used as f64 <= (cache + frame) as f64 * 1.07 + 2e6,
            "{used} bytes"
        );
    }
    // The half-size drag preview goes through the same frame and staging.
    view.output_half(&gpu, Target::Srgb, false, false);
    view.read_frame(&gpu).unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    assert_eq!(view.output_bytes(), frame);
    if let (Some(full), Some(half)) = (full, allocated(&gpu)) {
        println!(
            "measured: {} bytes more for the half-size preview",
            half - full
        );
        assert!(half - full <= 65_536, "{} bytes", half - full);
    }
}

/// The level-0 tile cache: its atlas within DEV-01's 256 MB line as the
/// driver allocates it, and nothing more while tiles render into it.
#[test]
fn the_level0_cache_stays_within_its_cap() {
    use genzo_pipeline::gpu::level0::Level0Cache;
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let gpu = match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => gpu,
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    let src = encoded_source((2048, 1536));
    let t = encoded_transform(src.encoding.as_ref().unwrap());
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    // The tile set exists before the cache, so what is measured is the
    // atlas and its table.
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let before = allocated(&gpu);
    let mut cache = Level0Cache::new(&gpu);
    let whole = genzo_model::frame::Rect {
        x: 0,
        y: 0,
        width: 2048,
        height: 1536,
    };
    let tiles = Level0Cache::tiles_for(whole, (2048, 1536));
    cache
        .ensure(&gpu, &mut stages, &src, &t, 1, &tiles)
        .unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    println!(
        "computed: atlas {:.1} MB, {} slots (cap {:.0} MB)",
        cache.bytes() as f64 / MB,
        cache.slots(),
        budget::LEVEL0_CACHE as f64 / MB
    );
    assert!(cache.bytes() <= budget::LEVEL0_CACHE);
    if let (Some(before), Some(after)) = (before, allocated(&gpu)) {
        let used = after - before;
        println!("measured: {:.1} MB for the cache", used as f64 / MB);
        assert!(used <= budget::LEVEL0_CACHE, "{used} bytes");
    }
}

/// The base layer of a square image, its largest working size (1536 x
/// 1536): both planes while it builds within DEV-01's 20 MB line, and only
/// the coefficients' plane after; a rebuild frees the old plane first.
#[test]
fn the_base_layer_stays_within_its_cap() {
    use genzo_pipeline::gpu::base::{BaseLayer, peak_bytes};
    let _lock = ONE_DEVICE.lock().unwrap_or_else(|e| e.into_inner());
    let gpu = match Gpu::open(AdapterChoice::from_env()) {
        Ok(gpu) => gpu,
        Err(GpuError::NoAdapter) if std::env::var("GENZO_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipped: no GPU adapter");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    let size = (3072, 3072);
    let src = encoded_source(size);
    let t = encoded_transform(src.encoding.as_ref().unwrap());
    let mut stages = RawStages::new(&gpu, TileConfig::default());
    stages.render_pyramid(&gpu, &src, &t).unwrap();
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    // Measured from after the passes and their uniforms exist: D3D12 gives
    // each small buffer its own 64 KiB allocation (6 x 64 KiB on WARP).
    let mut layer = BaseLayer::new(&gpu);
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let before = allocated(&gpu);
    for build in ["first build", "rebuild"] {
        layer.build(&gpu, stages.pyramid().unwrap(), size, 1);
        assert_eq!(layer.size(), (1536, 1536));
        // Before the GPU finishes, the scratch plane is still held.
        let peak = allocated(&gpu);
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        println!(
            "{build}: computed peak {:.1} MB, held {:.1} MB (cap {:.0} MB)",
            peak_bytes(layer.size()) as f64 / MB,
            layer.bytes() as f64 / MB,
            budget::BASE_LAYER as f64 / MB
        );
        assert!(peak_bytes(layer.size()) <= budget::BASE_LAYER);
        assert_eq!(layer.bytes() * 2, peak_bytes(layer.size()));
        if let (Some(before), Some(peak), Some(after)) = (before, peak, allocated(&gpu)) {
            let (peak, held) = (peak - before, after - before);
            println!(
                "{build}: measured peak {:.1} MB, held {:.1} MB",
                peak as f64 / MB,
                held as f64 / MB
            );
            assert!(peak <= budget::BASE_LAYER, "{peak} bytes");
            assert!(held <= layer.bytes() + 100_000, "{held} bytes");
        }
    }
}
