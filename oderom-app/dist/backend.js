// Escolhe o backend que atende `invoke`, e é o ÚNICO lugar que sabe em
// qual hospedeiro a página está rodando. Ver LEIA-ME.md neste diretório.
//
// O `notebook.js` chama `invoke("comando", { ...args })` e nunca pergunta
// onde está. Manter essa ignorância é o que permite uma única cópia do
// frontend servir o app de desktop e a versão web: quando um comando novo
// aparece, a mudança no frontend é a mesma para os dois, e só o despacho
// aqui embaixo se bifurca.
//
// A detecção é por presença de `window.__TAURI__`, injetado pelo webview
// do Tauri antes de qualquer script da página rodar. Nunca por user-agent:
// o webview do Tauri no Linux é WebKit, o mesmo do Safari, e distinguir
// por string de navegador erraria exatamente no caso que importa.

(function () {
  "use strict";

  const noTauri = typeof window.__TAURI__ !== "undefined";

  if (noTauri) {
    // Desktop: o Tauri já expõe o `invoke` que fala com `#[tauri::command]`.
    window.ODEROM_invoke = window.__TAURI__.core.invoke;
    window.ODEROM_backend = "tauri";
    return;
  }

  // Navegador: o Rust está compilado para wasm. O módulo é carregado sob
  // demanda -- e não no topo do arquivo -- porque `backend.js` também é
  // carregado pelo desktop, onde o .wasm não existe e um import estático
  // falharia a página inteira antes de o Tauri ter chance de responder.
  window.ODEROM_backend = "wasm";

  let modulo = null;
  const carregando = (async () => {
    const wasm = await import("./wasm/oderom_wasm.js");
    await wasm.default();
    modulo = wasm;
    return wasm;
  })();

  window.ODEROM_invoke = async function (comando, args) {
    const wasm = modulo || (await carregando);
    const fn = wasm[comando];
    if (typeof fn !== "function") {
      // Falha alta e nomeando o comando: o modo de falha que este arquivo
      // existe para evitar é o frontend compartilhado chamar algo que só
      // um dos backends implementa, e descobrir isso na máquina do aluno.
      throw new Error(
        `comando "${comando}" não existe no backend wasm -- ` +
          `implemente-o em oderom-wasm ou veja LEIA-ME.md neste diretório`
      );
    }
    return fn(args || {});
  };
})();
