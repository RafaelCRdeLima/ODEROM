#!/usr/bin/env bash
# Rasteriza `oderom-icone.svg` nos tamanhos que o Tauri empacota e
# escreve em `oderom-app/src-tauri/icons/` -- os arquivos que
# `tauri.conf.json` lista em `bundle.icon`.
#
# Roda a mao, e nao no build: os icones so mudam quando a marca muda, e
# rasterizar num navegador a cada `cargo build` seria caro e daria ao
# build uma dependencia de Chrome que ele nao tem hoje. Os PNG ficam
# versionados, como ja estavam.
set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FONTE="$RAIZ/assets/oderom-icone.svg"
DESTINO="$RAIZ/oderom-app/src-tauri/icons"

# Chrome para rasterizar e ImageMagick para empacotar. O Chrome porque
# ele ja e' dependencia de teste (oderom-wasm/tests/navegador.rs) e
# renderiza SVG exatamente como o navegador do aluno vai renderizar;
# o `convert` do ImageMagick nao rasteriza SVG de forma confiavel sem
# delegate, mas junta PNG em .ico/.icns muito bem.
CHROME="$(command -v google-chrome || command -v chromium || command -v chromium-browser || true)"
if [ -z "$CHROME" ]; then
  echo "erro: preciso de google-chrome/chromium para rasterizar o SVG." >&2
  exit 1
fi
if ! command -v convert >/dev/null; then
  echo "erro: preciso do ImageMagick (convert) para montar .ico/.icns." >&2
  echo "      sudo apt install imagemagick" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# O SVG tem cantos arredondados, entao o que estiver fora deles precisa
# ficar transparente -- sem isto o Chrome pinta branco e o icone ganha
# quatro quinas brancas em qualquer tema escuro.
render() {
  local px="$1" saida="$2"
  cat > "$TMP/i.html" <<EOF
<!doctype html><meta charset="utf-8">
<body style="margin:0"><img src="$FONTE" style="width:${px}px;height:${px}px;display:block"></body>
EOF
  "$CHROME" --headless --disable-gpu --no-sandbox \
    --default-background-color=00000000 \
    --screenshot="$saida" --window-size="$px,$px" \
    --virtual-time-budget=5000 "$TMP/i.html" 2>/dev/null
}

mkdir -p "$DESTINO"
for px in 16 32 48 64 128 256 512 1024; do
  echo "==> ${px}px"
  render "$px" "$TMP/icon-$px.png"
done

# Os quatro PNG que o tauri.conf.json nomeia.
for px in 32 128 256 512; do
  cp "$TMP/icon-$px.png" "$DESTINO/icon-$px.png"
done

# .ico do Windows: varios tamanhos num arquivo so, para o sistema
# escolher o certo por contexto (barra de tarefas, Alt+Tab, atalho).
convert "$TMP/icon-16.png" "$TMP/icon-32.png" "$TMP/icon-48.png" \
        "$TMP/icon-64.png" "$TMP/icon-128.png" "$TMP/icon-256.png" \
        "$DESTINO/icon.ico"

# .icns do macOS, montado a mao.
#
# O `convert` do ImageMagick NAO serve aqui: sem o delegate de ICNS ele
# escreve um PNG 16x16 e apenas o renomeia -- um arquivo de 1 KB que
# passa por qualquer verificacao superficial ("existe? tem bytes?") e
# nao e' um icone. Descoberto comparando com o .icns anterior, de 103
# KB. O formato em si e' simples o bastante para escrever direto:
# 'icns', tamanho total, e uma sequencia de blocos [tipo, tamanho,
# PNG], onde cada tipo declara uma resolucao.
python3 - "$TMP" "$DESTINO/icon.icns" <<'PY'
import struct, sys
from pathlib import Path

tmp, saida = Path(sys.argv[1]), Path(sys.argv[2])

# Tipo do bloco -> lado em pixels. Os `icp*` sao os pequenos e os `ic0*`
# os grandes; ambos aceitam PNG como conteudo, que e' o que geramos.
BLOCOS = [(b"icp4", 16), (b"icp5", 32), (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)]

corpo = b""
for tipo, px in BLOCOS:
    png = (tmp / f"icon-{px}.png").read_bytes()
    # O tamanho declarado INCLUI os 8 bytes do proprio cabecalho do bloco.
    corpo += tipo + struct.pack(">I", len(png) + 8) + png

saida.write_bytes(b"icns" + struct.pack(">I", len(corpo) + 8) + corpo)
print(f"icns: {len(BLOCOS)} resolucoes, {saida.stat().st_size} bytes")
PY

echo
ls -l "$DESTINO"
