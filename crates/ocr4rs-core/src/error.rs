//! Tipos de error del núcleo.

use thiserror::Error;

/// Alias de resultado de la crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errores del motor de OCR.
#[derive(Debug, Error)]
pub enum Error {
    /// No se encontraron los modelos (degradación honesta hacia el llamador).
    #[error("modelos OCR no disponibles: {0}")]
    ModelsUnavailable(String),

    /// Fallo cargando un modelo.
    #[error("fallo al cargar el modelo {path}: {source}")]
    ModelLoad {
        /// Ruta del modelo.
        path: String,
        /// Causa subyacente.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Fallo decodificando la imagen de entrada.
    #[error("imagen inválida: {0}")]
    Image(String),

    /// Fallo durante la inferencia OCR.
    #[error("fallo de OCR: {0}")]
    Ocr(String),

    /// El PDF trae capa de texto: es digital, no escaneado → usar Crawl4RS.
    #[error("el PDF es digital (tiene capa de texto): usa crawl4rs, no OCR")]
    IsDigitalPdf,

    /// Entrada de formato no soportado (ni imagen ni PDF escaneado legible).
    #[error("entrada no soportada: {0}")]
    UnsupportedInput(String),

    /// Fallo procesando el PDF de entrada.
    #[error("fallo de PDF: {0}")]
    Pdf(String),
}
