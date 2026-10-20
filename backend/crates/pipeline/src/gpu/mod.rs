//! The GPU pipeline (M0 spike 0.D).
//!
//! One render thread owns the wgpu device ([`thread::RenderThread`]); other
//! threads send it work over a channel. Rendering is offscreen: frames are
//! read back and leave through the tile protocol, so no surface (and no
//! unsafe code) is needed.

pub mod base;
pub mod level0;
pub mod raw_stages;
pub mod spike;
pub mod synthetic;
pub mod thread;
pub mod view;

use std::fmt;

/// Which adapter to ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterChoice {
    /// A hardware adapter; the software one if there is none, it is
    /// blocklisted, or it fails the self-test.
    Auto,
    /// The software adapter only: WARP on Windows, lavapipe or llvmpipe on
    /// Linux. The golden-image reference device.
    Software,
}

impl AdapterChoice {
    /// `GENZO_ADAPTER=software` forces the software adapter; anything else is
    /// [`AdapterChoice::Auto`].
    pub fn from_env() -> Self {
        match std::env::var("GENZO_ADAPTER").as_deref() {
            Ok("software") => Self::Software,
            _ => Self::Auto,
        }
    }
}

/// Facts about the adapter the render thread got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub software: bool,
    /// Why the engine is not on a hardware adapter, if it is not.
    pub fallback_reason: Option<String>,
    /// Whether GPU timestamps are available, so stage times are GPU time and
    /// not wall-clock time.
    pub timestamps: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    NoAdapter,
    Device(String),
    SelfTest(String),
    Readback(String),
    NoImage,
    ThreadGone,
    /// The GPU ran out of memory. The render thread keeps running.
    OutOfMemory,
    /// The device reported an error during a request (wgpu validation or
    /// internal error). The render thread keeps running.
    Request(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no GPU adapter, not even a software one"),
            Self::Device(e) => write!(f, "could not create the GPU device: {e}"),
            Self::SelfTest(e) => write!(f, "GPU self-test failed: {e}"),
            Self::Readback(e) => write!(f, "GPU readback failed: {e}"),
            Self::NoImage => write!(f, "no image loaded"),
            Self::ThreadGone => write!(f, "the render thread stopped"),
            Self::OutOfMemory => write!(f, "the GPU ran out of memory"),
            Self::Request(e) => write!(f, "GPU error: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// Drivers known to render the pipeline wrongly: (PCI vendor id, a substring
/// of the driver name or version, reason). Matching adapters are skipped in
/// favor of the software adapter. Empty until a bad driver is found.
const BLOCKLIST: &[(u32, &str, &str)] = &[];

fn blocked(info: &wgpu::AdapterInfo) -> Option<&'static str> {
    blocked_in(
        BLOCKLIST,
        info.vendor,
        &format!("{} {}", info.driver, info.driver_info),
    )
}

fn blocked_in(
    list: &[(u32, &str, &'static str)],
    vendor: u32,
    driver: &str,
) -> Option<&'static str> {
    list.iter()
        .find(|(v, d, _)| *v == vendor && driver.contains(d))
        .map(|(_, _, reason)| *reason)
}

/// An open device and its queue.
#[derive(Debug)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: GpuInfo,
}

impl Gpu {
    /// Opens an adapter and runs the self-test on it. With
    /// [`AdapterChoice::Auto`], a failing hardware adapter falls back to the
    /// software one.
    pub fn open(choice: AdapterChoice) -> Result<Self, GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        if choice == AdapterChoice::Auto {
            let hardware = match Self::try_open(&instance, false, None) {
                Ok(gpu) => return Ok(gpu),
                Err(e) => e,
            };
            return match Self::try_open(&instance, true, Some(hardware.to_string())) {
                Ok(gpu) => Ok(gpu),
                // No software adapter either (macOS has none): report why the
                // hardware one failed, which is the useful part.
                Err(GpuError::NoAdapter) => Err(hardware),
                Err(software) => Err(GpuError::Device(format!(
                    "hardware: {hardware}; software: {software}"
                ))),
            };
        }
        Self::try_open(&instance, true, Some("software adapter requested".into()))
    }

    fn try_open(
        instance: &wgpu::Instance,
        software: bool,
        fallback_reason: Option<String>,
    ) -> Result<Self, GpuError> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: software,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|_| GpuError::NoAdapter)?;
        let raw = adapter.get_info();
        let is_software = raw.device_type == wgpu::DeviceType::Cpu;
        if !software && is_software {
            // Asked for hardware and got a software adapter: say why.
            return Err(GpuError::Device(
                "only a software adapter is available".into(),
            ));
        }
        if let Some(reason) = blocked(&raw) {
            return Err(GpuError::Device(format!("driver blocklisted: {reason}")));
        }
        let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let required_features = if timestamps {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("genzo"),
            required_features,
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        let gpu = Self {
            device,
            queue,
            info: GpuInfo {
                name: raw.name.clone(),
                backend: format!("{:?}", raw.backend),
                software: is_software,
                fallback_reason,
                timestamps,
            },
        };
        spike::self_test(&gpu)?;
        Ok(gpu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocklist_matches_vendor_and_driver() {
        let list = [(0x10de, "bad 1.0", "renders black")];
        assert_eq!(
            blocked_in(&list, 0x10de, "nvidia bad 1.0"),
            Some("renders black")
        );
        assert_eq!(blocked_in(&list, 0x10de, "good 2.0"), None);
        assert_eq!(blocked_in(&list, 0x1002, "bad 1.0"), None);
    }
}
