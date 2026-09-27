use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use image::RgbaImage;

use crate::bmp::UtiBmpImage;
use crate::camera::{query_devices, DeviceInfo, UtiCamera};
use crate::frame::LiveFrame;
use crate::palette::Palette;
use crate::simulated::SimulatedCamera;
use crate::telemetry::Telemetry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempUnit {
    Celsius,
    Fahrenheit,
}

impl TempUnit {
    pub fn format(&self, c: f32) -> String {
        match self {
            Self::Celsius => format!("{:.1}°C", c),
            Self::Fahrenheit => format!("{:.1}°F", c * 1.8 + 32.0),
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Celsius => "°C",
            Self::Fahrenheit => "°F",
        }
    }
}

/// The main eframe/egui thermal camera application.
pub struct UtiApp {
    // Camera state
    camera: Option<UtiCamera>,
    simulated: SimulatedCamera,
    is_simulated: bool,
    available_devices: Vec<DeviceInfo>,
    selected_device_index: u32,
    connection_error: Option<String>,

    // Frame state
    current_frame: Option<LiveFrame>,
    texture: Option<egui::TextureHandle>,
    palette: Palette,
    use_custom_palette: bool,
    unit: TempUnit,

    // Overlays & UI options
    show_center_spot: bool,
    show_extremes: bool,
    show_colorbar: bool,
    show_plot: bool,
    emissivity: f32,
    high_temp_alarm: f32,
    alarm_enabled: bool,
    alarm_triggered: bool,

    // Metrics & plotting
    start_time: Instant,
    temp_history: VecDeque<(f64, f32, f32, f32)>, // (time_sec, max, min, center)
    history_window_secs: f64,
    frame_count: u64,
    fps: f32,
    last_fps_update: Instant,
    frames_since_fps: u64,

    // Logging & offline analysis
    status_toast: Option<(String, Instant)>,
    csv_logger: Option<(PathBuf, std::fs::File)>,
    offline_bmp: Option<UtiBmpImage>,

    // Screenshot capture
    screenshot_out: Option<PathBuf>,
    screenshot_requested_at: Option<Instant>,
    pending_screenshot: Option<PathBuf>,
}

impl UtiApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot_out: Option<PathBuf>) -> Self {
        // Set visual styling to match dark scientific instrumentation theme
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(18, 20, 24);
        visuals.window_fill = Color32::from_rgb(24, 26, 32);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(28, 30, 36);
        cc.egui_ctx.set_visuals(visuals);

        let devices = query_devices().unwrap_or_default();
        let likely = devices.iter().find(|d| d.is_likely_uti).map(|d| d.index).unwrap_or(0);

        let mut app = Self {
            camera: None,
            simulated: SimulatedCamera::new(),
            is_simulated: true, // Default to demo simulation until connected
            available_devices: devices,
            selected_device_index: likely,
            connection_error: None,

            current_frame: None,
            texture: None,
            palette: Palette::Iron,
            use_custom_palette: true,
            unit: TempUnit::Celsius,

            show_center_spot: true,
            show_extremes: true,
            show_colorbar: true,
            show_plot: true,
            emissivity: 0.95,
            high_temp_alarm: 45.0,
            alarm_enabled: true,
            alarm_triggered: false,

            start_time: Instant::now(),
            temp_history: VecDeque::with_capacity(3600),
            history_window_secs: 45.0,
            frame_count: 0,
            fps: 0.0,
            last_fps_update: Instant::now(),
            frames_since_fps: 0,

            status_toast: None,
            csv_logger: None,
            offline_bmp: None,

            screenshot_out,
            screenshot_requested_at: None,
            pending_screenshot: None,
        };

        // Attempt connecting to real camera if present
        if app.available_devices.iter().any(|d| d.is_likely_uti) {
            app.connect_camera(app.selected_device_index);
        }

        app
    }

    fn connect_camera(&mut self, index: u32) {
        match UtiCamera::open(index) {
            Ok(cam) => {
                self.camera = Some(cam);
                self.is_simulated = false;
                self.connection_error = None;
                self.set_toast(format!("Connected to camera device [{}]", index));
            }
            Err(e) => {
                self.connection_error = Some(format!("{}", e));
                self.camera = None;
                self.is_simulated = true;
                self.set_toast(format!("Hardware connection failed: {}. Switched to Demo Mode.", e));
            }
        }
    }

    fn disconnect_camera(&mut self) {
        if let Some(mut cam) = self.camera.take() {
            let _ = cam.stop();
        }
        self.is_simulated = true;
        self.set_toast("Disconnected from camera (Demo Mode active)".into());
    }

    fn set_toast(&mut self, msg: String) {
        self.status_toast = Some((msg, Instant::now()));
    }

    fn fetch_next_frame(&mut self) {
        let frame = if let Some(cam) = &mut self.camera {
            match cam.next_frame() {
                Ok(f) => f,
                Err(e) => {
                    self.connection_error = Some(format!("Frame read error: {}", e));
                    self.disconnect_camera();
                    self.simulated.next_frame()
                }
            }
        } else {
            self.simulated.next_frame()
        };

        // Update FPS counter
        self.frame_count += 1;
        self.frames_since_fps += 1;
        let elapsed = self.last_fps_update.elapsed();
        if elapsed >= Duration::from_millis(500) {
            self.fps = self.frames_since_fps as f32 / elapsed.as_secs_f32();
            self.frames_since_fps = 0;
            self.last_fps_update = Instant::now();
        }

        // Record history
        if let Some(telem) = &frame.telemetry {
            let t = self.start_time.elapsed().as_secs_f64();
            let center_temp = (telem.max_temp_c + telem.warn_temp_c) * 0.5; // Approximation if center not explicitly tracked
            self.temp_history.push_back((t, telem.max_temp_c, telem.warn_temp_c, center_temp));

            // Prune old history
            while let Some(&(old_t, _, _, _)) = self.temp_history.front() {
                if t - old_t > self.history_window_secs * 2.0 {
                    self.temp_history.pop_front();
                } else {
                    break;
                }
            }

            // Check alarm
            if self.alarm_enabled && telem.max_temp_c >= self.high_temp_alarm {
                self.alarm_triggered = true;
            } else {
                self.alarm_triggered = false;
            }

            // CSV logging
            if let Some((_, file)) = &mut self.csv_logger {
                let _ = writeln!(
                    file,
                    "{:.3},{:.2},{:.2},{:.2}",
                    t, telem.max_temp_c, telem.warn_temp_c, telem.emissivity
                );
            }
        }

        self.current_frame = Some(frame);
    }

    fn update_texture(&mut self, ctx: &egui::Context) {
        if let Some(frame) = &self.current_frame {
            let rgba: RgbaImage = if self.use_custom_palette {
                let gray = frame.to_gray_image();
                let mut remapped = RgbaImage::new(frame.width, frame.height);
                for (x, y, p) in gray.enumerate_pixels() {
                    let rgb = self.palette.map_color(p[0]);
                    remapped.put_pixel(x, y, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
                }
                remapped
            } else {
                frame.to_rgba_image()
            };

            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                rgba.as_raw(),
            );

            match &mut self.texture {
                Some(tex) => tex.set(color_image, egui::TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ctx.load_texture(
                        "uti260b_thermal",
                        color_image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
        }
    }

    fn take_snapshot(&mut self) {
        if let Some(frame) = &self.current_frame {
            let filename = format!(
                "uti260b_snapshot_{}.png",
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            );
            if let Err(e) = frame.save_png(&filename) {
                self.set_toast(format!("Failed to save snapshot: {}", e));
            } else {
                self.set_toast(format!("Saved snapshot to {}", filename));
            }
        }
    }

    fn take_window_screenshot(&mut self, ctx: &egui::Context) {
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let filename = PathBuf::from(format!("screenshot_{}.png", timestamp));
        self.pending_screenshot = Some(filename);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }

    fn toggle_csv_logging(&mut self) {
        if self.csv_logger.is_some() {
            self.csv_logger = None;
            self.set_toast("Stopped CSV temperature logging".into());
        } else {
            let filename = PathBuf::from(format!(
                "uti260b_log_{}.csv",
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            ));
            match OpenOptions::new().create(true).write(true).open(&filename) {
                Ok(mut file) => {
                    let _ = writeln!(file, "time_secs,max_temp_c,warn_temp_c,emissivity");
                    self.set_toast(format!("Logging temperatures to {:?}", filename));
                    self.csv_logger = Some((filename, file));
                }
                Err(e) => {
                    self.set_toast(format!("Failed to start CSV log: {}", e));
                }
            }
        }
    }

    fn open_offline_bmp(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("UTi BMP Snapshot", &["bmp"])
            .pick_file()
        {
            match UtiBmpImage::from_file(&path) {
                Ok(bmp) => {
                    self.set_toast(format!("Loaded BMP snapshot: {:?}", path.file_name().unwrap()));
                    self.offline_bmp = Some(bmp);
                }
                Err(e) => {
                    self.set_toast(format!("Failed to parse BMP: {}", e));
                }
            }
        }
    }
}

impl eframe::App for UtiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Screenshot collection
        if let Some(path) = self.pending_screenshot.clone() {
            let screenshot = ctx.input(|i| {
                i.raw.events.iter().find_map(|ev| match ev {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = screenshot {
                self.pending_screenshot = None;
                let [w, h] = image.size;
                let mut raw = Vec::with_capacity(w * h * 4);
                for p in &image.pixels {
                    raw.extend_from_slice(&p.to_array());
                }
                if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, raw) {
                    if let Some(parent) = path.parent() {
                        if !parent.as_os_str().is_empty() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                    }
                    if let Err(e) = img.save(&path) {
                        self.set_toast(format!("Failed to save screenshot: {}", e));
                    } else {
                        self.set_toast(format!("Saved window screenshot to {:?}", path));
                    }
                }
                if self.screenshot_out.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        // Automatic screenshot trigger for CLI screenshot mode
        if let Some(path) = self.screenshot_out.clone() {
            if self.frame_count >= 10 && self.pending_screenshot.is_none() && self.screenshot_requested_at.is_none() {
                self.screenshot_requested_at = Some(Instant::now());
                self.pending_screenshot = Some(path);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
        }

        // Fetch next frame and update texture
        self.fetch_next_frame();
        self.update_texture(ctx);

        // 1. Top Control Bar
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("🔥 UTi-Thermal-Viewer");
                ui.separator();

                // Connection badge
                if !self.is_simulated && self.camera.is_some() {
                    ui.colored_label(Color32::from_rgb(80, 220, 100), "🟢 LIVE STREAM");
                } else {
                    ui.colored_label(Color32::from_rgb(240, 200, 60), "🟡 DEMO / SIMULATION");
                }

                ui.separator();

                // Device selector
                egui::ComboBox::from_label("")
                    .selected_text(
                        self.available_devices
                            .iter()
                            .find(|d| d.index == self.selected_device_index)
                            .map(|d| format!("[{}] {}", d.index, d.name))
                            .unwrap_or_else(|| "Select Device".into()),
                    )
                    .show_ui(ui, |ui| {
                        for d in &self.available_devices {
                            let label = format!("[{}] {}{}", d.index, d.name, if d.is_likely_uti { " ★" } else { "" });
                            ui.selectable_value(&mut self.selected_device_index, d.index, label);
                        }
                    });

                if self.camera.is_none() {
                    if ui.button("🔌 Connect").clicked() {
                        self.connect_camera(self.selected_device_index);
                    }
                } else {
                    if ui.button("⏹ Disconnect").clicked() {
                        self.disconnect_camera();
                    }
                }

                if ui.button("🔄 Rescan").clicked() {
                    self.available_devices = query_devices().unwrap_or_default();
                    self.set_toast(format!("Found {} capture device(s)", self.available_devices.len()));
                }

                ui.separator();

                // Snapshot button
                if ui.button("📷 Snapshot").clicked() {
                    self.take_snapshot();
                }

                // Window Screenshot button
                if ui.button("🖼 Screenshot").clicked() {
                    self.take_window_screenshot(ctx);
                }

                // CSV recording button
                let rec_label = if self.csv_logger.is_some() {
                    "🔴 Stop CSV"
                } else {
                    "📊 Log CSV"
                };
                if ui.button(rec_label).clicked() {
                    self.toggle_csv_logging();
                }

                // Open BMP button
                if ui.button("📂 Open BMP").clicked() {
                    self.open_offline_bmp();
                }

                ui.separator();

                // FPS display
                ui.label(format!("FPS: {:4.1}", self.fps));
            });
        });

        // 2. Bottom Toast & Trend Plot Panel
        if self.show_plot {
            egui::TopBottomPanel::bottom("bottom_plot")
                .resizable(true)
                .default_height(160.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("📈 Real-Time Temperature History").strong());
                        ui.separator();
                        ui.label(format!("Window: {:.0}s", self.history_window_secs));
                        ui.add(egui::Slider::new(&mut self.history_window_secs, 10.0..=120.0).text("secs"));
                        if ui.button("Clear History").clicked() {
                            self.temp_history.clear();
                        }
                    });

                    let max_pts: PlotPoints = self
                        .temp_history
                        .iter()
                        .map(|&(t, max, _, _)| [t, max as f64])
                        .collect();
                    let min_pts: PlotPoints = self
                        .temp_history
                        .iter()
                        .map(|&(t, _, min, _)| [t, min as f64])
                        .collect();
                    let center_pts: PlotPoints = self
                        .temp_history
                        .iter()
                        .map(|&(t, _, _, center)| [t, center as f64])
                        .collect();

                    let line_max = Line::new("Max Temp", max_pts)
                        .color(Color32::from_rgb(255, 80, 80));
                    let line_min = Line::new("Min Temp", min_pts)
                        .color(Color32::from_rgb(80, 160, 255));
                    let line_center = Line::new("Center Temp", center_pts)
                        .color(Color32::from_rgb(100, 255, 120));

                    Plot::new("temp_plot")
                        .legend(Legend::default())
                        .y_axis_label(format!("Temp ({})", self.unit.symbol()))
                        .x_axis_label("Time (s)")
                        .show(ui, |plot_ui| {
                            plot_ui.line(line_max);
                            plot_ui.line(line_min);
                            plot_ui.line(line_center);
                        });
                });
        }

        // 3. Right Dashboard Panel
        egui::SidePanel::right("right_dashboard")
            .min_width(280.0)
            .show(ctx, |ui| {
                ui.heading("Telemetry & Analysis");
                ui.separator();

                // Large digital temperature gauges
                if let Some(frame) = &self.current_frame {
                    if let Some(telem) = &frame.telemetry {
                        // Max Temp Card
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.colored_label(Color32::from_rgb(255, 90, 90), "▲ MAX TEMP");
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if self.alarm_triggered {
                                        ui.colored_label(Color32::RED, "⚠ ALARM");
                                    }
                                });
                            });
                            ui.label(
                                egui::RichText::new(self.unit.format(telem.max_temp_c))
                                    .size(32.0)
                                    .strong()
                                    .color(Color32::from_rgb(255, 100, 100)),
                            );
                        });

                        ui.add_space(4.0);

                        // Min / Warn Temp Card
                        ui.group(|ui| {
                            ui.colored_label(Color32::from_rgb(90, 170, 255), "▼ MIN / WARN TEMP");
                            ui.label(
                                egui::RichText::new(self.unit.format(telem.warn_temp_c))
                                    .size(24.0)
                                    .strong()
                                    .color(Color32::from_rgb(100, 180, 255)),
                            );
                        });

                        ui.add_space(4.0);

                        // Emissivity Display & Adjustment
                        ui.group(|ui| {
                            ui.label("Emissivity Factor");
                            ui.add(egui::Slider::new(&mut self.emissivity, 0.10..=1.00).text("ε"));
                        });
                    }
                }

                ui.separator();

                // Alarm Configuration
                ui.collapsing("🔔 Temperature Alarm", |ui| {
                    ui.checkbox(&mut self.alarm_enabled, "Enable High Temp Alarm");
                    ui.add(
                        egui::Slider::new(&mut self.high_temp_alarm, 20.0..=150.0)
                            .text(self.unit.symbol())
                            .suffix(self.unit.symbol()),
                    );
                    if self.alarm_triggered {
                        ui.colored_label(
                            Color32::RED,
                            format!("⚠ Triggered: Exceeds {:.1}{}", self.high_temp_alarm, self.unit.symbol()),
                        );
                    }
                });

                ui.separator();

                // Palette selection
                ui.collapsing("🎨 Thermal Palette", |ui| {
                    ui.checkbox(&mut self.use_custom_palette, "Apply False Color Remapping");
                    ui.radio_value(&mut self.palette, Palette::Iron, "Ironbow (Default)");
                    ui.radio_value(&mut self.palette, Palette::Rainbow, "Rainbow (Spectrum)");
                    ui.radio_value(&mut self.palette, Palette::WhiteHot, "White Hot (Greyscale)");
                    ui.radio_value(&mut self.palette, Palette::BlackHot, "Black Hot (Inverted)");
                    ui.radio_value(&mut self.palette, Palette::RedHot, "Red Hot (Hotspots)");
                });

                ui.separator();

                // Display Settings
                ui.collapsing("⚙ Display & Overlay", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Unit:");
                        ui.radio_value(&mut self.unit, TempUnit::Celsius, "°C");
                        ui.radio_value(&mut self.unit, TempUnit::Fahrenheit, "°F");
                    });
                    ui.checkbox(&mut self.show_center_spot, "Show Center Crosshair");
                    ui.checkbox(&mut self.show_extremes, "Track Max/Min Spots");
                    ui.checkbox(&mut self.show_colorbar, "Show Thermal Colorbar");
                    ui.checkbox(&mut self.show_plot, "Show Temperature Graph");
                });

                ui.separator();

                // Status Toast
                if let Some((msg, time)) = &self.status_toast {
                    if time.elapsed() < Duration::from_secs(5) {
                        ui.info_label(msg);
                    }
                }
            });

        // 4. Central Panel: Thermal Video & Overlays
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(tex) = &self.texture {
                let avail_size = ui.available_size();
                let aspect = 320.0 / 240.0;

                // Fit aspect ratio within available area
                let (w, h) = if avail_size.x / avail_size.y > aspect {
                    (avail_size.y * aspect, avail_size.y)
                } else {
                    (avail_size.x, avail_size.x / aspect)
                };

                let image_size = Vec2::new(w, h);
                let (rect, response) = ui.allocate_exact_size(image_size, egui::Sense::hover());

                // Draw video texture
                ui.painter().image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );

                // Draw Overlays
                if let Some(frame) = &self.current_frame {
                    let scale_x = rect.width() / (frame.width as f32);
                    let scale_y = rect.height() / (frame.height as f32);

                    // 1. Center Crosshair
                    if self.show_center_spot {
                        let center_screen = Pos2::new(
                            rect.min.x + 160.0 * scale_x,
                            rect.min.y + 120.0 * scale_y,
                        );
                        draw_crosshair(ui.painter(), center_screen, Color32::GREEN, 10.0);
                        if let Some(telem) = &frame.telemetry {
                            let center_t = (telem.max_temp_c + telem.warn_temp_c) * 0.5;
                            ui.painter().text(
                                center_screen + Vec2::new(8.0, -12.0),
                                egui::Align2::LEFT_BOTTOM,
                                format!("C: {}", self.unit.format(center_t)),
                                egui::FontId::monospace(13.0),
                                Color32::GREEN,
                            );
                        }
                    }

                    // 2. Extreme Hot/Cold Spot Tracking
                    if self.show_extremes {
                        if let Some((max_pt, min_pt)) = find_extremes(frame) {
                            let max_screen = Pos2::new(
                                rect.min.x + (max_pt.0 as f32) * scale_x,
                                rect.min.y + (max_pt.1 as f32) * scale_y,
                            );
                            let min_screen = Pos2::new(
                                rect.min.x + (min_pt.0 as f32) * scale_x,
                                rect.min.y + (min_pt.1 as f32) * scale_y,
                            );

                            draw_crosshair(ui.painter(), max_screen, Color32::RED, 12.0);
                            draw_crosshair(ui.painter(), min_screen, Color32::from_rgb(100, 180, 255), 12.0);

                            if let Some(telem) = &frame.telemetry {
                                ui.painter().text(
                                    max_screen + Vec2::new(8.0, -12.0),
                                    egui::Align2::LEFT_BOTTOM,
                                    format!("H: {}", self.unit.format(telem.max_temp_c)),
                                    egui::FontId::monospace(13.0),
                                    Color32::from_rgb(255, 90, 90),
                                );
                                ui.painter().text(
                                    min_screen + Vec2::new(8.0, 12.0),
                                    egui::Align2::LEFT_TOP,
                                    format!("L: {}", self.unit.format(telem.warn_temp_c)),
                                    egui::FontId::monospace(13.0),
                                    Color32::from_rgb(100, 180, 255),
                                );
                            }
                        }
                    }

                    // 3. Hover Inspector
                    if let Some(mouse_pos) = response.hover_pos() {
                        let px = (((mouse_pos.x - rect.min.x) / scale_x) as u32).min(frame.width - 1);
                        let py = (((mouse_pos.y - rect.min.y) / scale_y) as u32).min(frame.height - 1);

                        // Draw pointer circle
                        ui.painter().circle_stroke(mouse_pos, 8.0, Stroke::new(1.5_f32, Color32::YELLOW));
                        ui.painter().text(
                            mouse_pos + Vec2::new(12.0, 12.0),
                            egui::Align2::LEFT_TOP,
                            format!("({}, {})", px, py),
                            egui::FontId::monospace(12.0),
                            Color32::YELLOW,
                        );
                    }

                    // 4. Colorbar gradient
                    if self.show_colorbar {
                        draw_colorbar(ui.painter(), rect, &self.palette, &frame.telemetry, self.unit);
                    }
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label("Waiting for video stream...");
                });
            }
        });

        // 5. Offline BMP Modal Window
        if let Some(bmp) = &self.offline_bmp {
            let mut close_modal = false;
            egui::Window::new("📁 Offline BMP Analysis")
                .default_size([640.0, 480.0])
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Captured Radiometric Image");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").clicked() {
                                close_modal = true;
                            }
                        });
                    });
                    ui.separator();

                    ui.columns(2, |cols| {
                        // Metadata column
                        cols[0].label(format!("Dimensions: {} x {}", bmp.width, bmp.height));
                        cols[0].label(format!("Temp Max: {:.2}°{}", bmp.temp_max, bmp.temp_units));
                        cols[0].label(format!("Temp Min: {:.2}°{}", bmp.temp_min, bmp.temp_units));
                        cols[0].label(format!("Center:   {:.2}°{}", bmp.temp_center, bmp.temp_units));
                        cols[0].label(format!("Emissivity: {:.2}", bmp.emissivity));
                        if let Some(ts) = bmp.timestamp {
                            cols[0].label(format!("Captured: {}", ts));
                        }

                        // Export actions
                        cols[1].vertical(|ui| {
                            if ui.button("💾 Export Clean PNG").clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("PNG", &["png"]).save_file() {
                                    let img = bmp.render_clean_image(Some(Palette::Iron));
                                    let _ = img.save(path);
                                }
                            }
                            if ui.button("📊 Export CSV Temperature Matrix").clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).save_file() {
                                    let csv = bmp.export_temperature_csv();
                                    let _ = std::fs::write(path, csv);
                                }
                            }
                        });
                    });
                });
            if close_modal {
                self.offline_bmp = None;
            }
        }

        // Request continuous repaint for smooth video streaming
        ctx.request_repaint();
    }
}

// Helper to draw crosshairs on image
fn draw_crosshair(painter: &egui::Painter, pos: Pos2, color: Color32, radius: f32) {
    let stroke = Stroke::new(2.0_f32, color);
    painter.line_segment([Pos2::new(pos.x - radius, pos.y), Pos2::new(pos.x + radius, pos.y)], stroke);
    painter.line_segment([Pos2::new(pos.x, pos.y - radius), Pos2::new(pos.x, pos.y + radius)], stroke);
    painter.circle_stroke(pos, radius * 0.5, stroke);
}

// Helper to find extreme luma (hot / cold) pixels
fn find_extremes(frame: &LiveFrame) -> Option<((u32, u32), (u32, u32))> {
    let width = frame.width;
    let height = frame.height;
    let yuyv = &frame.raw_data[..((width * height * 2) as usize)];

    let mut max_y = 0u8;
    let mut min_y = 255u8;
    let mut max_pos = (width / 2, height / 2);
    let mut min_pos = (width / 2, height / 2);

    for (chunk_idx, chunk) in yuyv.chunks_exact(4).enumerate() {
        let y0 = chunk[0];
        let y1 = chunk[2];
        let px = (chunk_idx * 2) as u32;
        let x0 = px % width;
        let y = px / width;
        let x1 = x0 + 1;

        if y0 > max_y { max_y = y0; max_pos = (x0, y); }
        if y0 < min_y { min_y = y0; min_pos = (x0, y); }
        if y1 > max_y { max_y = y1; max_pos = (x1, y); }
        if y1 < min_y { min_y = y1; min_pos = (x1, y); }
    }

    Some((max_pos, min_pos))
}

// Helper to draw a thermal colorbar scale on the right edge of the image
fn draw_colorbar(
    painter: &egui::Painter,
    rect: Rect,
    palette: &Palette,
    telemetry: &Option<Telemetry>,
    unit: TempUnit,
) {
    let bar_width = 14.0;
    let bar_height = rect.height() * 0.75;
    let bar_right = rect.max.x - 12.0;
    let bar_top = rect.min.y + (rect.height() - bar_height) * 0.5;

    let steps = 64;
    let step_h = bar_height / (steps as f32);

    for i in 0..steps {
        let val = 255 - ((i as f32 / steps as f32) * 255.0) as u8;
        let rgb = palette.map_color(val);
        let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        let step_rect = Rect::from_min_size(
            Pos2::new(bar_right - bar_width, bar_top + i as f32 * step_h),
            Vec2::new(bar_width, step_h + 1.0),
        );
        painter.rect_filled(step_rect, 0.0, color);
    }

    // Border around colorbar
    let full_bar = Rect::from_min_size(Pos2::new(bar_right - bar_width, bar_top), Vec2::new(bar_width, bar_height));
    painter.rect_stroke(
        full_bar,
        0.0,
        Stroke::new(1.0_f32, Color32::from_white_alpha(160)),
        egui::StrokeKind::Inside,
    );

    // Labels at top and bottom of colorbar
    if let Some(telem) = telemetry {
        painter.text(
            Pos2::new(bar_right - bar_width - 4.0, bar_top),
            egui::Align2::RIGHT_TOP,
            unit.format(telem.max_temp_c),
            egui::FontId::monospace(11.0),
            Color32::WHITE,
        );
        painter.text(
            Pos2::new(bar_right - bar_width - 4.0, bar_top + bar_height),
            egui::Align2::RIGHT_BOTTOM,
            unit.format(telem.warn_temp_c),
            egui::FontId::monospace(11.0),
            Color32::WHITE,
        );
    }
}

// Ext helper for ui
trait UiToastExt {
    fn info_label(&mut self, text: impl Into<String>);
}

impl UiToastExt for egui::Ui {
    fn info_label(&mut self, text: impl Into<String>) {
        self.group(|ui| {
            ui.colored_label(Color32::from_rgb(100, 200, 255), format!("ℹ {}", text.into()));
        });
    }
}
