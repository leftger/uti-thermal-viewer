use serde::{Deserialize, Serialize};

/// Telemetry information embedded in each live frame from the UTi-260B camera.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Telemetry {
    /// Maximum detected temperature in Celsius (°C).
    pub max_temp_c: f32,
    /// Alarm / warning threshold or minimum temperature in Celsius (°C).
    pub warn_temp_c: f32,
    /// Configured emissivity factor (e.g. 0.95).
    pub emissivity: f32,
}

impl Telemetry {
    /// Expected byte offset of telemetry data in a 320x240 YUYV live frame.
    pub const FRAME_OFFSET: usize = 320 * 240 * 2; // 153,600 (0x25800)

    /// Parses telemetry from the raw frame buffer.
    /// Returns `None` if the buffer is smaller than `FRAME_OFFSET + 6`.
    pub fn parse_from_frame(data: &[u8]) -> Option<Self> {
        if data.len() < Self::FRAME_OFFSET + 6 {
            return None;
        }

        let slice = &data[Self::FRAME_OFFSET..];
        let max_raw = i16::from_le_bytes([slice[0], slice[1]]);
        let warn_raw = i16::from_le_bytes([slice[2], slice[3]]);
        let emiss_raw = u16::from_le_bytes([slice[4], slice[5]]);

        Some(Self {
            max_temp_c: (max_raw as f32) / 10.0,
            warn_temp_c: (warn_raw as f32) / 10.0,
            emissivity: (emiss_raw as f32) / 100.0,
        })
    }

    /// Maximum temperature in Fahrenheit (°F).
    #[inline]
    pub fn max_temp_f(&self) -> f32 {
        self.max_temp_c * 1.8 + 32.0
    }

    /// Warning/alarm temperature in Fahrenheit (°F).
    #[inline]
    pub fn warn_temp_f(&self) -> f32 {
        self.warn_temp_c * 1.8 + 32.0
    }
}

impl std::fmt::Display for Telemetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Max: {:.1}°C ({:.1}°F) | Warn: {:.1}°C ({:.1}°F) | Emissivity: {:.2}",
            self.max_temp_c,
            self.max_temp_f(),
            self.warn_temp_c,
            self.warn_temp_f(),
            self.emissivity
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_telemetry() {
        let mut buffer = vec![0u8; Telemetry::FRAME_OFFSET + 6];
        // 25.4 °C = 254 (0x00fe)
        buffer[Telemetry::FRAME_OFFSET] = 0xfe;
        buffer[Telemetry::FRAME_OFFSET + 1] = 0x00;
        // 37.0 °C = 370 (0x0172)
        buffer[Telemetry::FRAME_OFFSET + 2] = 0x72;
        buffer[Telemetry::FRAME_OFFSET + 3] = 0x01;
        // 0.95 emissivity = 95 (0x005f)
        buffer[Telemetry::FRAME_OFFSET + 4] = 0x5f;
        buffer[Telemetry::FRAME_OFFSET + 5] = 0x00;

        let telem = Telemetry::parse_from_frame(&buffer).expect("Should parse");
        assert!((telem.max_temp_c - 25.4).abs() < 1e-4);
        assert!((telem.warn_temp_c - 37.0).abs() < 1e-4);
        assert!((telem.emissivity - 0.95).abs() < 1e-4);
    }
}
