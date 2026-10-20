//! TEMPORARY: which part of view_tone.wgsl loses the device on CI's WARP
//! (Windows Server 2022), when Windows 11's WARP runs it. Each variant of
//! the shader runs on a fresh device; the test fails with the outcomes so
//! CI shows them. Removed before the PR is ready.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use genzo_pipeline::gpu::{AdapterChoice, Gpu};

const SHADER: &str = include_str!("../src/gpu/shaders/view_tone.wgsl");
const SIZE: u32 = 64;

/// The uniforms as view.rs fills them, with the base curve `flag`.
fn uniforms(flag: u32) -> Vec<u8> {
    let mut f: Vec<f32> = vec![0.2661, 0.6760, 0.0579, 0.0];
    for _ in 0..2 {
        f.extend([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    }
    f.extend([0.3, -0.2, 0.1, 0.3]);
    f.extend([2.474, -5.17, 1.5, 0.4]);
    f.extend([0.005, 0.0025, 1.005, 100.5]);
    let mut b: Vec<u8> = f.iter().flat_map(|v| v.to_le_bytes()).collect();
    for v in [SIZE, SIZE, flag, 0u32] {
        b.extend(v.to_le_bytes());
    }
    b
}

fn run(source: &str, flag: u32) -> String {
    let Ok(gpu) = Gpu::open(AdapterChoice::from_env()) else {
        return "no adapter".into();
    };
    let lost = Arc::new(AtomicBool::new(false));
    let l = lost.clone();
    gpu.device
        .set_device_lost_callback(move |_, _| l.store(true, Ordering::SeqCst));
    gpu.device.on_uncaptured_error(Arc::new(|_| {}));
    let d = &gpu.device;
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let texture = |usage| {
        d.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage,
            view_formats: &[],
        })
    };
    let input = texture(wgpu::TextureUsages::TEXTURE_BINDING);
    let output = texture(wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC);
    let u = uniforms(flag);
    let ubuf = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue.write_buffer(&ubuf, 0, &u);
    let curve = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4098 * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let table: Vec<u8> = (0..4098)
        .flat_map(|i| ((i as f32 / 4096.0).min(1.0)).to_le_bytes())
        .collect();
    gpu.queue.write_buffer(&curve, 0, &table);
    let layout = pipeline.get_bind_group_layout(0);
    let iv = input.create_view(&Default::default());
    let ov = output.create_view(&Default::default());
    let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: ubuf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&iv),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&ov),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: curve.as_entire_binding(),
            },
        ],
    });
    let mut encoder = d.create_command_encoder(&Default::default());
    {
        let mut cp = encoder.begin_compute_pass(&Default::default());
        cp.set_pipeline(&pipeline);
        cp.set_bind_group(0, &group, &[]);
        cp.dispatch_workgroups(SIZE / 16, SIZE / 16, 1);
    }
    gpu.queue.submit([encoder.finish()]);
    let poll = d.poll(wgpu::PollType::wait_indefinitely());
    format!(
        "poll {:?}, device lost {}",
        poll.map(|_| ()),
        lost.load(Ordering::SeqCst)
    )
}

#[test]
fn warp_diagnostics() {
    let main_at = SHADER.find("@compute").unwrap();
    let functions = &SHADER[..main_at];
    // Each variant keeps every binding in use, so the layouts match.
    let keep = "    let keep = curve[0] + f32(tone.size.x) + tone.ramp.x;\n";
    let body = |code: &str| {
        format!(
            "{functions}@compute @workgroup_size(16, 16)\nfn main(@builtin(global_invocation_id) id: vec3<u32>) {{\n    if (id.x >= tone.size.x || id.y >= tone.size.y) {{\n        return;\n    }}\n    let texel = textureLoad(input, vec2<i32>(id.xy), 0);\n    var rgb = texel.rgb;\n{keep}{code}    textureStore(result, vec2<i32>(id.xy), vec4<f32>(rgb + keep * 0.0, texel.a));\n}}\n"
        )
    };
    let variants: Vec<(&str, String, u32)> = vec![
        ("copy", body(""), 0),
        (
            "exposure gain",
            body("    rgb = rgb * exp2(tone.controls.x);\n"),
            0,
        ),
        (
            "log2/exp2 ratio",
            body(
                "    let y = dot(tone.y_row.xyz, rgb);\n    if (y > MIN_LUMINANCE) {\n        let t = log2(y / MIDDLE_GRAY) + tone.controls.x;\n        rgb = rgb * (MIDDLE_GRAY * exp2(t) / y);\n    }\n",
            ),
            0,
        ),
        (
            "whites_blacks",
            body(
                "    let y = dot(tone.y_row.xyz, rgb);\n    if (y > MIN_LUMINANCE) {\n        let t = whites_blacks(log2(y / MIDDLE_GRAY));\n        rgb = rgb * (MIDDLE_GRAY * exp2(t) / y);\n    }\n",
            ),
            0,
        ),
        (
            "contrast",
            body(
                "    let y = dot(tone.y_row.xyz, rgb);\n    if (y > MIN_LUMINANCE) {\n        let t = contrast(log2(y / MIDDLE_GRAY));\n        rgb = rgb * (MIDDLE_GRAY * exp2(t) / y);\n    }\n",
            ),
            0,
        ),
        ("base curve only", body("    rgb = base_curve(rgb);\n"), 1),
        (
            "rgb_tone only",
            body("    rgb = rgb_tone(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)));\n"),
            1,
        ),
        (
            "table only",
            body("    rgb = vec3<f32>(table(rgb.x), table(rgb.y), table(rgb.z));\n"),
            1,
        ),
        (
            "exposure_ramp only",
            body(
                "    rgb = vec3<f32>(exposure_ramp(rgb.x), exposure_ramp(rgb.y), exposure_ramp(rgb.z));\n",
            ),
            1,
        ),
        ("full shader, flag 0", SHADER.to_owned(), 0),
        ("full shader, flag 1", SHADER.to_owned(), 1),
    ];
    let mut log = String::new();
    for (name, source, flag) in &variants {
        log.push_str(&format!("{name}: {}\n", run(source, *flag)));
    }
    panic!("diagnostics (always fails, to show the log):\n{log}");
}
