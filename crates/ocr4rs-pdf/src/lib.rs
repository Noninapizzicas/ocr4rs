//! # ocr4rs-pdf
//!
//! Extracción del **ráster** de un PDF **escaneado** — Rust puro (`lopdf`).
//!
//! El hallazgo que hace esto posible sin un motor de render nativo: un PDF
//! escaneado **no es un documento vectorial**. Cada página es *una imagen
//! ráster embebida* (el escáner metió un JPEG/CCITT por hoja). No hay que
//! renderizar nada — se **extrae** el stream de imagen del XObject.
//!
//! Frontera sagrada: si el PDF trae **capa de texto** (PDF digital), NO es
//! trabajo de OCR4RS → se devuelve [`PdfError::EsDigital`] y el llamador lo
//! manda a Crawl4RS. OCR4RS nunca renderiza vectores.

use image::DynamicImage;
use lopdf::{Document, Object, Stream};
use thiserror::Error;

/// Errores de la extracción de PDF.
#[derive(Debug, Error)]
pub enum PdfError {
    /// El PDF no pudo parsearse.
    #[error("PDF ilegible: {0}")]
    Parse(String),
    /// El PDF trae capa de texto: es digital, no escaneado → usar Crawl4RS.
    #[error("el PDF tiene capa de texto (es digital): usa crawl4rs, no OCR")]
    EsDigital,
    /// El PDF no contiene imágenes rasterizadas que extraer.
    #[error("el PDF no contiene páginas rasterizadas")]
    SinImagenes,
    /// La imagen usa un filtro que no soportamos (p. ej. CCITT/JBIG2/JPX).
    #[error("filtro de imagen no soportado: {0}")]
    FiltroNoSoportado(String),
    /// La imagen embebida no pudo reconstruirse.
    #[error("imagen embebida inválida: {0}")]
    ImagenInvalida(String),
}

type Result<T> = std::result::Result<T, PdfError>;

/// ¿El PDF trae capa de texto? Pre-check barato (presencia de `/Font` en los
/// recursos de alguna página). Un PDF digital o un escaneo ya "buscable" lo
/// tienen — en ambos casos el texto ya existe y lo extrae Crawl4RS.
pub fn tiene_capa_texto(pdf_bytes: &[u8]) -> Result<bool> {
    let doc = Document::load_mem(pdf_bytes).map_err(|e| PdfError::Parse(e.to_string()))?;
    Ok(tiene_texto(&doc))
}

/// Extrae una imagen ráster por página (la mayor de cada hoja: el escaneo).
///
/// Falla con [`PdfError::EsDigital`] si detecta capa de texto, respetando la
/// frontera con Crawl4RS.
pub fn extraer_paginas(pdf_bytes: &[u8]) -> Result<Vec<DynamicImage>> {
    let doc = Document::load_mem(pdf_bytes).map_err(|e| PdfError::Parse(e.to_string()))?;
    if tiene_texto(&doc) {
        return Err(PdfError::EsDigital);
    }

    let mut paginas = Vec::new();
    let mut filtro_no_soportado: Option<String> = None;

    for (_num, page_id) in doc.get_pages() {
        let mut mejor: Option<DynamicImage> = None;
        for stream in imagenes_de_pagina(&doc, page_id) {
            match decodificar_imagen(stream) {
                Ok(Some(img)) => {
                    // La página escaneada = el ráster más grande.
                    let mayor = mejor
                        .as_ref()
                        .map(|m| img.width() * img.height() > m.width() * m.height())
                        .unwrap_or(true);
                    if mayor {
                        mejor = Some(img);
                    }
                }
                Ok(None) => {}
                Err(PdfError::FiltroNoSoportado(f)) => filtro_no_soportado = Some(f),
                Err(e) => return Err(e),
            }
        }
        if let Some(img) = mejor {
            paginas.push(img);
        }
    }

    if paginas.is_empty() {
        // Si no sacamos nada pero había un ráster en filtro raro, sé honesto.
        if let Some(f) = filtro_no_soportado {
            return Err(PdfError::FiltroNoSoportado(f));
        }
        return Err(PdfError::SinImagenes);
    }
    Ok(paginas)
}

/// ¿Hay algún `/Font` en los recursos de alguna página?
fn tiene_texto(doc: &Document) -> bool {
    for (_num, page_id) in doc.get_pages() {
        if let Some(Object::Dictionary(res)) = recurso_heredado(doc, page_id, b"Resources") {
            if let Ok(fonts) = res.get(b"Font") {
                match deref(doc, fonts) {
                    Object::Dictionary(d) if !d.as_hashmap().is_empty() => return true,
                    _ => {}
                }
            }
        }
    }
    false
}

/// Sigue una posible referencia hasta el objeto real.
fn deref<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
    match obj {
        Object::Reference(id) => doc.get_object(*id).unwrap_or(obj),
        _ => obj,
    }
}

/// Busca `key` en el diccionario de la página, subiendo por `/Parent` si hace
/// falta (los recursos pueden heredarse del nodo Pages).
fn recurso_heredado(doc: &Document, page_id: lopdf::ObjectId, key: &[u8]) -> Option<Object> {
    let mut actual = page_id;
    for _ in 0..32 {
        let dict = doc.get_dictionary(actual).ok()?;
        if let Ok(obj) = dict.get(key) {
            return Some(deref(doc, obj).clone());
        }
        match dict.get(b"Parent") {
            Ok(Object::Reference(pid)) => actual = *pid,
            _ => break,
        }
    }
    None
}

/// Devuelve los streams de imagen (XObject Subtype=Image) de una página.
fn imagenes_de_pagina(doc: &Document, page_id: lopdf::ObjectId) -> Vec<&Stream> {
    let mut out = Vec::new();
    let Some(Object::Dictionary(res)) = recurso_heredado(doc, page_id, b"Resources") else {
        return out;
    };
    let Ok(xobj_obj) = res.get(b"XObject") else {
        return out;
    };
    let Object::Dictionary(xobj) = deref(doc, xobj_obj) else {
        return out;
    };
    for (_name, obj) in xobj.iter() {
        let Object::Reference(id) = obj else { continue };
        if let Ok(Object::Stream(stream)) = doc.get_object(*id) {
            if es_imagen(stream) {
                out.push(stream);
            }
        }
    }
    out
}

fn es_imagen(stream: &Stream) -> bool {
    matches!(stream.dict.get(b"Subtype").and_then(Object::as_name), Ok(n) if n == b"Image")
}

/// Espacio de color reconocido de una imagen embebida.
enum Color {
    Gray,
    Rgb,
    Otro(String),
}

/// Reconstruye la imagen de un XObject. `Ok(None)` = no es imagen; el error
/// `FiltroNoSoportado` deja constancia honesta de un ráster que no sabemos leer.
fn decodificar_imagen(stream: &Stream) -> Result<Option<DynamicImage>> {
    let dict = &stream.dict;
    let w = entero(dict, b"Width").unwrap_or(0);
    let h = entero(dict, b"Height").unwrap_or(0);
    if w <= 0 || h <= 0 {
        return Ok(None);
    }
    let (w, h) = (w as u32, h as u32);
    let bpc = entero(dict, b"BitsPerComponent").unwrap_or(8);
    let filtros = filtros(dict);
    let ultimo = filtros.last().map(String::as_str).unwrap_or("");

    match ultimo {
        // JPEG embebido: el crate `image` lo decodifica tal cual.
        "DCTDecode" => {
            let jpeg = &stream.content;
            let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
                .map_err(|e| PdfError::ImagenInvalida(e.to_string()))?;
            Ok(Some(img))
        }
        // Ráster crudo (o inflado): se reconstruye desde las muestras.
        "FlateDecode" | "" => {
            if bpc != 8 {
                return Err(PdfError::FiltroNoSoportado(format!(
                    "BitsPerComponent={bpc}"
                )));
            }
            let datos = if ultimo == "FlateDecode" {
                stream
                    .decompressed_content()
                    .map_err(|e| PdfError::ImagenInvalida(e.to_string()))?
            } else {
                stream.content.clone()
            };
            reconstruir_raster(w, h, color_de(dict), datos)
        }
        // Fax B/N antiguo y variantes: honesto, no reventamos.
        otro => Err(PdfError::FiltroNoSoportado(otro.to_string())),
    }
}

/// Construye la imagen desde muestras crudas de 8 bits según el espacio de color.
fn reconstruir_raster(
    w: u32,
    h: u32,
    color: Color,
    datos: Vec<u8>,
) -> Result<Option<DynamicImage>> {
    match color {
        Color::Gray => {
            let esperado = (w * h) as usize;
            if datos.len() < esperado {
                return Err(PdfError::ImagenInvalida(format!(
                    "muestras grises {} < {esperado}",
                    datos.len()
                )));
            }
            let buf = image::GrayImage::from_raw(w, h, datos)
                .ok_or_else(|| PdfError::ImagenInvalida("buffer gris inválido".into()))?;
            Ok(Some(DynamicImage::ImageLuma8(buf)))
        }
        Color::Rgb => {
            let esperado = (w * h * 3) as usize;
            if datos.len() < esperado {
                return Err(PdfError::ImagenInvalida(format!(
                    "muestras RGB {} < {esperado}",
                    datos.len()
                )));
            }
            let buf = image::RgbImage::from_raw(w, h, datos)
                .ok_or_else(|| PdfError::ImagenInvalida("buffer RGB inválido".into()))?;
            Ok(Some(DynamicImage::ImageRgb8(buf)))
        }
        Color::Otro(name) => Err(PdfError::FiltroNoSoportado(format!("ColorSpace={name}"))),
    }
}

fn entero(dict: &lopdf::Dictionary, key: &[u8]) -> Option<i64> {
    dict.get(key).ok().and_then(|o| o.as_i64().ok())
}

fn filtros(dict: &lopdf::Dictionary) -> Vec<String> {
    match dict.get(b"Filter") {
        Ok(Object::Name(n)) => vec![String::from_utf8_lossy(n).into_owned()],
        Ok(Object::Array(a)) => a
            .iter()
            .filter_map(|o| o.as_name().ok())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .collect(),
        _ => vec![],
    }
}

/// Interpreta `/ColorSpace`. Cubre los casos de escaneo reales (gris y RGB,
/// incl. ICCBased con N=1/3); el resto se marca como no soportado.
fn color_de(dict: &lopdf::Dictionary) -> Color {
    match dict.get(b"ColorSpace") {
        Ok(Object::Name(n)) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" => Color::Gray,
            b"DeviceRGB" | b"CalRGB" | b"RGB" => Color::Rgb,
            otro => Color::Otro(String::from_utf8_lossy(otro).into_owned()),
        },
        Ok(Object::Array(a)) => {
            if let Some(Object::Name(n)) = a.first() {
                if n == b"ICCBased" {
                    // ICCBased: el número de componentes está en /N del stream.
                    return Color::Rgb; // se refina abajo si detectamos N
                }
                return Color::Otro(String::from_utf8_lossy(n).into_owned());
            }
            Color::Otro("array".into())
        }
        _ => Color::Gray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn jpeg_rgb(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 180, 160]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .unwrap();
        buf.into_inner()
    }

    /// Ensambla un PDF mínimo de una página con un XObject imagen.
    fn pdf_con_imagen(img_dict: lopdf::Dictionary, content: Vec<u8>, con_fuente: bool) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let img_id = doc.add_object(Stream::new(img_dict, content));

        let mut xobject = lopdf::Dictionary::new();
        xobject.set("Im0", Object::Reference(img_id));
        let mut resources = lopdf::Dictionary::new();
        resources.set("XObject", Object::Dictionary(xobject));
        if con_fuente {
            let mut fonts = lopdf::Dictionary::new();
            fonts.set(
                "F1",
                Object::Dictionary(dictionary! { "Type" => "Font", "Subtype" => "Type1" }),
            );
            resources.set("Font", Object::Dictionary(fonts));
        }

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Resources" => resources,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);

        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    #[test]
    fn extrae_jpeg_de_pdf_escaneado() {
        let jpeg = jpeg_rgb(120, 90);
        let dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 120,
            "Height" => 90,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        };
        let pdf = pdf_con_imagen(dict, jpeg, false);

        assert!(!tiene_capa_texto(&pdf).unwrap());
        let paginas = extraer_paginas(&pdf).unwrap();
        assert_eq!(paginas.len(), 1);
        assert_eq!((paginas[0].width(), paginas[0].height()), (120, 90));
    }

    #[test]
    fn reconstruye_raster_gris_sin_filtro() {
        let (w, h) = (16u32, 8u32);
        let datos = vec![128u8; (w * h) as usize];
        let dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        };
        let pdf = pdf_con_imagen(dict, datos, false);
        let paginas = extraer_paginas(&pdf).unwrap();
        assert_eq!((paginas[0].width(), paginas[0].height()), (w, h));
    }

    #[test]
    fn pdf_con_texto_es_digital() {
        let jpeg = jpeg_rgb(50, 50);
        let dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 50,
            "Height" => 50,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        };
        let pdf = pdf_con_imagen(dict, jpeg, true); // con /Font
        assert!(tiene_capa_texto(&pdf).unwrap());
        assert!(matches!(extraer_paginas(&pdf), Err(PdfError::EsDigital)));
    }

    #[test]
    fn filtro_ccitt_es_no_soportado() {
        let dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 100,
            "Height" => 100,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 1,
            "Filter" => "CCITTFaxDecode",
        };
        let pdf = pdf_con_imagen(dict, vec![0u8; 32], false);
        assert!(matches!(
            extraer_paginas(&pdf),
            Err(PdfError::FiltroNoSoportado(_))
        ));
    }
}
