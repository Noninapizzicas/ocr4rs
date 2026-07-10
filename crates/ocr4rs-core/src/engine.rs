//! Motor de OCR sobre `ocrs` (puro Rust, sin runtime nativo).
//!
//! Carga dos modelos `.rten` (detección de texto + reconocimiento) desde un
//! directorio. Los modelos NO se incluyen en el binario: se aportan en
//! tiempo de ejecución (montados o descargados una vez). Sin ellos, el motor
//! no arranca y el llamador degrada con honestidad.

use std::path::Path;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use serde::Serialize;

use crate::error::{Error, Result};

/// Nombres de fichero esperados dentro del directorio de modelos.
pub const DETECTION_MODEL: &str = "text-detection.rten";
pub const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// Una línea de texto reconocida.
#[derive(Debug, Clone, Serialize)]
pub struct OcrLine {
    /// Texto de la línea.
    pub text: String,
}

/// Resultado de aplicar OCR a una imagen.
#[derive(Debug, Clone, Serialize)]
pub struct OcrOutput {
    /// Texto completo (líneas unidas por `\n`).
    pub text: String,
    /// Líneas individuales, en orden de lectura.
    pub lines: Vec<OcrLine>,
}

/// Motor de OCR listo para reconocer imágenes.
pub struct Ocr {
    engine: OcrEngine,
}

impl Ocr {
    /// Carga los modelos desde `dir` y prepara el motor.
    pub fn from_model_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let det_path = dir.join(DETECTION_MODEL);
        let rec_path = dir.join(RECOGNITION_MODEL);
        if !det_path.exists() || !rec_path.exists() {
            return Err(Error::ModelsUnavailable(format!(
                "faltan {DETECTION_MODEL} o {RECOGNITION_MODEL} en {}",
                dir.display()
            )));
        }

        let detection_model = Model::load_file(&det_path).map_err(|e| Error::ModelLoad {
            path: det_path.display().to_string(),
            source: Box::new(e),
        })?;
        let recognition_model = Model::load_file(&rec_path).map_err(|e| Error::ModelLoad {
            path: rec_path.display().to_string(),
            source: Box::new(e),
        })?;

        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        })
        .map_err(|e| Error::Ocr(e.to_string()))?;

        Ok(Self { engine })
    }

    /// Reconoce el texto de una imagen (bytes PNG/JPEG/WebP).
    pub fn recognize_bytes(&self, image_bytes: &[u8]) -> Result<OcrOutput> {
        let img = image::load_from_memory(image_bytes).map_err(|e| Error::Image(e.to_string()))?;
        self.recognize_image(&img)
    }

    /// Reconoce el texto de una imagen ya decodificada (p. ej. tras el
    /// pre-proceso o extraída de un PDF). El pre-proceso de OCR4RS produce
    /// grises; aquí se convierte a RGB para el reconocedor.
    pub fn recognize_image(&self, img: &image::DynamicImage) -> Result<OcrOutput> {
        let img = img.to_rgb8();
        let (w, h) = img.dimensions();
        let source = ImageSource::from_bytes(img.as_raw(), (w, h))
            .map_err(|e| Error::Image(e.to_string()))?;

        let input = self
            .engine
            .prepare_input(source)
            .map_err(|e| Error::Ocr(e.to_string()))?;
        let words = self
            .engine
            .detect_words(&input)
            .map_err(|e| Error::Ocr(e.to_string()))?;
        let lines = self.engine.find_text_lines(&input, &words);
        let recognized = self
            .engine
            .recognize_text(&input, &lines)
            .map_err(|e| Error::Ocr(e.to_string()))?;

        let lines: Vec<OcrLine> = recognized
            .iter()
            .flatten()
            .map(|line| OcrLine {
                text: line.to_string(),
            })
            .filter(|l| !l.text.trim().is_empty())
            .collect();
        let text = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(OcrOutput { text, lines })
    }
}
