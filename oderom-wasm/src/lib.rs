//! O ODEROM rodando dentro do navegador do aluno.
//!
//! Este crate é o segundo backend do frontend em `oderom-app/dist`: o
//! primeiro é o app de desktop (`oderom-app/src-tauri`, via
//! `#[tauri::command]`), este é a mesma página servida como arquivo
//! estático, com o Rust compilado para `wasm32-unknown-unknown`. O
//! aluno abre um link e o programa roda -- nada para baixar, nada para
//! instalar, nada que dependa do sistema operacional dele.
//!
//! **A forma dos dados não é decidida aqui.** Todo DTO que atravessa a
//! fronteira vem do `oderom-ui`, compartilhado com o backend Tauri, de
//! modo que os dois produzem literalmente o mesmo JSON por construção.
//! O que este crate decide é só o *despacho*: onde mora o estado e como
//! uma execução acontece.
//!
//! # A convenção de chamada
//!
//! Cada comando recebe uma string JSON (o objeto de argumentos que o
//! `notebook.js` já passa para o `invoke`) e devolve uma string JSON. O
//! `dist/backend.js` faz o `JSON.stringify`/`JSON.parse` nas pontas.
//!
//! Passar JSON como texto, em vez de converter `JsValue` campo a campo,
//! é deliberado: significa que a serialização aqui é *a mesma*
//! `serde_json` que o Tauri usa do outro lado, sobre *os mesmos* tipos
//! do `oderom-ui`. Qualquer outro caminho seria uma segunda
//! implementação da serialização, e uma segunda implementação é
//! exatamente o que este arranjo existe para não ter.
//!
//! # A divisão entre [`cmd`] e os exports
//!
//! Toda a lógica está em [`cmd`], onde erro é `String` e nada depende
//! de wasm. As funções `#[wasm_bindgen]` aqui embaixo são invólucros de
//! uma linha que só trocam `String` por `JsValue`.
//!
//! O motivo é poder testar: `JsValue` **aborta o processo** se
//! construído fora do wasm, então um teste de host que tocasse os
//! exports diretamente não falharia -- mataria o test runner. Com a
//! divisão, a suíte roda em `cargo test` junto com o resto do projeto,
//! em vez de exigir `wasm-pack test` e um navegador, que é o tipo de
//! teste que na prática ninguém roda.
//!
//! # O estado
//!
//! `wasm32-unknown-unknown` sem `SharedArrayBuffer` é single-threaded:
//! não há duas threads que possam tocar o notebook, então o estado é um
//! `thread_local!` com `RefCell`, e não o `Mutex` do lado Tauri. Não é
//! uma simplificação preguiçosa -- um `Mutex` aqui daria a impressão de
//! proteger contra uma concorrência que a plataforma não tem.

use wasm_bindgen::prelude::*;

pub mod cmd;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Date, js_name = now)]
    fn date_now() -> f64;
}

/// Roda automaticamente quando o módulo wasm é inicializado, antes de
/// qualquer comando -- `wasm_bindgen(start)` é o gancho do próprio
/// wasm-bindgen para isso, e usá-lo evita depender de o `backend.js`
/// lembrar de chamar uma função de setup.
///
/// Sem isto, `oderom_core::clock` não tem de onde tirar milissegundos e
/// reporta zero em todo lugar. Com isto, o tempo de execução que o
/// aluno vê no navegador é medido de verdade. `oderom-core` não pode
/// obter isso sozinho: ele não conhece JavaScript, e é este crate --
/// que já conhece -- quem injeta a fonte. Ver o doc comment do
/// `oderom-core/src/clock.rs`.
#[wasm_bindgen(start)]
pub fn iniciar() {
    oderom_core::clock::set_time_source(date_now);
}

/// Gera o invólucro `#[wasm_bindgen]` de cada comando de [`cmd`].
///
/// Um `Err` do `Result` de um `#[tauri::command]` chega ao frontend
/// como rejeição da promise; um `Err(JsValue)` de uma função
/// `wasm_bindgen` chega como exceção, que é o que o `await` do
/// `notebook.js` transforma em rejeição. Mesmo comportamento visto de
/// lá -- que é a única coisa que precisa ser igual.
macro_rules! exportar {
    ($($nome:ident),* $(,)?) => {
        $(
            #[wasm_bindgen]
            pub fn $nome(args: &str) -> Result<String, JsValue> {
                cmd::$nome(args).map_err(|e| JsValue::from_str(&e))
            }
        )*
    };
}

exportar!(
    list_blocks,
    create_block,
    edit_block,
    delete_block,
    execute_block,
    cancel_block,
    clear_execution,
    new_notebook,
    gallery_list,
    load_gallery,
    notebook_text,
    load_notebook_text,
    frontend_ready,
);
