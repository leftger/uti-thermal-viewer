//! # uti-thermal-viewer
//!
//! An open-source Rust library and application for capturing live video/image data
//! and reading thermal telemetry from the **UNI-T UTi260B** handheld thermal camera.
//!
//! ## Overview
//!
//! When the UTi260B is connected over USB and set to **"PC Camera"** (or Live Screen mode),
//! it exposes a standard USB Video Class (UVC) stream.
//!
//! - **Video Resolution**: 320x240 @ 25 FPS
//! - **Pixel Format**: YUYV (4:2:2 chroma subsampled, 153,600 bytes per frame)
//! - **Telemetry Footer**: Embedded directly after the video payload (at offset `0x25800`),
//!   providing real-time maximum temperature, alarm threshold, and emissivity.
//! - **Snapshot Analysis**: Parses `.bmp` images recorded by the camera on its SD card,
//!   including the embedded raw thermal sensor values, palette, and calibration metadata.
//!
//! ## Example
//!
//! ```no_run
//! use uti_thermal_viewer::{UtiCamera, Result};
//!
//! fn main() -> Result<()> {
//!     // Automatically discover and open the UTi-260B
//!     let mut camera = UtiCamera::open_default()?;
//!
//!     // Pull a live frame
//!     let frame = camera.next_frame()?;
//!     if let Some(telem) = &frame.telemetry {
//!         println!("Max Temp: {:.1}°C, Emissivity: {:.2}", telem.max_temp_c, telem.emissivity);
//!     }
//!
//!     // Save as PNG
//!     frame.save_png("snapshot.png")?;
//!     Ok(())
//! }
//! ```

pub mod bmp;
pub mod camera;
pub mod error;
pub mod frame;
pub mod gui;
pub mod palette;
pub mod simulated;
pub mod telemetry;

pub use bmp::UtiBmpImage;
pub use camera::{find_uti_camera, query_devices, DeviceInfo, UtiCamera};
pub use error::{Error, Result};
pub use frame::LiveFrame;
pub use gui::UtiApp;
pub use palette::Palette;
pub use simulated::SimulatedCamera;
pub use telemetry::Telemetry;
