use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use image::{imageops::FilterType, DynamicImage, ImageFormat};
use serde::Serialize;
use std::io::Cursor;
use thiserror::Error;
use xcap::Monitor;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("xcap: {0}")]
    XCap(String),
    #[error("encode: {0}")]
    Encode(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDescription {
    pub index: usize,
    pub id: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedScreen {
    pub index: usize,
    pub jpeg_base64: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

fn sort_monitors(mut monitors: Vec<Monitor>) -> Vec<Monitor> {
    monitors.sort_by(|monitor_a, monitor_b| {
        let primary_ord = monitor_b.is_primary().cmp(&monitor_a.is_primary());
        primary_ord
            .then_with(|| (monitor_a.x(), monitor_a.y()).cmp(&(monitor_b.x(), monitor_b.y())))
    });
    monitors
}

pub fn enumerate_monitors() -> Result<Vec<MonitorDescription>, CaptureError> {
    let monitors = Monitor::all().map_err(|e| CaptureError::XCap(e.to_string()))?;
    let monitors = sort_monitors(monitors);
    Ok(monitors
        .into_iter()
        .enumerate()
        .map(|(index, monitor)| MonitorDescription {
            index,
            id: monitor.id(),
            name: monitor.name().to_string(),
            x: monitor.x(),
            y: monitor.y(),
            width: monitor.width(),
            height: monitor.height(),
            scale_factor: f64::from(monitor.scale_factor()),
            is_primary: monitor.is_primary(),
        })
        .collect())
}

const MAX_CAPTURE_WIDTH: u32 = 1280;

pub fn capture_all_screens() -> Result<Vec<CapturedScreen>, CaptureError> {
    let monitors = Monitor::all().map_err(|e| CaptureError::XCap(e.to_string()))?;
    let monitors = sort_monitors(monitors);
    let mut out = Vec::new();
    for (index, monitor) in monitors.into_iter().enumerate() {
        let rgba = monitor
            .capture_image()
            .map_err(|e| CaptureError::XCap(e.to_string()))?;
        let mut dynamic_image = DynamicImage::ImageRgba8(rgba);
        let capture_width = dynamic_image.width();
        if capture_width > MAX_CAPTURE_WIDTH {
            let ratio = MAX_CAPTURE_WIDTH as f32 / capture_width as f32;
            let new_h = ((dynamic_image.height() as f32 * ratio).round() as u32).max(1);
            dynamic_image = dynamic_image.resize(MAX_CAPTURE_WIDTH, new_h, FilterType::Triangle);
        }
        let mut cursor = Cursor::new(Vec::new());
        dynamic_image
            .write_to(&mut cursor, ImageFormat::Jpeg)
            .map_err(|e| CaptureError::Encode(e.to_string()))?;
        let jpeg_base64 = B64.encode(cursor.into_inner());

        out.push(CapturedScreen {
            index,
            jpeg_base64,
            x: monitor.x(),
            y: monitor.y(),
            width: monitor.width(),
            height: monitor.height(),
            scale_factor: f64::from(monitor.scale_factor()),
        });
    }
    Ok(out)
}
