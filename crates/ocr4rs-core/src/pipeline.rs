//! Orquestación del órgano: de bytes crudos a texto, haciendo TODO el camino.
//!
//! ```text
//! bytes ──▶ detectar tipo
//!            ├─ imagen  ──────────────▶ preparar ─▶ reconocer
//!            └─ PDF escaneado ─▶ extraer ráster por página ─▶ preparar ─▶ reconocer
//!               (PDF digital ─▶ error EsDigital → usa crawl4rs)
//! ```
//!
//! El pre-proceso vive DENTRO porque preparar la imagen *es* parte de hacer
//! OCR bien — igual que `fit_markdown` es parte de hacer crawling bien.

use ocr4rs_pdf::PdfError;
use ocr4rs_prep::{preparar, PrepConfig, PrepReport};
use serde::Serialize;

use crate::engine::{Ocr, OcrLine};
use crate::error::{Error, Result};

/// Configuración del procesamiento de una página cruda.
#[derive(Debug, Clone, Default)]
pub struct ProcessConfig {
    /// Configuración del pipeline de pre-proceso.
    pub prep: PrepConfig,
}

/// Origen detectado de la entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Imagen suelta (PNG/JPEG/WebP).
    Image,
    /// PDF escaneado (una o más páginas rasterizadas).
    ScannedPdf,
}

/// Texto reconocido de una página.
#[derive(Debug, Clone, Serialize)]
pub struct PageOutput {
    /// Número de página (0-based).
    pub n: usize,
    /// Texto de la página.
    pub text: String,
    /// Líneas en orden de lectura.
    pub lines: Vec<OcrLine>,
    /// Pasos de pre-proceso aplicados a esta página.
    pub prep: PrepReport,
}

/// Resultado del órgano completo.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessOutput {
    /// Texto de todas las páginas, unido por saltos dobles.
    pub text: String,
    /// Detalle por página.
    pub pages: Vec<PageOutput>,
    /// Origen detectado.
    pub source_kind: SourceKind,
}

/// Detecta el tipo de `bytes` sin depender de la extensión.
fn detectar(bytes: &[u8]) -> Result<SourceKind> {
    if bytes.starts_with(b"%PDF") {
        return Ok(SourceKind::ScannedPdf);
    }
    if image::guess_format(bytes).is_ok() {
        return Ok(SourceKind::Image);
    }
    Err(Error::UnsupportedInput(
        "ni imagen reconocible ni PDF".into(),
    ))
}

impl Ocr {
    /// Procesa bytes crudos (imagen o PDF escaneado) de punta a punta.
    ///
    /// - Imagen → preparar → reconocer.
    /// - PDF escaneado → extraer el ráster de cada página → preparar → reconocer.
    /// - PDF digital → [`Error::IsDigitalPdf`] (es trabajo de Crawl4RS).
    pub fn process(&self, bytes: &[u8], cfg: &ProcessConfig) -> Result<ProcessOutput> {
        let kind = detectar(bytes)?;
        let paginas = match kind {
            SourceKind::Image => {
                let img =
                    image::load_from_memory(bytes).map_err(|e| Error::Image(e.to_string()))?;
                vec![img]
            }
            SourceKind::ScannedPdf => ocr4rs_pdf::extraer_paginas(bytes).map_err(map_pdf_err)?,
        };

        let mut pages = Vec::with_capacity(paginas.len());
        for (n, img) in paginas.iter().enumerate() {
            let (limpia, prep) = preparar(img, &cfg.prep);
            let out = self.recognize_image(&image::DynamicImage::ImageLuma8(limpia))?;
            pages.push(PageOutput {
                n,
                text: out.text,
                lines: out.lines,
                prep,
            });
        }

        let text = pages
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(ProcessOutput {
            text,
            pages,
            source_kind: kind,
        })
    }
}

/// Prepara la **primera página** de la entrada SIN reconocer (para depurar el
/// pipeline: no necesita modelos). Devuelve la imagen limpia y el informe.
pub fn preparar_primera_pagina(
    bytes: &[u8],
    cfg: &PrepConfig,
) -> Result<(image::GrayImage, PrepReport)> {
    let kind = detectar(bytes)?;
    let img = match kind {
        SourceKind::Image => {
            image::load_from_memory(bytes).map_err(|e| Error::Image(e.to_string()))?
        }
        SourceKind::ScannedPdf => {
            let mut pags = ocr4rs_pdf::extraer_paginas(bytes).map_err(map_pdf_err)?;
            if pags.is_empty() {
                return Err(Error::UnsupportedInput("el PDF no tiene páginas".into()));
            }
            pags.remove(0)
        }
    };
    Ok(preparar(&img, cfg))
}

fn map_pdf_err(e: PdfError) -> Error {
    match e {
        PdfError::EsDigital => Error::IsDigitalPdf,
        PdfError::FiltroNoSoportado(f) => {
            Error::UnsupportedInput(format!("filtro de imagen no soportado: {f}"))
        }
        PdfError::SinImagenes => {
            Error::UnsupportedInput("el PDF no contiene páginas rasterizadas".into())
        }
        otro => Error::Pdf(otro.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecta_pdf_por_cabecera() {
        assert_eq!(detectar(b"%PDF-1.5\n...").unwrap(), SourceKind::ScannedPdf);
    }

    #[test]
    fn detecta_imagen_png() {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image::GrayImage::new(4, 4))
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        assert_eq!(detectar(&buf.into_inner()).unwrap(), SourceKind::Image);
    }

    #[test]
    fn rechaza_basura() {
        assert!(matches!(
            detectar(b"no soy ni imagen ni pdf"),
            Err(Error::UnsupportedInput(_))
        ));
    }
}
