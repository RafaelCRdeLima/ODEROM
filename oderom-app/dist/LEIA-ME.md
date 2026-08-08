# Este frontend é compartilhado entre o app Tauri e a versão web

**Leia antes de editar qualquer arquivo deste diretório.**

Os arquivos aqui (`index.html`, `notebook.js`, `notebook.css`,
`oderom-mode.js`, `vendor/`) servem a **dois** hospedeiros:

1. **O app de desktop** (`oderom-app/src-tauri`), onde um webview do Tauri
   carrega esta pasta e o Rust responde por `#[tauri::command]`.
2. **A versão web** (WebAssembly, publicada no GitHub Pages), onde o mesmo
   HTML roda no navegador do aluno e o Rust responde compilado para wasm.

A decisão de manter **uma** cópia, em vez de bifurcar em `oderom-web/`, foi
deliberada: duas cópias divergem, e o custo aparece meses depois, quando uma
correção de interface é feita em um lado e esquecida no outro. O preço dessa
escolha é a regra abaixo.

## A regra

> Uma mudança aqui afeta desktop **e** web. Se você mexer em algo, verifique
> os dois — ou deixe registrado que só verificou um.

## Onde está a fronteira

Todo o acoplamento com o hospedeiro está em **uma** função, `invoke`, obtida
em `backend.js`. O `notebook.js` não sabe em qual dos dois está rodando: ele
chama `invoke("nome_do_comando", { ...args })` e pronto.

```
notebook.js  ──chama──>  invoke()  ──despacha──>  Tauri  (desktop)
                          │
                          └───────────────────>  wasm   (navegador)
```

São **14 comandos** hoje:

```
create_block   delete_block    edit_block      list_blocks
execute_block  cancel_block    clear_execution
new_notebook   open_notebook   save_notebook
load_gallery   gallery_list    copy_to_clipboard   frontend_ready
```

**Acrescentar um comando exige implementá-lo nos dois backends.** Se só um
existir, o frontend compartilhado quebra no outro — e provavelmente não na
sua máquina, e sim na do aluno.

## Três comandos não são simétricos

Onze dos catorze são tradução direta. Estes têm semântica diferente por
hospedeiro, e é aqui que mora o risco:

| comando | desktop | navegador |
|---|---|---|
| `execute_block` / `cancel_block` | thread + cancelação profunda por unwind | Web Worker; cancelar = encerrar o worker |
| `open_notebook` / `save_notebook` | diálogo do sistema, caminho de arquivo | seletor do navegador e download |
| `copy_to_clipboard` | `arboard` (área de transferência do SO) | `navigator.clipboard` |

Sobre o cancelamento: `wasm32` é `panic = "abort"` por construção, então o
`catch_unwind` de `oderom-expr::cancel` não funciona lá. Veja o comentário
em `oderom-expr/src/cancel.rs` — a cancelação profunda é compilada fora no
alvo wasm de propósito, e quem cancela é o JavaScript encerrando o worker.

## Dívida conhecida: os DTOs ainda não são compartilhados

Os tipos que atravessam a fronteira (`BlockDto`, `NotebookDto`,
`ComponentDto`, `EntryDto`, `GalleryEntryDto`, ~145 linhas) estão definidos
**dentro** de `oderom-app/src-tauri/src/lib.rs`. O backend wasm precisa
produzir exatamente o mesmo JSON, e hoje isso é garantido por disciplina, não
pelo compilador.

Enquanto essa dívida existir, **mudar um DTO é mudar um contrato**: altere-o
nos dois lados na mesma mudança. O passo que fecha esse buraco é extrair os
DTOs e suas funções de conversão para um crate próprio, do qual os dois
backends dependam — aí o compilador passa a cobrar o que hoje é lembrança.
