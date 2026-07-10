//! # ocr4rs-server
//!
//! API HTTP del órgano OCR. Un motor autosuficiente, expuesto por HTTP para
//! que Enki lo consuma por el bus con solo pasar una ruta.
//!
//! Endpoints:
//! - `POST /ocr` — cuerpo = bytes (`image/*` **o** `application/pdf` escaneado).
//!   Detecta el tipo, extrae el ráster de cada página si es PDF, lo prepara
//!   (deskew, normalizar…) y reconoce. Devuelve `{ text, pages, source_kind }`.
//!   Flags de pipeline por query (`?deskew=false&binarize=otsu&…`).
//! - `POST /prep` — devuelve la primera página **limpia** en PNG, SIN OCR.
//!   Funciona aunque no haya modelos (para ver qué hace el pre-proceso).
//! - `GET /health` — sonda + estado de modelos y del pre-proceso.
//!
//! Frontera honesta: un PDF **digital** (con capa de texto) no es trabajo de
//! OCR4RS → se responde 422 apuntando a Crawl4RS. Sin modelos, `/ocr` degrada
//! con 503; `/prep` sigue funcionando (no depende de modelos).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use ocr4rs_core::prep::PrepConfig;
use ocr4rs_core::{Error, Ocr, ProcessConfig};

/// Estado compartido: el motor OCR, si los modelos estaban disponibles.
#[derive(Clone)]
struct AppState {
    ocr: Option<Arc<Ocr>>,
}

/// Construye el router. `models_dir` = directorio de modelos, si se conoce.
pub fn router(models_dir: Option<PathBuf>) -> Router {
    let ocr = match &models_dir {
        Some(dir) => match Ocr::from_model_dir(dir) {
            Ok(engine) => {
                tracing::info!(dir = %dir.display(), "modelos OCR cargados");
                Some(Arc::new(engine))
            }
            Err(e) => {
                tracing::warn!(error = %e, "sin modelos OCR; /ocr degradará con 503");
                None
            }
        },
        None => {
            tracing::warn!("OCR4RS_MODELS no configurado; /ocr degradará con 503");
            None
        }
    };

    Router::new()
        .route("/health", get(health))
        .route("/ocr", post(ocr_handler))
        .route("/prep", post(prep_handler))
        .with_state(AppState { ocr })
}

/// Arranca el servidor en `addr`. Bloquea hasta que termina.
pub async fn serve(addr: SocketAddr, models_dir: Option<PathBuf>) -> std::io::Result<()> {
    let app = router(models_dir);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "servidor OCR4RS escuchando");
    axum::serve(listener, app).await
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "models_loaded": state.ocr.is_some(),
        "prep_ready": true,
    }))
}

/// Traduce un error del núcleo a un código HTTP honesto.
fn error_a_http(e: &Error) -> (StatusCode, String) {
    match e {
        // El PDF digital no es nuestro trabajo: díselo claro y manda a crawl4rs.
        Error::IsDigitalPdf => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        Error::UnsupportedInput(_) => (StatusCode::UNSUPPORTED_MEDIA_TYPE, e.to_string()),
        Error::Image(_) | Error::Pdf(_) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn ocr_handler(
    State(state): State<AppState>,
    Query(prep): Query<PrepConfig>,
    body: Bytes,
) -> Response {
    let Some(ocr) = state.ocr.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "OCR no disponible: faltan los modelos (define OCR4RS_MODELS)",
        )
            .into_response();
    };
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "cuerpo vacío: envía bytes").into_response();
    }

    let cfg = ProcessConfig { prep };
    // La inferencia es CPU-bound; se ejecuta en un hilo de bloqueo.
    let result = tokio::task::spawn_blocking(move || ocr.process(&body, &cfg)).await;
    match result {
        Ok(Ok(out)) => Json(out).into_response(),
        Ok(Err(e)) => error_a_http(&e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Devuelve la primera página limpia en PNG, sin OCR. No depende de modelos.
async fn prep_handler(Query(prep): Query<PrepConfig>, body: Bytes) -> Response {
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "cuerpo vacío: envía bytes").into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        let (gray, _report) = ocr4rs_core::preparar_primera_pagina(&body, &prep)?;
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(gray)
            .write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| Error::Image(e.to_string()))?;
        Ok::<Vec<u8>, Error>(buf.into_inner())
    })
    .await;

    match result {
        Ok(Ok(png)) => ([(header::CONTENT_TYPE, "image/png")], png).into_response(),
        Ok(Err(e)) => error_a_http(&e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_reporta_sin_modelos() {
        let app = router(None);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["models_loaded"], false);
        assert_eq!(v["prep_ready"], true);
    }

    #[tokio::test]
    async fn ocr_sin_modelos_degrada_503() {
        let app = router(None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ocr")
                    .body(Body::from(vec![1u8, 2, 3]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// `/prep` funciona SIN modelos: prepara una imagen y devuelve PNG.
    #[tokio::test]
    async fn prep_sin_modelos_devuelve_png() {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(20, 20, image::Luma([200])))
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();

        let app = router(None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/prep")
                    .body(Body::from(buf.into_inner()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get(header::CONTENT_TYPE).cloned();
        assert_eq!(ct.unwrap(), "image/png");
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&bytes[1..4], b"PNG");
    }
}
