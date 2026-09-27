use std::fs::File;
use std::io::Read;
use std::path::Path;
use chrono::{DateTime, Utc};
use image::{Rgb, RgbImage};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::palette::Palette;

/// Parsed data from a UNI-T UTi260B thermal snapshot BMP file.
#[derive(Debug, Clone)]
pub struct UtiBmpImage {
    pub width: u32,
    pub height: u32,
    pub temp_units: char, // 'C' or 'F'
    pub temp_max: f32,
    pub temp_min: f32,
    pub temp_center: f32,
    pub emissivity: f32,
    pub temp_min_pos: (u16, u16),
    pub temp_max_pos: (u16, u16),
    pub temp_center_pos: (u16, u16),
    pub timestamp: Option<DateTime<Utc>>,
    /// Raw 8-bit thermal sensor matrix (0..254).
    pub raw_thermal_matrix: Vec<Vec<u8>>,
    /// Calculated per-pixel temperatures in Celsius.
    pub temperature_matrix: Vec<Vec<f32>>,
    /// 256 RGB colors extracted from the BMP palette.
    pub embedded_palette: [[u8; 3]; 256],
}

impl UtiBmpImage {
    /// Loads and parses a UTi-260B BMP file from a file path.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    /// Parses a UTi-260B BMP file from raw byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 54 || &bytes[0..2] != b"BM" {
            return Err(Error::InvalidBmp("File does not start with BM signature".into()));
        }

        let file_size = u32::from_le_bytes(bytes[2..6].try_into().unwrap()) as usize;
        let width = u32::from_le_bytes(bytes[18..22].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[22..26].try_into().unwrap());

        let raw_pixels_len = (width * height) as usize;
        let raw_img_end = file_size + raw_pixels_len;

        if bytes.len() < raw_img_end + 512 + 26 {
            return Err(Error::InvalidBmp(
                "File too small to contain UTi thermal metadata trailer".into(),
            ));
        }

        // 1. Extract raw grayscale thermal pixels
        let raw_slice = &bytes[file_size..raw_img_end];
        let mut raw_thermal_matrix = Vec::with_capacity(height as usize);
        for row in 0..height as usize {
            let start = row * width as usize;
            let end = start + width as usize;
            raw_thermal_matrix.push(raw_slice[start..end].to_vec());
        }

        // 2. Extract embedded 256-color RGB565 palette
        let palette_offset = raw_img_end;
        let mut embedded_palette = [[0u8; 3]; 256];
        for i in 0..256 {
            let p = palette_offset + i * 2;
            let color_raw = u16::from_le_bytes([bytes[p], bytes[p + 1]]);
            let r_5 = ((color_raw & 0xF800) >> 11) as f32;
            let g_6 = ((color_raw & 0x07E0) >> 5) as f32;
            let b_5 = (color_raw & 0x001F) as f32;

            let r = (r_5 * 255.0 / 31.0).round() as u8;
            let g = (g_6 * 255.0 / 63.0).round() as u8;
            let b = (b_5 * 255.0 / 31.0).round() as u8;
            embedded_palette[i] = [r, g, b];
        }

        // 3. Extract temperature metadata
        let meta_offset = palette_offset + 512;
        let temp_units = if bytes[meta_offset] == 0 { 'C' } else { 'F' };
        let temp_max = (i16::from_le_bytes([bytes[meta_offset + 1], bytes[meta_offset + 2]]) as f32) / 10.0;
        let temp_min = (i16::from_le_bytes([bytes[meta_offset + 3], bytes[meta_offset + 4]]) as f32) / 10.0;
        let temp_center = (i16::from_le_bytes([bytes[meta_offset + 7], bytes[meta_offset + 8]]) as f32) / 10.0;
        let emissivity = (bytes[meta_offset + 9] as f32) / 100.0;

        let temp_min_pos_x = u16::from_le_bytes([bytes[meta_offset + 14], bytes[meta_offset + 15]]);
        let temp_min_pos_y = u16::from_le_bytes([bytes[meta_offset + 16], bytes[meta_offset + 17]]);
        let temp_max_pos_x = u16::from_le_bytes([bytes[meta_offset + 18], bytes[meta_offset + 19]]);
        let temp_max_pos_y = u16::from_le_bytes([bytes[meta_offset + 20], bytes[meta_offset + 21]]);
        let temp_center_pos_x = u16::from_le_bytes([bytes[meta_offset + 22], bytes[meta_offset + 23]]);
        let temp_center_pos_y = u16::from_le_bytes([bytes[meta_offset + 24], bytes[meta_offset + 25]]);

        // Optional timestamp
        let mut timestamp = None;
        if bytes.len() >= meta_offset + 26 + 4 {
            let ts_secs = u32::from_le_bytes(bytes[meta_offset + 26..meta_offset + 30].try_into().unwrap()) as i64;
            if ts_secs > 0 {
                timestamp = DateTime::from_timestamp(ts_secs, 0);
            }
        }

        // 4. Calculate per-pixel temperature matrix (3-point calibration)
        let cx = (temp_center_pos_x as usize).min(width as usize - 1);
        let cy = (temp_center_pos_y as usize).min(height as usize - 1);
        let center_gray = raw_thermal_matrix[cy][cx] as f32;

        let mut temperature_matrix = Vec::with_capacity(height as usize);
        for row in 0..height as usize {
            let mut temp_row = Vec::with_capacity(width as usize);
            for col in 0..width as usize {
                let gray = raw_thermal_matrix[row][col] as f32;
                let t = if gray == 0.0 {
                    temp_min
                } else if gray == 254.0 {
                    temp_max
                } else if gray >= center_gray && (254.0 - center_gray) > 0.0 {
                    temp_center + (temp_max - temp_center) * ((gray - center_gray) / (254.0 - center_gray))
                } else if center_gray > 0.0 {
                    temp_min + (temp_center - temp_min) * (gray / center_gray)
                } else {
                    temp_min + (temp_max - temp_min) * (gray / 254.0)
                };
                temp_row.push(t);
            }
            temperature_matrix.push(temp_row);
        }

        Ok(Self {
            width,
            height,
            temp_units,
            temp_max,
            temp_min,
            temp_center,
            emissivity,
            temp_min_pos: (temp_min_pos_x, temp_min_pos_y),
            temp_max_pos: (temp_max_pos_x, temp_max_pos_y),
            temp_center_pos: (temp_center_pos_x, temp_center_pos_y),
            timestamp,
            raw_thermal_matrix,
            temperature_matrix,
            embedded_palette,
        })
    }

    /// Renders a clean thermal image (no UI overlays) using the embedded or custom palette.
    pub fn render_clean_image(&self, custom_palette: Option<Palette>) -> RgbImage {
        let mut img = RgbImage::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let gray = self.raw_thermal_matrix[y as usize][x as usize];
                let rgb = match custom_palette {
                    Some(pal) => pal.map_color(gray),
                    None => self.embedded_palette[gray as usize],
                };
                img.put_pixel(x, y, Rgb(rgb));
            }
        }
        img
    }

    /// Exports per-pixel temperature values to CSV format.
    pub fn export_temperature_csv(&self) -> String {
        let mut out = String::new();
        // Header info
        out.push_str(&format!(
            "# Units: {}, Max: {:.2}, Min: {:.2}, Center: {:.2}, Emissivity: {:.2}\n",
            self.temp_units, self.temp_max, self.temp_min, self.temp_center, self.emissivity
        ));
        for row in &self.temperature_matrix {
            let row_strs: Vec<String> = row.iter().map(|t| format!("{:.2}", t)).collect();
            out.push_str(&row_strs.join(","));
            out.push('\n');
        }
        out
    }

    /// Exports radiometric BMP data, metadata, calibration, and per-pixel temperatures to JSON.
    pub fn export_temperature_json(&self) -> serde_json::Result<String> {
        #[derive(Serialize)]
        struct BmpJsonExport<'a> {
            width: u32,
            height: u32,
            units: String,
            temp_max: f32,
            temp_min: f32,
            temp_center: f32,
            emissivity: f32,
            timestamp: Option<String>,
            temperatures: &'a Vec<Vec<f32>>,
        }

        let export = BmpJsonExport {
            width: self.width,
            height: self.height,
            units: self.temp_units.to_string(),
            temp_max: self.temp_max,
            temp_min: self.temp_min,
            temp_center: self.temp_center,
            emissivity: self.emissivity,
            timestamp: self.timestamp.as_ref().map(|dt| dt.to_rfc3339()),
            temperatures: &self.temperature_matrix,
        };

        serde_json::to_string_pretty(&export)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uti_bmp_parsing() {
        let width = 4u32;
        let height = 2u32;
        let file_size = 54 + (width * height * 3);

        let mut data = vec![0u8; file_size as usize];
        data[0] = b'B';
        data[1] = b'M';
        data[2..6].copy_from_slice(&(file_size as u32).to_le_bytes());
        data[10..14].copy_from_slice(&54u32.to_le_bytes());
        data[14..18].copy_from_slice(&40u32.to_le_bytes());
        data[18..22].copy_from_slice(&width.to_le_bytes());
        data[22..26].copy_from_slice(&height.to_le_bytes());
        data[26..28].copy_from_slice(&1u16.to_le_bytes());
        data[28..30].copy_from_slice(&24u16.to_le_bytes());

        // Trailer:
        // 1. Raw pixels
        let raw_pixels = vec![0u8, 64, 128, 192, 32, 96, 160, 254];
        data.extend_from_slice(&raw_pixels);

        // 2. Palette (512 bytes)
        data.extend_from_slice(&[0u8; 512]);

        // 3. Metadata (26 bytes)
        let mut meta = vec![0u8; 26];
        meta[0] = 0; // Celsius
        meta[1..3].copy_from_slice(&350i16.to_le_bytes()); // 35.0 C
        meta[3..5].copy_from_slice(&200i16.to_le_bytes()); // 20.0 C
        meta[7..9].copy_from_slice(&250i16.to_le_bytes()); // 25.0 C
        meta[9] = 95; // 0.95
        // Center at (1, 0) -> raw_pixel is 64
        meta[22..24].copy_from_slice(&1u16.to_le_bytes());
        meta[24..26].copy_from_slice(&0u16.to_le_bytes());

        data.extend_from_slice(&meta);

        let parsed = UtiBmpImage::from_bytes(&data).expect("Should parse valid UTi BMP");
        assert_eq!(parsed.width, 4);
        assert_eq!(parsed.height, 2);
        assert!((parsed.temp_max - 35.0).abs() < 1e-4);
        assert!((parsed.temp_min - 20.0).abs() < 1e-4);
        assert!((parsed.temp_center - 25.0).abs() < 1e-4);
        assert!((parsed.emissivity - 0.95).abs() < 1e-4);

        // Test CSV export
        let csv = parsed.export_temperature_csv();
        assert!(csv.contains("Max: 35.00"));

        // Test JSON export
        let json = parsed.export_temperature_json().expect("Should export JSON");
        assert!(json.contains("\"temp_max\": 35.0"));
        assert!(json.contains("\"temperatures\":"));

        // Test clean image export
        let img = parsed.render_clean_image(Some(Palette::Iron));
        assert_eq!(img.width(), 4);
        assert_eq!(img.height(), 2);
    }
}

