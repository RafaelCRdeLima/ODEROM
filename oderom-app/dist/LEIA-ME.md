# Este frontend é compartilhado entre o app Tauri e a versão web

**Leia antes de editar qualquer arquivo deste diretório.**

Os arquivos aqui (`index.html`, `notebook.js`, `notebook.css`,
`oderom-mode.js`, `backend.js`, `worker.js`, `vendor/`) servem a **dois**
hospedeiros:

1. **O app de desktop** (`oderom-app/src-tauri`), onde um webview do Tauri
   carrega esta pasta e o Rust responde por `#[tauri::command]`.
2. **A versão web** (crate `oderom-wasm`, publicada no GitHub Pages por
   `.github/workflows/web.yml`), onde o mesmo HTML roda no navegador do
   aluno e o Rust responde compilado para wasm.

Para montar a versão web localmente: `./oderom-wasm/construir.sh`, que
escreve em `oderom-web/` (não versionado — é tudo derivado daqui e do
crate). O teste `oderom-wasm/tests/navegador.rs` faz exatamente isso e
depois abre o resultado num Chrome de verdade.

A decisão de manter **uma** cópia, em vez de bifurcar o frontend em dois, foi
deliberada: duas cópias divergem, e o custo aparece meses depois, quando uma
correção de interface é feita em um lado e esquecida no outro. Por isso
`oderom-web/` é montado por script e não versionado — é build, não fonte. O
preço dessa escolha é a regra abaixo.

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

São **15 comandos** hoje:

```
create_block   delete_block    edit_block      list_blocks
execute_block  cancel_block    clear_execution
new_notebook   open_notebook   save_notebook
load_gallery   gallery_list    export_options
copy_to_clipboard              frontend_ready
```

**Acrescentar um comando exige implementá-lo nos dois backends.** Se só um
existir, o frontend compartilhado quebra no outro — e provavelmente não na
sua máquina, e sim na do aluno. O `backend.js` lança um erro nomeando o
comando quando isso acontece, em vez de falhar em silêncio.

Além desses catorze, o wasm expõe `notebook_text` e `load_notebook_text`, que
o `notebook.js` **nunca** chama: são as metades em Rust de
`save_notebook`/`open_notebook`, usadas só pelo `backend.js`. Ver adiante.

## `keytest.html` é uma cópia à mão deste markup

O `index.html` e o `keytest.html` (ponto de entrada de `tests/keymap.rs`)
carregam o **mesmo** `notebook.js`, então todo id que ele procura tem de
existir nos dois. Acrescentar um elemento só ao `index.html` faz o
`notebook.js` chamar `addEventListener` sobre `null` e derrubar a página
inteira no teste — cujo sintoma é "o teste de teclado quebrou", que não
aponta para a causa. Já aconteceu duas vezes.

`oderom-app/src-tauri/tests/paginas_em_sincronia.rs` cobra isso agora:
lê os ids que o `notebook.js` realmente procura e exige que as duas
páginas os declarem, além de conferir que carregam os mesmos `<script>`
na mesma ordem.

## Três comandos não são simétricos

Doze dos quinze são tradução direta. Estes têm semântica diferente por
hospedeiro, e é aqui que mora o risco:

| comando | desktop | navegador |
|---|---|---|
| `execute_block` / `cancel_block` | thread + cancelação profunda por unwind | Web Worker; cancelar = encerrar o worker |
| `open_notebook` / `save_notebook` | diálogo do sistema, caminho de arquivo | seletor do navegador e download (`backend.js`) |
| `copy_to_clipboard` | `arboard` (área de transferência do SO) | `navigator.clipboard` (`backend.js`) |

Os dois últimos são traduzidos no `backend.js`, sobre duas funções que só
existem no wasm (`notebook_text`, `load_notebook_text`): o Rust faz a metade
que é dele, o navegador faz a dele, e o `notebook.js` continua chamando
`save_notebook`/`open_notebook` sem saber de nada disso.

## O Web Worker, e o preço do cancelamento

O `oderom-wasm` **não** roda na thread da página: ele vive em `worker.js`. É
isso que faz a aba não congelar durante uma conta longa, e é isso que torna
o cancelamento possível.

Por que não dá para cancelar por dentro: `wasm32` é `panic = "abort"` por
construção, então o `catch_unwind` de `oderom-expr::cancel` não funciona lá
(veja o comentário em `oderom-expr/src/cancel.rs`, onde a cancelação
profunda é compilada fora nesse alvo). E o sinal por `Atomics` sobre
`SharedArrayBuffer` — o caminho usual — exige cabeçalhos COOP/COEP que o
GitHub Pages não deixa configurar. Sobra `worker.terminate()`, que é
chamado da página e não depende de o worker cooperar.

**Duas consequências.** A primeira, que o aluno vê: cancelar mata o worker, e
com ele todo o estado calculado. O `backend.js` recria o worker e recarrega o texto do
caderno (guardado antes de a conta começar), mas os resultados já obtidos
não voltam. Na prática, **cancelar equivale a cancelar e limpar a
execução** — o texto fica intacto, os resultados somem, e é preciso rodar as
declarações de novo. É a menor degradação disponível: a alternativa seria
não poder cancelar.

A segunda é para quem programa: **os ids dos blocos mudam depois de um
cancelamento**. O caderno é reconstruído a partir do texto, e o `Notebook`
novo numera de zero. O `notebook.js` não se importa — ele redesenha tudo a
cada `refresh()` e nunca guarda id entre uma chamada e outra — mas qualquer
código que guarde um id do lado de fora precisa saber. O `backend.js`
rastreia o bloco cancelado pela POSIÇÃO justamente por isso.

O `backend.js` reproduz o resto do contrato do Tauri para que o
`notebook.js` não precise saber de nada disso: `execute_block` volta na
hora, o bloco aparece como `running`, uma segunda execução é recusada com
`Blocked`. Enquanto o worker está ocupado, `list_blocks` é respondido a
partir do último resultado conhecido, com o bloco em execução marcado — o
worker não responderia, e sem isso o `pollUntilSettled` ficaria preso na
fila, que é justamente o congelamento que o worker existe para evitar.

## Os DTOs são compartilhados, e o compilador cobra isso

Os tipos que atravessam a fronteira (`BlockDto`, `NotebookDto`,
`ComponentDto`, `EntryDto`, `GalleryEntryDto`, `ExecuteOutcomeDto`) e suas
funções de conversão moram no crate **`oderom-ui`**, do qual os dois
backends dependem. Há uma definição só: mudar um campo quebra o build dos
dois lados na mesma hora, em vez de a versão web silenciosamente produzir um
JSON que o frontend não entende — na máquina do aluno, meses depois.

`oderom-ui` também monta as duas respostas inteiras que mais teriam a perder
com uma divergência: `notebook_dto()` (o `list_blocks`, chamado depois de
*toda* mudança de estado) e `gallery_entries()`. Os backends só as repassam.

O que o `oderom-ui` deliberadamente **não** contém é o despacho dos
comandos: `execute_block` numa thread aqui e síncrono lá, diálogo do
sistema contra seletor do navegador. Essas diferenças são reais, e forçá-las
numa abstração comum trocaria uma duplicação honesta por uma indireção que
mente. O que é obrigatoriamente igual — a *forma* dos dados — é o que está
compartilhado.
