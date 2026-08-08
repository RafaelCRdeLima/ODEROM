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

  const temTauri = typeof window.__TAURI__ !== "undefined";

  if (temTauri) {
    // Desktop: o Tauri já expõe o `invoke` que fala com `#[tauri::command]`.
    window.ODEROM_invoke = window.__TAURI__.core.invoke;
    window.ODEROM_backend = "tauri";
    return;
  }

  // ---------------------------------------------------------------
  // Navegador: o Rust está compilado para wasm (crate `oderom-wasm`).
  // ---------------------------------------------------------------
  window.ODEROM_backend = "wasm";

  // O módulo é carregado sob demanda -- e não no topo do arquivo --
  // porque `backend.js` também é carregado pelo desktop, onde o .wasm não
  // existe e um import estático falharia a página inteira antes de o
  // Tauri ter chance de responder.
  let modulo = null;
  const carregando = (async () => {
    const wasm = await import("./wasm/oderom_wasm.js");
    await wasm.default();
    modulo = wasm;
    return wasm;
  })();

  // Os comandos do wasm falam JSON como texto nas duas pontas (ver o doc
  // comment do `oderom-wasm/src/lib.rs`: assim a serialização é a mesma
  // `serde_json`, sobre os mesmos tipos do `oderom-ui`, que o Tauri usa
  // do outro lado). Esta função é a tradução, e o único lugar dela.
  async function chamarWasm(comando, args) {
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
    return JSON.parse(fn(JSON.stringify(args || {})));
  }

  // ---------------------------------------------------------------
  // Os três comandos assimétricos.
  //
  // Onze dos catorze são tradução direta. Estes três pedem algo que só o
  // hospedeiro sabe fazer -- gravar num caminho, ler de um caminho, tocar
  // a área de transferência do sistema -- e no navegador nenhuma dessas
  // três coisas existe na forma que o Tauri oferece. O Rust faz a metade
  // que é dele (`notebook_text`, `load_notebook_text`) e a metade do
  // navegador está aqui.
  //
  // O contrato visto pelo `notebook.js` é idêntico ao do Tauri: mesmos
  // nomes, mesmos argumentos, promise que resolve em sucesso e rejeita
  // em erro. Ele não sabe que esta tradução existe, e não deve saber.
  // ---------------------------------------------------------------
  const assimetricos = {
    // O `path` digitado pelo aluno vira o nome do arquivo baixado. Não há
    // "gravar em um caminho" no navegador: o que existe é oferecer um
    // download, e o nome é a única parte do caminho que sobrevive.
    async save_notebook({ path }) {
      const texto = await chamarWasm("notebook_text", {});
      const nome = (path || "caderno.oderom").split(/[/\\]/).pop();
      const url = URL.createObjectURL(new Blob([texto], { type: "text/plain" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = nome.endsWith(".oderom") ? nome : nome + ".oderom";
      a.click();
      // Sem o revoke o Blob fica na memória da aba até ela fechar; o
      // atraso existe porque o download começa depois do clique, e
      // revogar na mesma volta do event loop o cancelaria no Firefox.
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
      return null;
    },

    // `path` é ignorado, e é ignorado porque não pode ser honrado: uma
    // página não lê um arquivo por caminho, só um que o usuário escolheu
    // no seletor do sistema. Fingir o contrário -- ler algo diferente do
    // que o campo diz -- seria pior que a assimetria.
    async open_notebook() {
      const texto = await escolherArquivo();
      // Cancelar o seletor não é um erro: resolve sem mudar nada, e o
      // `refresh()` que o `notebook.js` faz em seguida redesenha o mesmo
      // caderno. Rejeitar aqui mostraria um "Erro ao abrir" para quem só
      // mudou de ideia.
      if (texto === null) return null;
      return chamarWasm("load_notebook_text", { texto });
    },

    async copy_to_clipboard({ text }) {
      // `navigator.clipboard` exige contexto seguro (https ou localhost).
      // O GitHub Pages é https, então isto vale em produção; num `file://`
      // aberto direto do disco não vale, e a mensagem diz isso em vez de
      // deixar um `undefined is not a function` na cara do aluno.
      if (!navigator.clipboard) {
        throw new Error(
          "a área de transferência exige uma página servida por https " +
            "(ou localhost) -- abrir o arquivo direto do disco não basta"
        );
      }
      await navigator.clipboard.writeText(text);
      return null;
    },
  };

  function escolherArquivo() {
    return new Promise((resolve) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = ".oderom,text/plain";
      input.addEventListener("change", async () => {
        const arquivo = input.files && input.files[0];
        resolve(arquivo ? await arquivo.text() : null);
      });
      // `cancel` é suportado por Chrome/Firefox/Safari recentes; onde não
      // for, a promise fica pendente e o aluno simplesmente clica de novo
      // -- nada quebra, o botão continua funcionando.
      input.addEventListener("cancel", () => resolve(null));
      input.click();
    });
  }

  window.ODEROM_invoke = async function (comando, args) {
    const assimetrico = assimetricos[comando];
    if (assimetrico) return assimetrico(args || {});
    return chamarWasm(comando, args);
  };

  // O campo de caminho começa vazio, e os botões "Salvar"/"Abrir" do
  // `notebook.js` não fazem nada enquanto estiver vazio (`if (!path)
  // return`). No desktop isso está certo: sem caminho não há onde gravar.
  // No navegador, onde "caminho" é só o nome do download, um campo vazio
  // vira um botão que parece quebrado. Preencher um padrão resolve sem
  // que o `notebook.js` precise saber onde está rodando -- a adaptação
  // fica deste lado da fronteira, que é onde ela pertence.
  window.addEventListener("DOMContentLoaded", () => {
    const campo = document.getElementById("path-input");
    if (campo && !campo.value) campo.value = "caderno.oderom";
  });
})();
