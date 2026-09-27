use nokhwa::pixel_format::YuyvFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
    Resolution,
};
use nokhwa::Camera;

use crate::error::{Error, Result};
use crate::frame::LiveFrame;

/// Information about a detected video camera.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub is_likely_uti: bool,
}

/// Detects all video capture devices on the system.
pub fn query_devices() -> Result<Vec<DeviceInfo>> {
    let cameras = nokhwa::query(ApiBackend::Auto)?;
    let mut devices = Vec::new();

    for cam in cameras {
        let index = match cam.index() {
            CameraIndex::Index(i) => *i,
            CameraIndex::String(s) => s.parse::<u32>().unwrap_or(0),
        };

        let name = cam.human_name();
        let desc = cam.description().to_string();
        let combined = format!("{} {}", name, desc).to_lowercase();

        // UTi-260B identifies as "UVC Camera" or VID 1d6b, PID 0102
        let is_likely_uti = combined.contains("uvc camera")
            || combined.contains("uti")
            || combined.contains("thermal")
            || combined.contains("1d6b")
            || combined.contains("0102");

        devices.push(DeviceInfo {
            index,
            name,
            description: desc,
            is_likely_uti,
        });
    }

    Ok(devices)
}

/// Finds the first connected camera that matches the UTi-260B signatures.
pub fn find_uti_camera() -> Result<DeviceInfo> {
    let devices = query_devices()?;
    for dev in devices {
        if dev.is_likely_uti {
            return Ok(dev);
        }
    }
    Err(Error::CameraNotFound)
}

/// Camera handle for streaming live thermal video from a UTi-260B device.
pub struct UtiCamera {
    camera: Camera,
    width: u32,
    height: u32,
}

impl UtiCamera {
    /// Opens the first detected UTi-260B camera.
    pub fn open_default() -> Result<Self> {
        let dev = find_uti_camera()?;
        Self::open(dev.index)
    }

    /// Opens a camera by index.
    pub fn open(index: u32) -> Result<Self> {
        let cam_index = CameraIndex::Index(index);
        let resolution = Resolution::new(320, 240);
        let camera_format = CameraFormat::new(resolution, FrameFormat::YUYV, 25);
        let format = RequestedFormat::new::<YuyvFormat>(RequestedFormatType::Closest(camera_format));

        let mut camera = Camera::new(cam_index, format)?;
        camera.open_stream()?;

        Ok(Self {
            camera,
            width: 320,
            height: 240,
        })
    }

    /// Captures the next frame from the camera stream.
    pub fn next_frame(&mut self) -> Result<LiveFrame> {
        let frame = self.camera.frame()?;
        let buffer = frame.buffer().to_vec();

        LiveFrame::new(self.width, self.height, buffer)
    }

    /// Returns whether the camera stream is currently open.
    pub fn is_stream_open(&self) -> bool {
        self.camera.is_stream_open()
    }

    /// Stops the video stream.
    pub fn stop(&mut self) -> Result<()> {
        if self.camera.is_stream_open() {
            self.camera.stop_stream()?;
        }
        Ok(())
    }
}

impl Drop for UtiCamera {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
