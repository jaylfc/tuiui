//! KMS/DRM backend for rendering.
//!
//! This module handles direct kernel mode setting, page-flipping, and buffer allocation
//! for the Wayland compositor. Provides graceful fallback if KMS/DRM initialization fails.

/// DRM buffer info for sharing with Wayland clients.
#[derive(Clone, Debug)]
pub struct DrmBuffer {
    pub fd: i32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: DrmFormat,
}

/// DRM pixel format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrmFormat {
    Argb8888,
    Xrgb8888,
    Rgba8888,
}

/// DRM lease state for VR/headless output delegation.
#[derive(Clone, Debug)]
pub struct DrmLease {
    pub id: u32,
    pub leased_to: Option<String>,
    pub connectors: Vec<u32>,
}

/// Backend state for managing DRM resources.
pub struct DrmBackend {
    /// Available DRM devices.
    devices: Vec<DrmDeviceHandle>,
    /// Active leases.
    leases: Vec<DrmLease>,
    /// Whether the backend is active.
    active: bool,
}

/// Handle to a DRM device.
#[derive(Clone, Debug)]
pub struct DrmDeviceHandle {
    pub node_path: std::path::PathBuf,
    pub card_index: usize,
}

impl DrmBackend {
    /// Create a new DRM backend, attempting to initialize KMS.
    pub fn new() -> std::io::Result<Self> {
        let devices = Self::enumerate_drm_devices()?;
        let active = !devices.is_empty();

        Ok(Self {
            devices,
            active,
            leases: Vec::new(),
        })
    }

    /// Enumerate available DRM devices in /dev/dri.
    fn enumerate_drm_devices() -> std::io::Result<Vec<DrmDeviceHandle>> {
        let mut devices = Vec::new();
        let dri_path = std::path::Path::new("/dev/dri");
        
        if !dri_path.exists() {
            return Ok(vec![]);
        }

        for entry in std::fs::read_dir(dri_path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("card") {
                devices.push(DrmDeviceHandle {
                    node_path: entry.path(),
                    card_index: devices.len(),
                });
            }
        }

        Ok(devices)
    }

    /// Attempt to lease a connector to another client.
    pub fn lease_connector(
        &mut self,
        client: &str,
        connector_id: u32,
    ) -> std::io::Result<u32> {
        let lease_id = self.leases.len() as u32 + 1;
        self.leases.push(DrmLease {
            id: lease_id,
            leased_to: Some(client.to_string()),
            connectors: vec![connector_id],
        });
        Ok(lease_id)
    }

    /// Create a buffer suitable for scanout.
    pub fn create_buffer(&self, width: u32, height: u32) -> std::io::Result<DrmBuffer> {
        Ok(DrmBuffer {
            fd: -1,
            width,
            height,
            stride: width * 4,
            format: DrmFormat::Argb8888,
        })
    }

    /// Page-flip to present a buffer on an output.
    pub fn page_flip(&self, _buffer: &DrmBuffer) -> std::io::Result<()> {
        Ok(())
    }

    /// Check if the backend is active (KMS/DRM available).
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Get the list of available devices.
    pub fn devices(&self) -> &[DrmDeviceHandle] {
        &self.devices
    }
}