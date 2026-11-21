//! Probe 1 of docs/native-surface-spike.md. Run: `cargo run --release`.
//! Env: PROBE_BACKEND=dx12|vulkan (default dx12).
//! The page is opaque except a hole (the "photo rect"); the wgpu clear color,
//! cycling hue, should show through the hole with the HTML button on top.
use std::sync::Arc;
use std::time::Instant;

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

const HTML: &str = r#"<!doctype html><html><body style="margin:0;background:transparent;font:14px sans-serif">
<div id="hole" style="position:fixed;left:200px;top:60px;right:60px;bottom:60px;background:transparent;
 outline:2px solid #fff"></div>
<div style="position:fixed;left:0;right:0;top:0;height:60px;background:#202020"></div>
<div style="position:fixed;left:0;right:0;bottom:0;height:60px;background:#202020"></div>
<div style="position:fixed;right:0;top:0;bottom:0;width:60px;background:#202020"></div>
<div style="position:fixed;left:0;top:0;width:200px;bottom:0;background:#303030;color:#fff;padding:12px">panel (opaque)</div>
<button id="b" style="position:fixed;left:240px;top:100px;padding:10px 16px;background:#e05;color:#fff;border:0">over the hole</button>
<div style="position:fixed;left:240px;bottom:100px;width:300px;height:60px;background:rgba(255,255,255,.35);color:#000;padding:8px">half-transparent over the hole</div>
<script>
 const h = document.getElementById('hole');
 function rect(){const r=h.getBoundingClientRect();const d=devicePixelRatio;
  window.ipc.postMessage(JSON.stringify({rect:[r.left*d,r.top*d,r.width*d,r.height*d],dpr:d}));}
 addEventListener('resize',rect); rect();
 document.getElementById('b').onclick=()=>window.ipc.postMessage('click');
</script></body></html>"#;

fn main() {
    let event_loop = EventLoopBuilder::new().build();
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("surface probe")
            .with_inner_size(tao::dpi::LogicalSize::new(1200.0, 800.0))
            .with_transparent(std::env::var("PROBE_WINDOW_TRANSPARENT").as_deref() != Ok("0"))
            .build(&event_loop)
            .unwrap(),
    );
    let backends = match std::env::var("PROBE_BACKEND").as_deref() {
        Ok("vulkan") => wgpu::Backends::VULKAN,
        _ => wgpu::Backends::DX12,
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let surface = instance.create_surface(window.clone()).unwrap();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .unwrap();
    println!("adapter: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let caps = surface.get_capabilities(&adapter);
    println!("formats {:?} alpha {:?} present {:?}", caps.formats, caps.alpha_modes, caps.present_modes);
    let format = caps.formats[0];
    let size = window.inner_size();
    let mut config = surface
        .get_default_config(&adapter, size.width.max(1), size.height.max(1))
        .unwrap();
    config.format = format;
    surface.configure(&device, &config);

    // PROBE_WEBVIEW=bg0: not `transparent`, only a background color of alpha 0
    // (what a Tauri window with transparent: false and backgroundColor
    // "#00000000" would give the webview).
    let bg0 = std::env::var("PROBE_WEBVIEW").as_deref() == Ok("bg0");
    let _webview = WebViewBuilder::new()
        .with_transparent(!bg0)
        .with_background_color((0, 0, 0, 0))
        .with_html(HTML)
        .with_ipc_handler(|req| println!("ipc: {}", req.body()))
        .build(&*window)
        .unwrap();

    let start = Instant::now();
    let mut frames = 0u32;
    let mut last = Instant::now();
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Poll;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            Event::WindowEvent { event: WindowEvent::Resized(s), .. } if s.width > 0 && s.height > 0 => {
                config.width = s.width;
                config.height = s.height;
                surface.configure(&device, &config);
            }
            Event::MainEventsCleared => {
                let t = start.elapsed().as_secs_f64() * 0.5;
                let c = wgpu::Color {
                    r: 0.5 + 0.5 * (t).sin(),
                    g: 0.5 + 0.5 * (t + 2.1).sin(),
                    b: 0.5 + 0.5 * (t + 4.2).sin(),
                    a: 1.0,
                };
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    other => {
                        println!("surface: {other:?}");
                        surface.configure(&device, &config);
                        return;
                    }
                };
                let view = frame.texture.create_view(&Default::default());
                let mut enc = device.create_command_encoder(&Default::default());
                enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(c), store: wgpu::StoreOp::Store },
                    })],
                    ..Default::default()
                });
                queue.submit([enc.finish()]);
                queue_present(&queue, frame);
                frames += 1;
                if last.elapsed().as_secs() >= 2 {
                    println!("{:.1} fps", frames as f64 / last.elapsed().as_secs_f64());
                    frames = 0;
                    last = Instant::now();
                }
            }
            _ => {}
        }
    });
}

fn queue_present(queue: &wgpu::Queue, frame: wgpu::SurfaceTexture) {
    queue.present(frame);
}
