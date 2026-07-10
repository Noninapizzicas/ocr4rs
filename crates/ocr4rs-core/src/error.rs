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
}
