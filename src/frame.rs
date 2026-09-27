use std::path::Path;
use image::{GrayImage, Rgb, RgbImage, Rgba, RgbaImage};

use crate::error::{Error, Result};
use crate::telemetry::Telemetry;

/// A live frame captured from the UTi-260B camera.
#[derive(Debug, Clone)]
pub struct LiveFrame {
    /// Width of the frame (standard 320 for UTi-260B live screen).
    pub width: u32,
    /// Height of the frame (standard 240 for UTi-260B live screen).
    pub height: u32,
    /// Raw frame buffer (including YUYV pixel data and any telemetry footer).
    pub raw_data: Vec<u8>,
    /// Extracted telemetry (temperatures, emissivity), if present.
    pub telemetry: Option<Telemetry>,
}

impl LiveFrame {
    /// Expected video payload size in bytes for a 320x240 YUYV frame (2 bytes per pixel).
    pub const VIDEO_SIZE: usize = 320 * 240 * 2; // 153,600 bytes

    /// Creates a new `LiveFrame` from a raw frame buffer.
    pub fn new(width: u32, height: u32, raw_data: Vec<u8>) -> Result<Self> {
        let expected_min_size = (width * height * 2) as usize;
        if raw_data.len() < expected_min_size {
            return Err(Error::FrameTooSmall {
                expected: expected_min_size,
                actual: raw_data.len(),
            });
        }

        let telemetry = Telemetry::parse_from_frame(&raw_data);

        Ok(Self {
            width,
            height,
            raw_data,
            telemetry,
        })
    }

    /// Converts the YUYV frame to an RGB image.
    pub fn to_rgb_image(&self) -> RgbImage {
        let mut img = RgbImage::new(self.width, self.height);
        let yuyv = &self.raw_data[..((self.width * self.height * 2) as usize)];

        for (chunk_idx, chunk) in yuyv.chunks_exact(4).enumerate() {
            let y0 = chunk[0] as f32;
            let u = chunk[1] as f32 - 128.0;
            let y1 = chunk[2] as f32;
            let v = chunk[3] as f32 - 128.0;

            let pixel_idx = (chunk_idx * 2) as u32;
            let x0 = pixel_idx % self.width;
            let y = pixel_idx / self.width;
            let x1 = x0 + 1;

            let r0 = (y0 + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g0 = (y0 - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
            let b0 = (y0 + 1.772 * u).clamp(0.0, 255.0) as u8;

            let r1 = (y1 + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g1 = (y1 - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
            let b1 = (y1 + 1.772 * u).clamp(0.0, 255.0) as u8;

            img.put_pixel(x0, y, Rgb([r0, g0, b0]));
            if x1 < self.width {
                img.put_pixel(x1, y, Rgb([r1, g1, b1]));
            }
        }

        img
    }

    /// Converts the YUYV frame to an RGBA image.
    pub fn to_rgba_image(&self) -> RgbaImage {
        let rgb = self.to_rgb_image();
        let mut rgba = RgbaImage::new(self.width, self.height);
        for (x, y, pixel) in rgb.enumerate_pixels() {
            rgba.put_pixel(x, y, Rgba([pixel[0], pixel[1], pixel[2], 255]));
        }
        rgba
    }

    /// Converts the YUYV frame to a Grayscale (luma Y only) image.
    pub fn to_gray_image(&self) -> GrayImage {
        let mut img = GrayImage::new(self.width, self.height);
        let yuyv = &self.raw_data[..((self.width * self.height * 2) as usize)];

        for (chunk_idx, chunk) in yuyv.chunks_exact(4).enumerate() {
            let y0 = chunk[0];
            let y1 = chunk[2];

            let pixel_idx = (chunk_idx * 2) as u32;
            let x0 = pixel_idx % self.width;
            let y = pixel_idx / self.width;
            let x1 = x0 + 1;

            img.put_pixel(x0, y, image::Luma([y0]));
            if x1 < self.width {
                img.put_pixel(x1, y, image::Luma([y1]));
            }
        }

        img
    }

    /// Saves the frame as a PNG file.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<()> {
        let img = self.to_rgb_image();
        img.save_with_format(path, image::ImageFormat::Png)?;
        Ok(())
    }

    /// Saves the frame as a BMP file.
    pub fn save_bmp(&self, path: impl AsRef<Path>) -> Result<()> {
        let img = self.to_rgb_image();
        img.save_with_format(path, image::ImageFormat::Bmp)?;
        Ok(())
    }

    /// Generates an ASCII/ANSI color block preview for terminal display.
    pub fn render_ascii(&self, term_width: usize, term_height: usize) -> String {
        let rgb = self.to_rgb_image();
        let mut output = String::new();

        let step_x = (self.width as f32) / (term_width as f32);
        let step_y = (self.height as f32) / (term_height as f32);

        // Character ramp from darkest to brightest
        const CHARS: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '%', '@', '#'];

        for ty in 0..term_height {
            for tx in 0..term_width {
                let sx = ((tx as f32 * step_x) as u32).min(self.width - 1);
                let sy = ((ty as f32 * step_y) as u32).min(self.height - 1);
                let pixel = rgb.get_pixel(sx, sy);

                // Perceived luminance: 0.299 R + 0.587 G + 0.114 B
                let luma = 0.299 * (pixel[0] as f32) + 0.587 * (pixel[1] as f32) + 0.114 * (pixel[2] as f32);
                let idx = ((luma / 255.0) * (CHARS.len() - 1) as f32).round() as usize;
                let ch = CHARS[idx.min(CHARS.len() - 1)];

                // ANSI 24-bit truecolor output
                output.push_str(&format!("\x1b[38;2;{};{};{}m{}", pixel[0], pixel[1], pixel[2], ch));
            }
            output.push_str("\x1b[0m\n");
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_live_frame_yuyv_conversion() {
        let width = 4;
        let height = 2;
        // 4 * 2 * 2 = 16 bytes
        let yuyv = vec![
            // Row 0: 2 pairs
            128, 128, 128, 128, // Y0, U, Y1, V
            200, 128, 200, 128,
            // Row 1: 2 pairs
            50, 128, 50, 128,
            255, 128, 255, 128,
        ];

        let frame = LiveFrame::new(width, height, yuyv).expect("Valid frame");
        let rgb = frame.to_rgb_image();
        assert_eq!(rgb.width(), 4);
        assert_eq!(rgb.height(), 2);

        // Neutral U=128, V=128 means R=G=B=Y
        assert_eq!(rgb.get_pixel(0, 0), &Rgb([128, 128, 128]));
        assert_eq!(rgb.get_pixel(1, 0), &Rgb([128, 128, 128]));
        assert_eq!(rgb.get_pixel(2, 0), &Rgb([200, 200, 200]));
    }
}
