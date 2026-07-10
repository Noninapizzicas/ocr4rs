# OCR4RS

> Órgano de **OCR en Rust puro**: convierte una imagen (o página de PDF
> escaneado) en texto. Sin ONNX Runtime, sin MNN, sin Python — solo `cargo`.

Motor sobre [`ocrs`](https://github.com/robertknight/ocrs) + `rten`
(inferencia ML en Rust puro). Pensado como **órgano independiente** que
complementa a [Crawl4RS](https://github.com/noninapizzicas/d-os): cada uno
especializado en lo suyo.

- **Crawl4RS** → web (HTML/PDF **digital**) → Markdown.
- **OCR4RS** → imagen / PDF **escaneado** → texto.

Se encuentran en el bus de Enki, no en el código.

## Filosofía

- **Rust puro**: el binario no arrastra runtimes nativos (por eso `ocrs`, no
  las variantes con ONNX Runtime/MNN).
- **Sin LLM dentro**: OCR4RS devuelve *texto*. La *estructura* (campos de una
  factura: total, NIF, fecha) la pone el LLM del consumidor (Enki) —
  composición, no acoplamiento.
- **Degradación honesta**: sin modelos, `POST /ocr` responde **503** con
  motivo; nunca inventa texto.

## Modelos (se aportan en runtime)

Los modelos `.rten` **no se versionan** (pesan MB). Descárgalos una vez:

```bash
./scripts/get-models.sh models      # baja text-detection.rten y text-recognition.rten
export OCR4RS_MODELS=$PWD/models
```

## Uso

```bash
cargo build --release

# OCR de una imagen → texto (o --json con líneas)
ocr4rs image factura.png
ocr4rs image factura.png --json

# Servidor HTTP (para el bus de Enki)
ocr4rs serve --port 8090
#   POST /ocr   (cuerpo = bytes de imagen)  -> { text, lines: [{text}] }
#   GET  /health                            -> { status, models_loaded }
```

## Docker

Imagen **independiente** (distroless, Rust puro, ~47 MB — sin navegador, sin
ONNX Runtime). Los modelos se montan como volumen; sin ellos, `/ocr` da 503.

```bash
./scripts/get-models.sh models            # una vez
docker build -t ocr4rs:latest .
docker run -p 8090:8090 -v "$PWD/models:/models:ro" ocr4rs:latest
```

Cada órgano web es su propia imagen (se especializan por separado y se
encuentran en el bus de Enki, no en el mismo contenedor). El
`docker-compose.yml` levanta Crawl4RS y OCR4RS juntos por conveniencia, sin
acoplarlos:

```bash
docker compose up      # crawl4rs:8080 + ocr4rs:8090
```

## Estructura

| Crate | Responsabilidad |
|-------|-----------------|
| `ocr4rs-core` | Motor `Ocr` (ocrs/rten): imagen → `{text, lines}`. |
| `ocr4rs-cli` | Binario `ocr4rs` (`image`, `serve`). |
| `ocr4rs-server` | API HTTP (`axum`); 503 honesto sin modelos. |

## Estado

Núcleo, CLI y servidor compilando; el flujo sin modelos degrada con 503
(probado). La verificación de **precisión** requiere los modelos y una imagen
real — se hace en una máquina con red (los modelos se descargan una vez).

## Licencia

MIT.
