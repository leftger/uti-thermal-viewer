use std::time::Instant;
use crate::frame::LiveFrame;
use crate::telemetry::Telemetry;

/// Generates synthetic UTi-260B thermal frames for demo and offline UI testing.
pub struct SimulatedCamera {
    start_time: Instant,
    frame_count: u64,
}

impl SimulatedCamera {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            frame_count: 0,
        }
    }

    /// Generates the next simulated 320x240 YUYV frame with realistic thermal patterns.
    pub fn next_frame(&mut self) -> LiveFrame {
        self.frame_count += 1;
        let t = self.start_time.elapsed().as_secs_f32();

        let width = 320usize;
        let height = 240usize;
        let video_len = width * height * 2;
        let total_len = video_len + 6; // Include telemetry footer

        let mut raw_data = vec![0u8; total_len];

        // Simulate moving thermal hotspots (like inspecting an active circuit board)
        let cx1 = 160.0 + 50.0 * (t * 0.7).cos();
        let cy1 = 120.0 + 35.0 * (t * 0.9).sin();

        let cx2 = 100.0 + 20.0 * (t * 1.3).sin();
        let cy2 = 160.0 + 20.0 * (t * 1.1).cos();

        let mut max_luma = 0.0f32;
        let mut min_luma = 255.0f32;

        for y in 0..height {
            for x in (0..width).step_by(2) {
                // Point 1
                let d1_sq = ((x as f32) - cx1).powi(2) + ((y as f32) - cy1).powi(2);
                let heat1 = 200.0 * (-d1_sq / 1200.0).exp();

                // Point 2
                let d2_sq = ((x as f32) - cx2).powi(2) + ((y as f32) - cy2).powi(2);
                let heat2 = 160.0 * (-d2_sq / 900.0).exp();

                // Ambient gradient + subtle thermal noise
                let ambient = 35.0 + ((x + y) as f32 * 0.05);
                let noise = (((x * 17 + y * 31 + (self.frame_count as usize * 7)) % 13) as f32) - 6.0;

                let luma0 = (ambient + heat1 + heat2 + noise).clamp(16.0, 240.0);

                // Adjacent pixel
                let d1_sq_b = (((x + 1) as f32) - cx1).powi(2) + ((y as f32) - cy1).powi(2);
                let d2_sq_b = (((x + 1) as f32) - cx2).powi(2) + ((y as f32) - cy2).powi(2);
                let heat1_b = 200.0 * (-d1_sq_b / 1200.0).exp();
                let heat2_b = 160.0 * (-d2_sq_b / 900.0).exp();
                let luma1 = (ambient + heat1_b + heat2_b + noise).clamp(16.0, 240.0);

                if luma0 > max_luma { max_luma = luma0; }
                if luma0 < min_luma { min_luma = luma0; }

                // Thermal pseudo-chroma (iron palette tint in YUYV)
                let u = (128.0 - 0.25 * luma0).clamp(40.0, 200.0) as u8;
                let v = (128.0 + 0.35 * luma0).clamp(50.0, 220.0) as u8;

                let idx = (y * width + x) * 2;
                raw_data[idx] = luma0 as u8;     // Y0
                raw_data[idx + 1] = u;           // U
                raw_data[idx + 2] = luma1 as u8; // Y1
                raw_data[idx + 3] = v;           // V
            }
        }

        // Calculate temperatures
        let max_temp_c = 22.0 + (max_luma / 240.0) * 38.0 + 1.5 * (t * 0.5).sin();
        let min_temp_c = 18.0 + (min_luma / 240.0) * 8.0;
        let emissivity = 0.95;

        // Embed telemetry footer at offset 0x25800
        let max_raw = (max_temp_c * 10.0) as i16;
        let warn_raw = (min_temp_c * 10.0) as i16;
        let emiss_raw = (emissivity * 100.0) as u16;

        raw_data[video_len..video_len + 2].copy_from_slice(&max_raw.to_le_bytes());
        raw_data[video_len + 2..video_len + 4].copy_from_slice(&warn_raw.to_le_bytes());
        raw_data[video_len + 4..video_len + 6].copy_from_slice(&emiss_raw.to_le_bytes());

        LiveFrame {
            width: width as u32,
            height: height as u32,
            raw_data,
            telemetry: Some(Telemetry {
                max_temp_c,
                warn_temp_c: min_temp_c,
                emissivity,
            }),
        }
    }
}
