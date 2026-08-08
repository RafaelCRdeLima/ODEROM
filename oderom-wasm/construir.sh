#!/usr/bin/env bash
# Monta a versão web do ODEROM em `oderom-web/` -- uma pasta de arquivos
# estáticos, pronta para o GitHub Pages, que o aluno abre e usa sem
# instalar nada.
#
# A pasta é montada, nunca versionada: o conteúdo dela é inteiramente
# derivado de `oderom-app/dist/` (o frontend, compartilhado com o app de
# desktop) e do crate `oderom-wasm`. Ver `oderom-app/dist/LEIA-ME.md`.
set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SAIDA="${1:-$RAIZ/oderom-web}"

# --- 1. As duas versões do wasm-bindgen precisam bater --------------
#
# O crate gera metadados que o CLI lê, e o formato muda entre versões. Sem
# esta checagem o erro aparece só no passo 3, com uma mensagem que não diz
# qual dos dois mudar -- e a resposta certa é quase sempre "atualize o
# CLI", que não é o que a mensagem sugere.
VERSAO_CRATE="$(grep -oP 'wasm-bindgen = "=\K[0-9.]+' "$RAIZ/oderom-wasm/Cargo.toml")"
VERSAO_CLI="$(wasm-bindgen --version 2>/dev/null | grep -oP '[0-9.]+' || true)"
if [ -z "$VERSAO_CLI" ]; then
  echo "erro: wasm-bindgen (o programa) não está instalado." >&2
  echo "      cargo install -f wasm-bindgen-cli --version $VERSAO_CRATE" >&2
  exit 1
fi
if [ "$VERSAO_CRATE" != "$VERSAO_CLI" ]; then
  echo "erro: wasm-bindgen CLI é $VERSAO_CLI, mas oderom-wasm pede $VERSAO_CRATE." >&2
  echo "      cargo install -f wasm-bindgen-cli --version $VERSAO_CRATE" >&2
  echo "      (ou mude a versão fixada em oderom-wasm/Cargo.toml para $VERSAO_CLI)" >&2
  exit 1
fi

# --- 2. Compila o Rust para wasm ------------------------------------
echo "==> compilando oderom-wasm para wasm32-unknown-unknown"
cargo build -p oderom-wasm --target wasm32-unknown-unknown --release --manifest-path "$RAIZ/Cargo.toml"

# --- 3. Gera a ponte JS ---------------------------------------------
#
# `--target web` produz um módulo ES que o `import()` do `backend.js`
# carrega direto, sem bundler e sem servidor de build: o resultado é uma
# pasta de arquivos, que é exatamente o que o GitHub Pages serve.
echo "==> gerando a ponte JS em $SAIDA/wasm"
rm -rf "$SAIDA"
mkdir -p "$SAIDA/wasm"
wasm-bindgen \
  --target web \
  --no-typescript \
  --out-dir "$SAIDA/wasm" \
  "$RAIZ/target/wasm32-unknown-unknown/release/oderom_wasm.wasm"

# --- 4. Copia o frontend --------------------------------------------
#
# A MESMA pasta que o app de desktop carrega -- não uma cópia mantida em
# paralelo. Ficam de fora só os arquivos que existem para o teste
# automatizado do desktop (`keytest.*`, dirigidos por `tests/keymap.rs`) e
# o LEIA-ME, que é documentação para quem edita, não para quem usa.
echo "==> copiando o frontend de oderom-app/dist"
cp -r "$RAIZ/oderom-app/dist/." "$SAIDA/"
rm -f "$SAIDA"/keytest.* "$SAIDA/LEIA-ME.md"

TAMANHO="$(du -sh "$SAIDA" | cut -f1)"
echo
echo "pronto: $SAIDA ($TAMANHO)"
echo
echo "para testar localmente (o import() de módulo ES não funciona por file://):"
echo "  python3 -m http.server -d $SAIDA 8000"
echo "  e abra http://localhost:8000"
