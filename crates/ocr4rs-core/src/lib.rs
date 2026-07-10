//! # ocr4rs-core
//!
//! Motor de OCR de OCR4RS en **Rust puro** (sobre `ocrs`/`rten`, sin ONNX
//! Runtime ni MNN). Convierte una imagen (o página de PDF escaneado
//! rasterizada) en texto y líneas.
//!
//! Los modelos `.rten` se aportan en tiempo de ejecución (ver
//! [`engine::Ocr::from_model_dir`]); no se empotran en el binario.

pub mod engine;
pub mod error;

pub use engine::{Ocr, OcrLine, OcrOutput, DETECTION_MODEL, RECOGNITION_MODEL};
pub use error::{Error, Result};
