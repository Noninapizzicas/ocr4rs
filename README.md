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
- **Órgano autosuficiente**: acepta la página en su forma **cruda** (foto
  torcida, PDF escaneado) y hace TODO el camino — rasterizar + limpiar +
  reconocer. El puente de Enki solo pasa una ruta.
- **Sin LLM dentro**: OCR4RS devuelve *texto*. La *estructura* (campos de una
  factura: total, NIF, fecha) la pone el LLM del consumidor (Enki) —
  composición, no acoplamiento.
- **Degradación honesta**: sin modelos, `POST /ocr` responde **503** con
  motivo; nunca inventa texto. El pre-proceso *mejora*, no *inventa*.

## Frontera con Crawl4RS

OCR4RS trabaja lo **rasterizado**; el PDF **digital** (con capa de texto) es de
Crawl4RS. Un PDF escaneado *no es un documento vectorial*: cada página es una
imagen ráster embebida — no se **renderiza**, se **extrae** (con `lopdf`, Rust
puro). Si el PDF trae capa de texto, OCR4RS lo detecta y responde **422**
apuntando a Crawl4RS. Nunca renderiza vectores.

## Pipeline de pre-proceso

Preparar la imagen *es* parte de hacer OCR bien. Cada paso es opcional y
graduable (`ocr4rs-prep`, Rust puro):

`grises → normalizar → deskew → denoise → upscale → binarizar → recortar`

Por defecto se aplican **grises + normalizar + deskew** (lo que casi siempre
gana). La **binarización va apagada por defecto**: `ocrs` es un reconocedor
neuronal entrenado sobre grises con antialias, y binarizar duro puede *bajar*
el acierto (al revés que el OCR clásico). Se activa y se calibra con datos
reales.

## Modelos (se aportan en runtime)

Los modelos `.rten` **no se versionan** (pesan MB). Descárgalos una vez:

```bash
./scripts/get-models.sh models      # baja text-detection.rten y text-recognition.rten
export OCR4RS_MODELS=$PWD/models
```

## Uso

```bash
cargo build --release

# OCR de una imagen → texto (o --json con páginas y líneas)
ocr4rs image factura.png
ocr4rs image factura.png --json

# OCR de un PDF escaneado (extrae el ráster de cada página)
ocr4rs pdf escaneo.pdf

# Graduar el pipeline (deskew, binarización, upscale…)
ocr4rs image foto.jpg --no-deskew --binarize otsu
ocr4rs image foto.jpg --upscale-min 1000

# Ver la imagen YA LIMPIA sin OCR (no necesita modelos)
ocr4rs prep foto.jpg --out limpia.png

# Servidor HTTP (para el bus de Enki)
ocr4rs serve --port 8090
```

### API

```text
POST /ocr    cuerpo = bytes (image/* | application/pdf escaneado)
             flags de pipeline por query: ?deskew=false&binarize=otsu&normalize=false
             -> { text, pages:[{ n, text, lines:[{text}], prep }], source_kind }
             422 si el PDF es digital (usa crawl4rs) · 503 sin modelos
POST /prep   devuelve la primera página LIMPIA en PNG, SIN OCR (funciona sin modelos)
GET  /health -> { status, models_loaded, prep_ready }
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
| `ocr4rs-core` | Motor `Ocr` (ocrs/rten) + orquestación: bytes → texto por páginas. |
| `ocr4rs-prep` | Pipeline de limpieza (grises, deskew, binarizar…). Rust puro, sin modelos. |
| `ocr4rs-pdf` | Extracción del ráster de un PDF escaneado (`lopdf`). No renderiza vectores. |
| `ocr4rs-cli` | Binario `ocr4rs` (`image`, `pdf`, `prep`, `serve`). |
| `ocr4rs-server` | API HTTP (`axum`); 503 honesto sin modelos. |

## Estado

Los cinco crates compilan; `fmt`/`clippy -D warnings` y la batería de tests en
verde (deskew, extracción de JPEG de PDF, detección de PDF digital, filtros no
soportados, `/prep` sin modelos, degradación 503). La verificación de
**precisión** requiere los modelos y una imagen/PDF real — se hace en una
máquina con red (los modelos se descargan una vez).

## Licencia

MIT.
