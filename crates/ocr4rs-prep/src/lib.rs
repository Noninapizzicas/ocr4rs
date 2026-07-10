//! # ocr4rs-prep
//!
//! Pre-proceso de imagen para OCR — **Rust puro** (`image` + `imageproc`),
//! sin modelos ni runtime nativo. Prepara una página cruda (foto torcida,
//! ráster de escaneo) para que el reconocedor acierte más.
//!
//! El principio: **mejora, no inventa**. Cada paso sube la legibilidad de lo
//! que ya está; si la entrada viene limpia, el pipeline se gradúa y hace poco.
//!
//! ## Sobre la binarización
//!
//! El pipeline clásico de OCR termina en blanco/negro (Otsu/Sauvola) porque
//! los motores tradicionales lo piden. `ocrs` es un reconocedor **neuronal**
//! entrenado sobre grises con antialias: binarizar duro puede *bajar* el
//! acierto. Por eso la binarización se ofrece pero viene **apagada por
//! defecto**; se activa y se calibra con datos reales.

use image::{DynamicImage, GrayImage, Luma};
use serde::{Deserialize, Serialize};

/// Modo de binarización final.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Binarize {
    /// Sin binarizar — se conserva la escala de grises (recomendado para `ocrs`).
    #[default]
    None,
    /// Umbral global de Otsu.
    Otsu,
    /// Umbral adaptativo por bloques (mejor con sombras de foto de móvil).
    Adaptive,
}

/// Configuración del pipeline. Cada paso es opcional y graduable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrepConfig {
    /// Estirar el contraste (auto-niveles) — sube el rango dinámico de una
    /// foto plana.
    pub normalize: bool,
    /// Corregir la inclinación de la página. El paso que más acierto gana en
    /// escaneos torcidos.
    pub deskew: bool,
    /// Ángulo máximo de corrección de inclinación, en grados.
    pub max_skew_deg: f32,
    /// Filtro de mediana para quitar grano de sensor. Suave, pero puede comer
    /// texto muy pequeño: apagado por defecto.
    pub denoise: bool,
    /// Binarización final. `None` por defecto (ver nota del módulo).
    pub binarize: Binarize,
    /// Si el lado mayor es menor que este umbral (px), escalar ×2 (Lanczos).
    /// `None` = no escalar. El reconocedor pide una altura de línea mínima.
    pub upscale_min_dim: Option<u32>,
    /// Recortar el marco vacío/negro del escaneo alrededor del contenido.
    pub trim_border: bool,
}

impl Default for PrepConfig {
    fn default() -> Self {
        // Por defecto: lo que casi siempre ayuda y nunca destruye —
        // grises + normalizar + deskew. El resto, bajo demanda.
        Self {
            normalize: true,
            deskew: true,
            max_skew_deg: 8.0,
            denoise: false,
            binarize: Binarize::None,
            upscale_min_dim: None,
            trim_border: false,
        }
    }
}

impl PrepConfig {
    /// Pipeline "crudo": no toca nada (sólo convierte a grises). Útil para
    /// medir cuánto aporta el pre-proceso.
    pub fn passthrough() -> Self {
        Self {
            normalize: false,
            deskew: false,
            max_skew_deg: 0.0,
            denoise: false,
            binarize: Binarize::None,
            upscale_min_dim: None,
            trim_border: false,
        }
    }
}

/// Qué pasos se aplicaron de verdad (para poder auditar el resultado).
#[derive(Debug, Clone, Default, Serialize)]
pub struct PrepReport {
    /// Pasos aplicados, en orden.
    pub steps: Vec<String>,
    /// Ángulo de deskew corregido (grados), si se aplicó.
    pub skew_deg: Option<f32>,
}

/// Prepara `img` según `cfg`. Devuelve la imagen en grises lista para OCR y un
/// informe de lo aplicado. Trabaja siempre en el dominio de grises: el color
/// no aporta al OCR y triplica el trabajo.
pub fn preparar(img: &DynamicImage, cfg: &PrepConfig) -> (GrayImage, PrepReport) {
    let mut report = PrepReport::default();
    let mut gray = img.to_luma8();
    report.steps.push("grayscale".into());

    if cfg.normalize {
        auto_levels(&mut gray);
        report.steps.push("normalize".into());
    }

    if cfg.deskew {
        let angle = estimar_inclinacion(&gray, cfg.max_skew_deg);
        // Un giro por debajo de ~0.3° no compensa el coste ni el suavizado.
        if angle.abs() >= 0.3 {
            gray = rotar(&gray, angle);
            report.skew_deg = Some(angle);
            report.steps.push("deskew".into());
        }
    }

    if cfg.denoise {
        gray = imageproc::filter::median_filter(&gray, 1, 1);
        report.steps.push("denoise".into());
    }

    if let Some(min_dim) = cfg.upscale_min_dim {
        let (w, h) = gray.dimensions();
        if w.max(h) < min_dim {
            gray = image::imageops::resize(
                &gray,
                w.saturating_mul(2),
                h.saturating_mul(2),
                image::imageops::FilterType::Lanczos3,
            );
            report.steps.push("upscale2x".into());
        }
    }

    match cfg.binarize {
        Binarize::None => {}
        Binarize::Otsu => {
            let level = imageproc::contrast::otsu_level(&gray);
            umbral(&mut gray, level);
            report.steps.push("binarize:otsu".into());
        }
        Binarize::Adaptive => {
            // Radio de bloque proporcional al tamaño; mínimo razonable.
            let (w, h) = gray.dimensions();
            let radius = (w.min(h) / 32).clamp(8, 64);
            gray = imageproc::contrast::adaptive_threshold(&gray, radius);
            report.steps.push("binarize:adaptive".into());
        }
    }

    if cfg.trim_border {
        if let Some(cropped) = recortar_contenido(&gray) {
            gray = cropped;
            report.steps.push("trim".into());
        }
    }

    (gray, report)
}

/// Auto-niveles: estira el histograma entre los percentiles ~2% y ~98% para no
/// dejar que unos pocos píxeles extremos aplasten el contraste.
fn auto_levels(gray: &mut GrayImage) {
    let mut hist = [0u32; 256];
    for p in gray.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total: u32 = gray.width() * gray.height();
    if total == 0 {
        return;
    }
    let cut = (total as f32 * 0.02) as u32;

    let mut acc = 0u32;
    let mut lo = 0u8;
    for (i, &c) in hist.iter().enumerate() {
        acc += c;
        if acc > cut {
            lo = i as u8;
            break;
        }
    }
    acc = 0;
    let mut hi = 255u8;
    for i in (0..256).rev() {
        acc += hist[i];
        if acc > cut {
            hi = i as u8;
            break;
        }
    }
    if hi <= lo {
        return;
    }
    let range = (hi - lo) as f32;
    for p in gray.pixels_mut() {
        let v = p.0[0];
        let stretched = ((v.saturating_sub(lo)) as f32 / range * 255.0).clamp(0.0, 255.0);
        p.0[0] = stretched as u8;
    }
}

/// Umbral global in-place: tinta (≤level) a negro, fondo a blanco.
fn umbral(gray: &mut GrayImage, level: u8) {
    for p in gray.pixels_mut() {
        p.0[0] = if p.0[0] <= level { 0 } else { 255 };
    }
}

/// Estima el ángulo de inclinación (grados) por perfil de proyección: el
/// ángulo correcto es el que más "apila" la tinta en filas — máxima varianza
/// del número de píxeles oscuros por fila. Se trabaja sobre una copia pequeña
/// y binarizada por velocidad.
fn estimar_inclinacion(gray: &GrayImage, max_deg: f32) -> f32 {
    if max_deg <= 0.0 {
        return 0.0;
    }
    // Reducir a ~600 px de ancho para acelerar los giros de prueba.
    let (w, h) = gray.dimensions();
    let small = if w > 600 {
        let nh = (h as f32 * 600.0 / w as f32).round().max(1.0) as u32;
        image::imageops::resize(gray, 600, nh, image::imageops::FilterType::Triangle)
    } else {
        gray.clone()
    };
    let level = imageproc::contrast::otsu_level(&small);
    let mut bin = small;
    umbral(&mut bin, level);

    // Búsqueda gruesa (1°) y luego fina (0.2°) alrededor del mejor.
    let coarse = mejor_angulo(&bin, -max_deg, max_deg, 1.0);
    mejor_angulo(&bin, coarse - 1.0, coarse + 1.0, 0.2)
}

fn mejor_angulo(bin: &GrayImage, from: f32, to: f32, step: f32) -> f32 {
    let mut best_angle = 0.0f32;
    let mut best_score = f32::MIN;
    let mut a = from;
    while a <= to + f32::EPSILON {
        let score = puntuacion_proyeccion(&rotar(bin, a));
        if score > best_score {
            best_score = score;
            best_angle = a;
        }
        a += step;
    }
    best_angle
}

/// Varianza del recuento de tinta por fila (mayor = líneas más horizontales).
fn puntuacion_proyeccion(bin: &GrayImage) -> f32 {
    let (w, h) = bin.dimensions();
    if h == 0 {
        return 0.0;
    }
    let mut rows = vec![0u32; h as usize];
    for y in 0..h {
        let mut ink = 0u32;
        for x in 0..w {
            if bin.get_pixel(x, y).0[0] < 128 {
                ink += 1;
            }
        }
        rows[y as usize] = ink;
    }
    let n = rows.len() as f32;
    let mean = rows.iter().map(|&r| r as f32).sum::<f32>() / n;
    rows.iter()
        .map(|&r| {
            let d = r as f32 - mean;
            d * d
        })
        .sum::<f32>()
        / n
}

/// Gira la imagen `deg` grados alrededor del centro, fondo blanco.
fn rotar(gray: &GrayImage, deg: f32) -> GrayImage {
    let theta = deg.to_radians();
    imageproc::geometric_transformations::rotate_about_center(
        gray,
        theta,
        imageproc::geometric_transformations::Interpolation::Bilinear,
        Luma([255]),
    )
}

/// Recorta al bounding box del contenido (tinta), con un pequeño margen. Usa
/// Otsu para decidir qué es tinta. Devuelve `None` si no encuentra contenido.
fn recortar_contenido(gray: &GrayImage) -> Option<GrayImage> {
    let (w, h) = gray.dimensions();
    let level = imageproc::contrast::otsu_level(gray);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0u32, 0u32);
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            if gray.get_pixel(x, y).0[0] <= level {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if !found {
        return None;
    }
    let margin = 8;
    let x0 = min_x.saturating_sub(margin);
    let y0 = min_y.saturating_sub(margin);
    let x1 = (max_x + margin).min(w - 1);
    let y1 = (max_y + margin).min(h - 1);
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    // Si el recorte no quita casi nada, no vale la pena copiar.
    if cw >= w && ch >= h {
        return None;
    }
    Some(image::imageops::crop_imm(gray, x0, y0, cw, ch).to_image())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    /// Genera una imagen con bandas horizontales de tinta, girada `deg` grados.
    fn pagina_con_lineas(deg: f32) -> GrayImage {
        let mut img = GrayImage::from_pixel(400, 300, Luma([255]));
        for y in (20..280).step_by(20) {
            for x in 20..380 {
                img.put_pixel(x, y, Luma([0]));
                img.put_pixel(x, y + 1, Luma([0]));
            }
        }
        if deg != 0.0 {
            rotar(&img, deg)
        } else {
            img
        }
    }

    #[test]
    fn passthrough_solo_convierte_a_grises() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(10, 10, image::Rgb([120, 30, 200])));
        let (out, report) = preparar(&img, &PrepConfig::passthrough());
        assert_eq!(out.dimensions(), (10, 10));
        assert_eq!(report.steps, vec!["grayscale"]);
        assert!(report.skew_deg.is_none());
    }

    #[test]
    fn deskew_detecta_inclinacion() {
        // Página girada +5°: el estimador debe encontrar ~ -5° para enderezarla.
        let torcida = DynamicImage::ImageLuma8(pagina_con_lineas(5.0));
        let cfg = PrepConfig {
            normalize: false,
            deskew: true,
            max_skew_deg: 8.0,
            ..PrepConfig::passthrough()
        };
        let (_out, report) = preparar(&torcida, &cfg);
        let angle = report.skew_deg.expect("debería detectar inclinación");
        assert!(
            (angle + 5.0).abs() < 1.5,
            "ángulo estimado {angle}, esperado ~ -5"
        );
    }

    #[test]
    fn pagina_recta_no_gira_mucho() {
        let recta = DynamicImage::ImageLuma8(pagina_con_lineas(0.0));
        let (_out, report) = preparar(&recta, &PrepConfig::default());
        // Una página ya recta no debería inventarse un giro grande.
        if let Some(a) = report.skew_deg {
            assert!(a.abs() < 1.5, "giró {a}° una página recta");
        }
    }

    #[test]
    fn binarizar_otsu_deja_blanco_y_negro() {
        let img = DynamicImage::ImageLuma8(pagina_con_lineas(0.0));
        let cfg = PrepConfig {
            binarize: Binarize::Otsu,
            deskew: false,
            normalize: false,
            ..PrepConfig::passthrough()
        };
        let (out, report) = preparar(&img, &cfg);
        assert!(report.steps.iter().any(|s| s == "binarize:otsu"));
        assert!(out.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255));
    }
}
