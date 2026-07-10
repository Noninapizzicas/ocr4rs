//! CLI de OCR4RS.
//!
//! ```text
//! ocr4rs image <ruta>   OCR de una imagen → texto (o JSON con --json)
//! ocr4rs serve          Lanza el servidor API
//! ```

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use ocr4rs_core::Ocr;

#[derive(Parser)]
#[command(name = "ocr4rs", version, about = "OCR en Rust puro: imagen → texto.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Aplica OCR a una imagen.
    Image {
        /// Ruta de la imagen (PNG/JPEG/WebP).
        path: PathBuf,
        /// Directorio de modelos (`.rten`). Por defecto `$OCR4RS_MODELS`.
        #[arg(long)]
        models: Option<PathBuf>,
        /// Salida como JSON (texto + líneas).
        #[arg(long)]
        json: bool,
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

fn models_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    explicit
        .or_else(|| std::env::var_os("OCR4RS_MODELS").map(PathBuf::from))
        .ok_or_else(|| {
            anyhow::anyhow!("indica los modelos con --models o la variable OCR4RS_MODELS")
        })
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
        Command::Image { path, models, json } => {
            let dir = models_dir(models)?;
            let ocr = Ocr::from_model_dir(&dir)?;
            let bytes = std::fs::read(&path)?;
            let out = ocr.recognize_bytes(&bytes)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("{}", out.text);
            }
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
