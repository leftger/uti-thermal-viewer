use std::path::PathBuf;
use std::time::{Duration, Instant};
use clap::{Parser, Subcommand};
use uti_thermal_viewer::{query_devices, Palette, Result, TelemetryLogRecord, UtiBmpImage, UtiCamera};

#[derive(Parser)]
#[command(name = "uti-thermal-viewer-cli")]
#[command(author = "leftger")]
#[command(version = "0.1.0")]
#[command(about = "CLI tool for UNI-T UTi260B thermal imaging camera", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Detect and list all video capture devices on the system
    Detect,

    /// Pull and save a single snapshot from the live camera stream
    Capture {
        /// Camera device index (defaults to auto-detecting UTi-260B)
        #[arg(short, long)]
        index: Option<u32>,

        /// Output image path (.png or .bmp, defaults to timestamped filename)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output telemetry to JSON file
        #[arg(long)]
        json: Option<PathBuf>,

        /// Output telemetry to CSV file
        #[arg(long)]
        csv: Option<PathBuf>,
    },

    /// Live telemetry streaming in the console with optional CSV/JSON logging
    Stream {
        /// Camera device index (defaults to auto-detecting UTi-260B)
        #[arg(short, long)]
        index: Option<u32>,

        /// Maximum number of frames to capture (0 = indefinite)
        #[arg(short, long, default_value_t = 0)]
        count: u64,

        /// Log telemetry data continuously to a CSV file
        #[arg(long)]
        csv: Option<PathBuf>,

        /// Log telemetry data continuously to a JSON / JSONL file
        #[arg(long)]
        json: Option<PathBuf>,
    },

    /// Live ANSI TrueColor thermal preview directly in the terminal
    Preview {
        /// Camera device index (defaults to auto-detecting UTi-260B)
        #[arg(short, long)]
        index: Option<u32>,

        /// Terminal preview width in characters
        #[arg(long, default_value_t = 80)]
        width: usize,

        /// Terminal preview height in characters
        #[arg(long, default_value_t = 30)]
        height: usize,
    },

    /// Parse an exported .bmp image from the camera SD card
    ParseBmp {
        /// Path to the UTi260B BMP file
        file: PathBuf,

        /// Export clean thermal image (no camera UI overlay) to PNG
        #[arg(long)]
        export_png: Option<PathBuf>,

        /// Export per-pixel temperature matrix to CSV
        #[arg(long)]
        export_csv: Option<PathBuf>,

        /// Export radiometric data and per-pixel temperatures to JSON
        #[arg(long)]
        export_json: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Detect => {
            println!("Scanning for connected video devices...");
            let devices = query_devices()?;
            if devices.is_empty() {
                println!("No video capture devices found on the system.");
                println!("Make sure your UTi-260B is connected and USB Mode is set to 'PC Camera' in the device menu.");
                return Ok(());
            }

            println!("\nFound {} device(s):", devices.len());
            for dev in devices {
                let candidate_tag = if dev.is_likely_uti {
                    " [MATCH: Likely UTi-260B]"
                } else {
                    ""
                };
                println!("  - [{}] {} ({}){}", dev.index, dev.name, dev.description, candidate_tag);
            }
        }

        Commands::Capture { index, output, json, csv } => {
            let mut camera = match index {
                Some(idx) => UtiCamera::open(idx)?,
                None => UtiCamera::open_default()?,
            };

            println!("Capturing frame from camera...");
            let frame = camera.next_frame()?;

            let output_path = output.unwrap_or_else(|| {
                let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
                PathBuf::from(format!("uti260b_snapshot_{}.png", timestamp))
            });

            if let Some(telem) = &frame.telemetry {
                println!("Captured Telemetry: {}", telem);
                if let Some(json_path) = json {
                    let json_str = serde_json::to_string_pretty(telem).map_err(|e| {
                        uti_thermal_viewer::Error::Capture(format!("JSON serialization error: {}", e))
                    })?;
                    std::fs::write(&json_path, json_str)?;
                    println!("Saved telemetry JSON to {:?}", json_path);
                }
                if let Some(csv_path) = csv {
                    use std::io::Write;
                    let mut file = std::fs::File::create(&csv_path)?;
                    writeln!(file, "max_temp_c,warn_temp_c,emissivity")?;
                    writeln!(file, "{:.2},{:.2},{:.2}", telem.max_temp_c, telem.warn_temp_c, telem.emissivity)?;
                    println!("Saved telemetry CSV to {:?}", csv_path);
                }
            } else {
                println!("Frame captured (no extended telemetry footer found).");
            }

            frame.save_png(&output_path)?;
            println!("Saved snapshot to {:?}", output_path);
        }

        Commands::Stream { index, count, csv, json } => {
            let mut camera = match index {
                Some(idx) => UtiCamera::open(idx)?,
                None => UtiCamera::open_default()?,
            };

            let mut csv_writer = if let Some(csv_path) = &csv {
                use std::io::Write;
                let mut file = std::fs::File::create(csv_path)?;
                writeln!(file, "{}", TelemetryLogRecord::csv_header())?;
                println!("Logging telemetry CSV to {:?}", csv_path);
                Some(file)
            } else {
                None
            };

            let mut json_writer = if let Some(json_path) = &json {
                let file = std::fs::File::create(json_path)?;
                println!("Logging telemetry JSON Lines to {:?}", json_path);
                Some(file)
            } else {
                None
            };

            println!("Streaming from UTi-260B (press Ctrl+C to stop)...");
            let mut frame_count: u64 = 0;
            let start = Instant::now();
            let mut last_print = Instant::now();

            while count == 0 || frame_count < count {
                let frame = camera.next_frame()?;
                frame_count += 1;
                let elapsed = start.elapsed().as_secs_f64();

                if let Some(telem) = &frame.telemetry {
                    let fps = if elapsed > 0.0 { frame_count as f32 / elapsed as f32 } else { 0.0 };
                    let record = TelemetryLogRecord {
                        timestamp: chrono::Local::now().to_rfc3339(),
                        elapsed_secs: elapsed,
                        frame: frame_count,
                        max_temp_c: telem.max_temp_c,
                        warn_temp_c: telem.warn_temp_c,
                        min_temp_c: None,
                        center_temp_c: None,
                        emissivity: telem.emissivity,
                        fps: Some(fps),
                    };

                    if let Some(f) = &mut csv_writer {
                        use std::io::Write;
                        let _ = writeln!(f, "{}", record.to_csv_row());
                    }

                    if let Some(f) = &mut json_writer {
                        use std::io::Write;
                        if let Ok(line) = record.to_json_line() {
                            let _ = writeln!(f, "{}", line);
                        }
                    }
                }

                if last_print.elapsed() >= Duration::from_millis(500) {
                    let elapsed = start.elapsed().as_secs_f32();
                    let fps = if elapsed > 0.0 { frame_count as f32 / elapsed } else { 0.0 };

                    let telem_str = match &frame.telemetry {
                        Some(t) => format!("{}", t),
                        None => "No Telemetry".to_string(),
                    };

                    print!("\r[Frame {:6}] [FPS: {:4.1}] {}", frame_count, fps, telem_str);
                    std::io::Write::flush(&mut std::io::stdout())?;
                    last_print = Instant::now();
                }
            }
            println!("\nStreaming finished (captured {} frames).", frame_count);
        }

        Commands::Preview { index, width, height } => {
            let mut camera = match index {
                Some(idx) => UtiCamera::open(idx)?,
                None => UtiCamera::open_default()?,
            };

            println!("\x1b[2J\x1b[H"); // Clear screen
            loop {
                let frame = camera.next_frame()?;
                let ascii = frame.render_ascii(width, height);

                // Move cursor to top-left and redraw
                print!("\x1b[H{}", ascii);
                if let Some(telem) = &frame.telemetry {
                    print!("{}\n", telem);
                }
                std::io::Write::flush(&mut std::io::stdout())?;
                std::thread::sleep(Duration::from_millis(40));
            }
        }

        Commands::ParseBmp {
            file,
            export_png,
            export_csv,
            export_json,
        } => {
            println!("Parsing UTi260B BMP file: {:?}", file);
            let bmp = UtiBmpImage::from_file(&file)?;

            println!("\n=== Thermal Metadata ===");
            println!("Dimensions:      {} x {}", bmp.width, bmp.height);
            println!("Temperature Max: {:.2}°{}", bmp.temp_max, bmp.temp_units);
            println!("Temperature Min: {:.2}°{}", bmp.temp_min, bmp.temp_units);
            println!("Center Temp:     {:.2}°{}", bmp.temp_center, bmp.temp_units);
            println!("Emissivity:      {:.2}", bmp.emissivity);
            println!("Max Pos (X, Y):  ({}, {})", bmp.temp_max_pos.0, bmp.temp_max_pos.1);
            println!("Min Pos (X, Y):  ({}, {})", bmp.temp_min_pos.0, bmp.temp_min_pos.1);
            println!("Center (X, Y):   ({}, {})", bmp.temp_center_pos.0, bmp.temp_center_pos.1);
            if let Some(dt) = bmp.timestamp {
                println!("Timestamp:       {}", dt);
            }

            if let Some(png_path) = export_png {
                let clean_img = bmp.render_clean_image(Some(Palette::Iron));
                clean_img.save_with_format(&png_path, image::ImageFormat::Png)?;
                println!("\nExported clean thermal PNG to {:?}", png_path);
            }

            if let Some(csv_path) = export_csv {
                let csv_data = bmp.export_temperature_csv();
                std::fs::write(&csv_path, csv_data)?;
                println!("Exported temperature matrix CSV to {:?}", csv_path);
            }

            if let Some(json_path) = export_json {
                let json_data = bmp.export_temperature_json().map_err(|e| {
                    uti_thermal_viewer::Error::Capture(format!("JSON export error: {}", e))
                })?;
                std::fs::write(&json_path, json_data)?;
                println!("Exported radiometric JSON to {:?}", json_path);
            }
        }
    }

    Ok(())
}
