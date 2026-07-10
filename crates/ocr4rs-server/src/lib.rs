//! # ocr4rs-server
//!
//! API HTTP del órgano OCR. Un motor, expuesto por HTTP para que Enki lo
//! consuma por el bus. Si no hay modelos configurados, `POST /ocr` degrada
//! con **503** y un motivo — no inventa texto.
//!
//! Endpoints:
//! - `POST /ocr` — cuerpo = bytes de imagen (`image/*`). Devuelve
//!   `{ text, lines: [{text}] }`.
//! - `GET /health` — sonda de salud + estado de los modelos.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use ocr4rs_core::Ocr;

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
    }))
}

async fn ocr_handler(State(state): State<AppState>, body: Bytes) -> Response {
    let Some(ocr) = state.ocr.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "OCR no disponible: faltan los modelos (define OCR4RS_MODELS)",
        )
            .into_response();
    };
    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "cuerpo vacío: envía bytes de imagen",
        )
            .into_response();
    }

    // La inferencia es CPU-bound; se ejecuta en un hilo de bloqueo.
    let result = tokio::task::spawn_blocking(move || ocr.recognize_bytes(&body)).await;
    match result {
        Ok(Ok(out)) => Json(out).into_response(),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
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
}
