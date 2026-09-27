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

/// Detailed log record representing a single telemetry sample for CSV / JSON logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryLogRecord {
    /// ISO-8601 timestamp string (e.g. 2026-09-26T21:38:00+00:00)
    pub timestamp: String,
    /// Elapsed seconds since streaming or logging started
    pub elapsed_secs: f64,
    /// Frame sequence index
    pub frame: u64,
    /// Maximum detected temperature in Celsius (°C)
    pub max_temp_c: f32,
    /// Alarm / warning threshold temperature in Celsius (°C)
    pub warn_temp_c: f32,
    /// Minimum detected temperature in Celsius (°C) if computed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_temp_c: Option<f32>,
    /// Center spot temperature in Celsius (°C) if computed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center_temp_c: Option<f32>,
    /// Configured emissivity factor (e.g. 0.95)
    pub emissivity: f32,
    /// Instantaneous streaming frame rate (FPS) if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
}

impl TelemetryLogRecord {
    /// CSV header for TelemetryLogRecord
    pub fn csv_header() -> &'static str {
        "timestamp,elapsed_secs,frame,max_temp_c,warn_temp_c,min_temp_c,center_temp_c,emissivity,fps"
    }

    /// Formats this record as a single comma-separated row
    pub fn to_csv_row(&self) -> String {
        format!(
            "{},{:.3},{},{:.2},{:.2},{},{},{:.2},{}",
            self.timestamp,
            self.elapsed_secs,
            self.frame,
            self.max_temp_c,
            self.warn_temp_c,
            self.min_temp_c.map(|v| format!("{:.2}", v)).unwrap_or_default(),
            self.center_temp_c.map(|v| format!("{:.2}", v)).unwrap_or_default(),
            self.emissivity,
            self.fps.map(|v| format!("{:.1}", v)).unwrap_or_default(),
        )
    }

    /// Serializes this record to a single-line JSON string (JSON Lines)
    pub fn to_json_line(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
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

    #[test]
    fn test_telemetry_log_record() {
        let record = TelemetryLogRecord {
            timestamp: "2026-09-26T21:40:00Z".into(),
            elapsed_secs: 1.25,
            frame: 30,
            max_temp_c: 35.5,
            warn_temp_c: 50.0,
            min_temp_c: Some(21.0),
            center_temp_c: Some(26.0),
            emissivity: 0.95,
            fps: Some(25.0),
        };

        let csv_row = record.to_csv_row();
        assert!(csv_row.starts_with("2026-09-26T21:40:00Z,1.250,30,35.50,50.00,21.00,26.00,0.95,25.0"));

        let json_line = record.to_json_line().expect("Should serialize JSON line");
        assert!(json_line.contains("\"max_temp_c\":35.5"));
        assert!(json_line.contains("\"frame\":30"));
    }
}
