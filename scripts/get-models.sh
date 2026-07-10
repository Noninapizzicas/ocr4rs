#!/usr/bin/env bash
# Descarga los modelos `.rten` de ocrs a ./models (una vez).
#
# Los modelos NO se versionan (pesan MB). ocr4rs los carga en runtime desde
# el directorio indicado por --models o $OCR4RS_MODELS.
#
# Fuente: modelos convertidos que publica el proyecto ocrs.
# Ver: https://github.com/robertknight/ocrs
set -euo pipefail

DIR="${1:-models}"
BASE="https://ocrs-models.s3-accelerate.amazonaws.com"
mkdir -p "$DIR"

echo "Descargando modelos ocrs a $DIR ..."
curl -fsSL "$BASE/text-detection.rten"   -o "$DIR/text-detection.rten"
curl -fsSL "$BASE/text-recognition.rten" -o "$DIR/text-recognition.rten"
echo "Listo. Usa: OCR4RS_MODELS=$DIR ocr4rs image factura.png"
