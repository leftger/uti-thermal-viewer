# uti260b-rs

An open-source Rust library and command-line application for pulling live thermal video, images, and telemetry data from the **UNI-T UTi260B** handheld thermal camera.

---

## Technical Background & Reverse-Engineering Findings

Analysis and reverse-engineering of the official UNI-T `UTi-Live-Screen-1.68.exe` software revealed how the camera communicates over USB:

1. **USB Interface & Device Discovery**:
   - In the camera settings: **USB Mode** must be set to **"PC Camera"** (or Live Screen / screen projection mode).
   - The camera enumerates as a standard **UVC (USB Video Class)** webcam device:
     - **Vendor ID**: `0x1d6b` (Linux Foundation Gadget)
     - **Product ID**: `0x0102`
     - **Device Name**: `UVC Camera`

2. **Live Video Stream**:
   - **Resolution**: `320 x 240` @ 25 FPS
   - **Pixel Format**: `YUYV` (`YUY2`, 4:2:2 chroma subsampled, 2 bytes/pixel)
   - **Video Buffer Size**: `320 * 240 * 2 = 153,600` bytes (`0x25800`).

3. **Live Telemetry Footer**:
   - Immediately following the video payload (at byte offset `153,600` / `0x25800`), the camera sends real-time telemetry metadata:
     - `offset + 0x00`: Little-endian `i16` / `10.0` = **Maximum Temperature** in °C.
     - `offset + 0x02`: Little-endian `i16` / `10.0` = **Warning / Minimum Temperature** in °C.
     - `offset + 0x04`: Little-endian `u16` / `100.0` = **Emissivity** factor.

4. **Snapshot BMP Structure (SD Card / Recorded Images)**:
   - Contains a standard Windows 24-bit BMP image (with UI overlays).
   - Appended to the end of the BMP is:
     - Raw 8-bit thermal sensor intensities (`width * height` bytes).
     - 512 bytes 256-color RGB565 palette.
     - 26 bytes calibration & temperature telemetry block (`T_max`, `T_min`, `T_center`, emissivity, coordinates).
     - Allows reconstructing full floating-point temperature matrices and exporting clean thermal images without UI overlays.

---

> [!WARNING]
> **Hardware Power Warning**: Community tests on EEVblog show that the UTi260B internal power regulation circuitry can be damaged by certain USB-C PD fast chargers. Always connect the device using a standard 5V-only USB-A to USB-C cable.

---

## Features

- **Automatic Device Discovery**: Auto-detects connected UTi-260B devices across Linux (V4L2), Windows (MSMF), and macOS (AVFoundation).
- **Zero-Copy Frame Processing**: Fast YUYV to RGB, RGBA, and Grayscale conversion.
- **Real-Time Telemetry Extraction**: High-precision maximum, warning, and emissivity readout.
- **Terminal TrueColor Preview**: View live thermal imaging directly in your terminal using 24-bit ANSI colors.
- **Snapshot Analysis**: Full support for reading raw radiometric sensor data and metadata from UTi260B `.bmp` files.
- **Temperature Export**: Export per-pixel temperature matrices to CSV.

---

## CLI Usage

### Build and Install
```bash
cargo build --release --bin uti260b-cli
```

### Detect Connected Cameras
```bash
cargo run --bin uti260b-cli -- detect
```

### Live Telemetry Streaming
```bash
cargo run --bin uti260b-cli -- stream
```

### Terminal Live Thermal Preview (ASCII/ANSI TrueColor)
```bash
cargo run --bin uti260b-cli -- preview --width 80 --height 30
```

### Capture a Snapshot
```bash
cargo run --bin uti260b-cli -- capture --output snapshot.png --json telemetry.json
```

### Parse an Exported BMP from Camera
```bash
cargo run --bin uti260b-cli -- parse-bmp /path/to/capture.bmp --export-png clean_thermal.png --export-csv temperatures.csv
```

---

## Rust Library Usage

Add `uti260b` to your `Cargo.toml`:

```toml
[dependencies]
uti260b = { path = "../uti260b" }
```

### Example: Live Streaming and Frame Capture
```rust
use uti260b::{UtiCamera, Result};

fn main() -> Result<()> {
    // Automatically find and open the UTi-260B
    let mut camera = UtiCamera::open_default()?;

    // Read next frame
    let frame = camera.next_frame()?;

    if let Some(telem) = &frame.telemetry {
        println!("Max: {:.1}°C ({:.1}°F)", telem.max_temp_c, telem.max_temp_f());
        println!("Warn: {:.1}°C, Emissivity: {:.2}", telem.warn_temp_c, telem.emissivity);
    }

    // Save as PNG
    frame.save_png("live_snapshot.png")?;

    Ok(())
}
```

### Example: Parsing Recorded BMP Radiometric Images
```rust
use uti260b::{UtiBmpImage, Palette, Result};

fn main() -> Result<()> {
    let bmp = UtiBmpImage::from_file("sample.bmp")?;

    println!("Temp Range: {:.1}°C - {:.1}°C", bmp.temp_min, bmp.temp_max);

    // Export per-pixel temperature CSV
    let csv = bmp.export_temperature_csv();
    std::fs::write("matrix.csv", csv)?;

    // Render clean image without on-screen UI
    let clean_img = bmp.render_clean_image(Some(Palette::Iron));
    clean_img.save("clean_thermal.png")?;

    Ok(())
}
```

---

## License

Dual-licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))
