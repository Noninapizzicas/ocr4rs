//! CLI de OCR4RS.
//!
//! ```text
//! ocr4rs image <ruta>   OCR de una imagen → texto (o JSON con --json)
//! ocr4rs pdf   <ruta>   OCR de un PDF escaneado → texto por páginas
//! ocr4rs prep  <ruta>   Vuelca la primera página LIMPIA (sin OCR) a PNG
//! ocr4rs serve          Lanza el servidor API
//! ```
//!
//! El órgano hace TODO el camino: acepta la página cruda (foto torcida, PDF
//! escaneado), la prepara (deskew, normalizar…) y devuelve texto.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use ocr4rs_core::prep::{Binarize, PrepConfig};
use ocr4rs_core::{Ocr, ProcessConfig};

#[derive(Parser)]
#[command(
    name = "ocr4rs",
    version,
    about = "OCR en Rust puro: imagen/PDF escaneado → texto."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Aplica OCR a una imagen (PNG/JPEG/WebP).
    Image {
        /// Ruta de la imagen.
        path: PathBuf,
        #[command(flatten)]
        models: ModelArg,
        #[command(flatten)]
        prep: PrepArgs,
        /// Salida como JSON (texto + páginas + líneas).
        #[arg(long)]
        json: bool,
    },
    /// Aplica OCR a un PDF escaneado (extrae el ráster de cada página).
    Pdf {
        /// Ruta del PDF.
        path: PathBuf,
        #[command(flatten)]
        models: ModelArg,
        #[command(flatten)]
        prep: PrepArgs,
        /// Salida como JSON.
        #[arg(long)]
        json: bool,
    },
    /// Vuelca la primera página YA LIMPIA a un PNG (sin OCR; no usa modelos).
    Prep {
        /// Ruta de la imagen o PDF.
        path: PathBuf,
        /// Fichero PNG de salida.
        #[arg(long, default_value = "prep.png")]
        out: PathBuf,
        #[command(flatten)]
        prep: PrepArgs,
    },
    /// Lanza el servidor API.
    Serve {
        /// Puerto de escucha.
        #[arg(long, default_value_t = 8090)]
        port: u16,
        /// Dirección de escucha.
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
    },
}

#[derive(Args)]
struct ModelArg {
    /// Directorio de modelos (`.rten`). Por defecto `$OCR4RS_MODELS`.
    #[arg(long)]
    models: Option<PathBuf>,
}

/// Flags que gradúan el pipeline de pre-proceso.
#[derive(Args)]
struct PrepArgs {
    /// No estirar el contraste.
    #[arg(long)]
    no_normalize: bool,
    /// No corregir la inclinación (deskew).
    #[arg(long)]
    no_deskew: bool,
    /// Ángulo máximo de deskew (grados).
    #[arg(long, default_value_t = 8.0)]
    max_skew: f32,
    /// Filtro de mediana (denoise).
    #[arg(long)]
    denoise: bool,
    /// Binarización final.
    #[arg(long, value_enum, default_value_t = BinArg::None)]
    binarize: BinArg,
    /// Escalar ×2 si el lado mayor es menor que N px.
    #[arg(long)]
    upscale_min: Option<u32>,
    /// Recortar el marco vacío alrededor del contenido.
    #[arg(long)]
    trim: bool,
}

#[derive(Clone, Copy, ValueEnum, Default)]
enum BinArg {
    #[default]
    None,
    Otsu,
    Adaptive,
}

impl From<BinArg> for Binarize {
    fn from(b: BinArg) -> Self {
        match b {
            BinArg::None => Binarize::None,
            BinArg::Otsu => Binarize::Otsu,
            BinArg::Adaptive => Binarize::Adaptive,
        }
    }
}

impl PrepArgs {
    fn config(&self) -> PrepConfig {
        PrepConfig {
            normalize: !self.no_normalize,
            deskew: !self.no_deskew,
            max_skew_deg: self.max_skew,
            denoise: self.denoise,
            binarize: self.binarize.into(),
            upscale_min_dim: self.upscale_min,
            trim_border: self.trim,
        }
    }
}

fn models_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    explicit
        .or_else(|| std::env::var_os("OCR4RS_MODELS").map(PathBuf::from))
        .context("indica los modelos con --models o la variable OCR4RS_MODELS")
}

fn run_ocr(path: PathBuf, models: ModelArg, prep: PrepArgs, json: bool) -> Result<()> {
    let dir = models_dir(models.models)?;
    let ocr = Ocr::from_model_dir(&dir)?;
    let bytes = std::fs::read(&path).with_context(|| format!("leyendo {}", path.display()))?;
    let cfg = ProcessConfig {
        prep: prep.config(),
    };
    let out = ocr.process(&bytes, &cfg)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("{}", out.text);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Image {
            path,
            models,
            prep,
            json,
        } => run_ocr(path, models, prep, json),
        Command::Pdf {
            path,
            models,
            prep,
            json,
        } => run_ocr(path, models, prep, json),
        Command::Prep { path, out, prep } => {
            let bytes =
                std::fs::read(&path).with_context(|| format!("leyendo {}", path.display()))?;
            let (gray, report) = ocr4rs_core::preparar_primera_pagina(&bytes, &prep.config())?;
            gray.save(&out)
                .with_context(|| format!("guardando {}", out.display()))?;
            eprintln!("Pasos: {} · {}", report.steps.join(" → "), out.display());
            Ok(())
        }
        Command::Serve { host, port } => {
            let dir = models_dir(None).ok();
            let addr: std::net::SocketAddr = format!("{host}:{port}").parse()?;
            eprintln!("Servidor OCR4RS en http://{addr}");
            ocr4rs_server::serve(addr, dir).await?;
            Ok(())
        }
    }
}
