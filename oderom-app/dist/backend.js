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

  // ---------------------------------------------------------------
  // O worker: onde o Rust de fato roda.
  //
  // O wasm NÃO é carregado nesta thread. Ele vive em `worker.js`, e tudo
  // aqui é troca de mensagens. Duas coisas dependem disso:
  //
  //   1. a aba não congela durante uma conta longa -- a thread que
  //      calcula não é a que desenha;
  //   2. cancelar passa a ser possível. Em `wasm32` não há como
  //      interromper a conta por dentro (`panic = "abort"`, sem unwind,
  //      ver `oderom-expr/src/cancel.rs`), e o sinal por `Atomics` sobre
  //      `SharedArrayBuffer` exigiria cabeçalhos COOP/COEP que o GitHub
  //      Pages não deixa configurar. Sobra `terminate()`, que é chamado
  //      DAQUI e não precisa que o worker coopere.
  // ---------------------------------------------------------------
  let worker = null;
  let prontoDoWorker = null;
  let proximaSeq = 1;
  const pendentes = new Map();

  function criarWorker() {
    const w = new Worker("worker.js", { type: "module" });
    prontoDoWorker = new Promise((resolve) => {
      w.addEventListener("message", function aoPronto(e) {
        if (e.data && e.data.pronto) {
          w.removeEventListener("message", aoPronto);
          resolve();
        }
      });
    });
    w.addEventListener("message", (e) => {
      const { seq, ok, valor, erro } = e.data || {};
      const pendente = pendentes.get(seq);
      if (!pendente) return;
      pendentes.delete(seq);
      ok ? pendente.resolve(valor) : pendente.reject(new Error(erro));
    });
    // Um worker que morre sem responder deixaria toda promise pendente
    // esperando para sempre -- e um `await` que nunca resolve não produz
    // erro nenhum, o que o torna muito pior de diagnosticar do que uma
    // falha barulhenta. Acontece de verdade: basta o `.wasm` faltar, ou
    // ser servido com o Content-Type errado.
    w.addEventListener("error", (e) => {
      const onde = e.filename ? ` (${e.filename}:${e.lineno})` : "";
      descartarPendentes(`o worker do ODEROM falhou: ${e.message || "erro desconhecido"}${onde}`);
    });
    worker = w;
    return w;
  }
  criarWorker();

  // Os comandos do wasm falam JSON como texto nas duas pontas (ver o doc
  // comment do `oderom-wasm/src/lib.rs`: assim a serialização é a mesma
  // `serde_json`, sobre os mesmos tipos do `oderom-ui`, que o Tauri usa
  // do outro lado). A conversão em si acontece no worker; aqui só se
  // despacha e se espera.
  function chamarWasm(comando, args) {
    const seq = proximaSeq++;
    return new Promise((resolve, reject) => {
      pendentes.set(seq, { resolve, reject });
      worker.postMessage({ seq, comando, args: args || {} });
    });
  }

  // ---------------------------------------------------------------
  // Execução e cancelamento.
  //
  // O contrato do Tauri é: `execute_block` volta NA HORA, o bloco passa a
  // aparecer como `running`, e `cancel_block` interrompe. Reproduzi-lo
  // aqui é o que permite o `notebook.js` continuar sem saber onde está --
  // ele já foi escrito para esse contrato, com `pollUntilSettled`
  // perguntando por `list_blocks` até o bloco assentar.
  //
  // O truque é que, enquanto uma conta roda, o worker não responde a
  // nada (é uma thread só, ocupada). Então `list_blocks` não vai até ele:
  // é respondido daqui, a partir do último resultado conhecido, com o
  // bloco em execução marcado `running`. Isso não é inventar estado --
  // é relatar o que de fato está acontecendo, e é a única fonte
  // disponível enquanto o worker calcula.
  // ---------------------------------------------------------------
  let snapshot = null;      // último `list_blocks` que veio do worker
  let emExecucao = null;    // { id, texto, contagem } enquanto há conta em voo
  // Qual bloco foi cancelado por último. Precisa sobreviver às consultas
  // seguintes: o worker que morreu levou junto a memória de que houve um
  // cancelamento, e o worker novo responde `NeverRun` para todo mundo --
  // sem isto, o bloco cancelado apareceria como se nunca tivesse rodado
  // e o aluno não veria que a sua interrupção surtiu efeito.
  let idCancelado = null;

  // Rejeita o que estava na fila do worker que acabou de morrer.
  // (Usada por `criarWorker` acima, que roda antes desta linha --
  // declaração de função, portanto içada.) Sem
  // isto, uma promise pendente nunca resolve e o `await` de quem a
  // esperava fica preso para sempre -- o pior tipo de bug, porque não
  // produz erro nenhum.
  function descartarPendentes(motivo) {
    for (const { reject } of pendentes.values()) reject(new Error(motivo));
    pendentes.clear();
  }

  // O DTO de um bloco que está executando agora, montado sobre o que ele
  // era no snapshot. Os campos são os do `oderom_ui::BlockDto`.
  function marcarRodando(bloco, contagem) {
    return {
      ...bloco,
      execution_count: contagem,
      output: { kind: "Attempt", state: "running", previous: null },
    };
  }

  function marcarCancelado(bloco) {
    return { ...bloco, output: { kind: "Attempt", state: "cancelled", previous: null } };
  }

  async function listarBlocos() {
    if (emExecucao) {
      // O worker está ocupado. Responde-se do snapshot, marcando o bloco
      // que está rodando -- é o que faz o gutter mostrar a execução em
      // curso e o botão "Cancelar" aparecer.
      return {
        ...snapshot,
        blocks: snapshot.blocks.map((b) =>
          b.id === emExecucao.id ? marcarRodando(b, emExecucao.contagem) : b
        ),
      };
    }
    snapshot = await chamarWasm("list_blocks", {});
    if (idCancelado !== null) {
      snapshot = {
        ...snapshot,
        blocks: snapshot.blocks.map((b) => (b.id === idCancelado ? marcarCancelado(b) : b)),
      };
    }
    return snapshot;
  }

  const execucao = {
    async execute_block({ id }) {
      // No máximo uma execução por vez, igual ao desktop
      // (`Notebook::begin_execute`): o pedido que chega durante outra é
      // recusado e nomeia quem está ocupando.
      if (emExecucao) return { kind: "Blocked", by: emExecucao.id };
      idCancelado = null;

      const atual = await listarBlocos();
      const bloco = atual.blocks.find((b) => b.id === id);
      if (!bloco) return { kind: "NotFound" };

      // O texto do caderno é guardado ANTES de começar, porque cancelar
      // mata o worker e leva o estado junto -- é por ele que o caderno é
      // reconstruído. Pedir ao Rust em vez de remontar aqui evita ter uma
      // segunda implementação do formato de arquivo neste arquivo.
      const texto = await chamarWasm("notebook_text", {});

      // O próximo número de `In [n]`. O desktop obtém isso do contador do
      // `Notebook`; daqui, o maior já usado mais um dá o mesmo resultado.
      const contagem =
        Math.max(0, ...atual.blocks.map((b) => b.execution_count || 0)) + 1;
      // A POSIÇÃO do bloco, além do id. Cancelar recarrega o caderno a
      // partir do texto, e o `Notebook` reconstruído numera os blocos de
      // zero -- o id de agora não sobrevive. A posição sobrevive, porque
      // o texto é o mesmo e a ordem dos blocos é o que o arquivo grava.
      const indice = atual.blocks.findIndex((b) => b.id === id);
      emExecucao = { id, texto, contagem, indice };

      // Deliberadamente SEM `await`: é isto que faz o comando voltar na
      // hora, deixando a página livre para desenhar o `running` e para
      // receber o clique em "Cancelar".
      chamarWasm("execute_block", { id })
        .then(async () => {
          snapshot = await chamarWasm("list_blocks", {});
          emExecucao = null;
        })
        .catch(() => {
          // O worker morreu (cancelamento) ou o comando falhou. Quem
          // cuida do estado nesse caso é o `cancel_block`; aqui só se
          // evita deixar a promise sem tratamento.
          emExecucao = null;
        });

      return { kind: "Ok" };
    },

    async cancel_block({ id }) {
      if (!emExecucao || emExecucao.id !== id) return null;
      const { texto, indice } = emExecucao;

      // A única forma de parar uma conta em wasm sem unwind e sem
      // SharedArrayBuffer. O worker morre no meio, seja lá onde estiver.
      worker.terminate();
      descartarPendentes("execução cancelada");
      emExecucao = null;

      criarWorker();
      await prontoDoWorker;
      // O worker novo nasce com o caderno de exemplo; recarrega-se o
      // texto de antes. Os RESULTADOS já calculados não voltam -- eles
      // moravam no worker morto -- e é por isso que cancelar, aqui,
      // equivale a cancelar e limpar a execução. Está registrado em
      // LEIA-ME.md.
      await chamarWasm("load_notebook_text", { texto });
      const refeito = await chamarWasm("list_blocks", {});
      snapshot = refeito;
      // O id do bloco que estava rodando, no caderno reconstruído.
      const agora = refeito.blocks[indice];
      idCancelado = agora ? agora.id : null;
      if (idCancelado !== null) {
        snapshot = {
          ...refeito,
          blocks: refeito.blocks.map((b) => (b.id === idCancelado ? marcarCancelado(b) : b)),
        };
      }
      return null;
    },

    list_blocks: () => listarBlocos(),
  };

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
      let nome = (path || "caderno.od").split(/[/\\]/).pop();
      if (!nome.endsWith(".od")) nome += ".od";
      const url = URL.createObjectURL(new Blob([texto], { type: "text/plain" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = nome;
      a.click();
      // Sem o revoke o Blob fica na memória da aba até ela fechar; o
      // atraso existe porque o download começa depois do clique, e
      // revogar na mesma volta do event loop o cancelaria no Firefox.
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
      // O caderno passa a se chamar assim -- a metade que o `save` do
      // desktop faz junto com a escrita. Sem isto o cabeçalho continua
      // dizendo "sem título" depois de salvar.
      return chamarWasm("set_current_name", { nome });
    },

    // `path` é ignorado, e é ignorado porque não pode ser honrado: uma
    // página não lê um arquivo por caminho, só um que o usuário escolheu
    // no seletor do sistema. Fingir o contrário -- ler algo diferente do
    // que o campo diz -- seria pior que a assimetria.
    async open_notebook() {
      const escolhido = await escolherArquivo();
      // Cancelar o seletor não é um erro: resolve sem mudar nada, e o
      // `refresh()` que o `notebook.js` faz em seguida redesenha o mesmo
      // caderno. Rejeitar aqui mostraria um "Erro ao abrir" para quem só
      // mudou de ideia.
      if (escolhido === null) return null;
      // O nome vem do arquivo que o aluno escolheu, e não do campo de
      // texto: é o que ele acabou de abrir. Sem ele o caderno abre certo
      // e o cabeçalho continua dizendo "sem título", o que parece que
      // não abriu.
      return chamarWasm("load_notebook_text", escolhido);
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

  // Resolve com `{ texto, nome }` -- os dois argumentos que o
  // `load_notebook_text` do wasm espera -- ou `null` se o aluno
  // cancelou.
  function escolherArquivo() {
    return new Promise((resolve) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = ".od,text/plain";
      input.addEventListener("change", async () => {
        const arquivo = input.files && input.files[0];
        resolve(arquivo ? { texto: await arquivo.text(), nome: arquivo.name } : null);
      });
      // `cancel` é suportado por Chrome/Firefox/Safari recentes; onde não
      // for, a promise fica pendente e o aluno simplesmente clica de novo
      // -- nada quebra, o botão continua funcionando.
      input.addEventListener("cancel", () => resolve(null));
      input.click();
    });
  }

  window.ODEROM_invoke = async function (comando, args) {
    const especial = execucao[comando] || assimetricos[comando];
    if (especial) return especial(args || {});
    // Qualquer outro comando muda o caderno, então o snapshot guardado
    // deixa de valer -- descartá-lo força o próximo `list_blocks` a
    // perguntar ao worker de verdade. Durante uma execução ele é
    // mantido: o worker está ocupado e não responderia, e sem snapshot o
    // `pollUntilSettled` ficaria preso na fila até a conta acabar --
    // justamente o congelamento que este worker existe para evitar.
    if (!emExecucao) snapshot = null;
    if (comando === "clear_execution" || comando === "new_notebook" || comando === "load_notebook_text") {
      idCancelado = null;
    }
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
    if (campo && !campo.value) campo.value = "caderno.od";
  });
})();
