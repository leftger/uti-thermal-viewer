use thiserror::Error;

/// Errors that can occur when communicating with or parsing data from a UTi-260B camera.
#[derive(Debug, Error)]
pub enum Error {
    #[error("No UTi-260B camera found on system")]
    CameraNotFound,

    #[error("Capture error: {0}")]
    Capture(String),

    #[error("Frame data too small: expected at least {expected} bytes, got {actual} bytes")]
    FrameTooSmall { expected: usize, actual: usize },

    #[error("Invalid BMP format: {0}")]
    InvalidBmp(String),

    #[error("Camera driver error: {0}")]
    Nokhwa(#[from] nokhwa::NokhwaError),

    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
