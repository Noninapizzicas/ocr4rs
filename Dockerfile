# syntax=docker/dockerfile:1

# ============================================================================
# Etapa de compilación — `rust:1-bookworm` (Debian 12). IMPORTANTE: fijar la
# generación de Debian al mismo glibc que el runtime (`distroless-cc-debian12`,
# glibc 2.36). `rust:1` a secas flota a Debian 13 (glibc 2.39) y produce un
# binario que NO arranca en el runtime. El perfil `release` ya hace `strip`.
# ============================================================================
FROM rust:1-bookworm AS builder
WORKDIR /build

# No copiamos rust-toolchain.toml: el builder usa el toolchain de la imagen
# `rust:1` (evita un re-sync de rustup contra la red en cada build).
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --bin ocr4rs

# ============================================================================
# Imagen final — distroless (sin navegador, sin apt, ~pocos MB). OCR4RS es
# Rust puro: no arrastra ONNX Runtime, MNN ni Python; sólo necesita glibc.
#
# Los modelos `.rten` NO viven en la imagen (pesan MB y no se versionan): se
# montan como volumen de sólo lectura en /models y se descargan una vez con
# `scripts/get-models.sh`. Sin modelos, `POST /ocr` degrada con 503 honesto.
# ============================================================================
FROM gcr.io/distroless/cc-debian12 AS runtime
COPY --from=builder /build/target/release/ocr4rs /usr/local/bin/ocr4rs

ENV OCR4RS_MODELS=/models
EXPOSE 8090
ENTRYPOINT ["/usr/local/bin/ocr4rs"]
CMD ["serve", "--host", "0.0.0.0", "--port", "8090"]
