// O Rust roda AQUI, e não na thread da página.
//
// Este arquivo é curto de propósito: ele não decide nada. Recebe
// `{seq, comando, args}`, chama a função de mesmo nome no módulo wasm, e
// devolve `{seq, ok, valor}` ou `{seq, ok:false, erro}`. Quem orquestra é
// o `backend.js`; ver o LEIA-ME deste diretório.
//
// Existir separado é o que impede a aba de congelar: uma conta longa
// ocupa esta thread, e a página segue rolando, clicando e desenhando. É
// também o que torna o cancelamento possível -- `worker.terminate()` é
// chamado do lado de lá e não depende deste código cooperar, o que
// importa porque em `wasm32` não há como interromper a conta por dentro
// (`panic = "abort"`, sem unwind) e `SharedArrayBuffer` -- que permitiria
// um sinal via `Atomics` -- exige cabeçalhos COOP/COEP que o GitHub Pages
// não deixa configurar.

let wasm = null;

// O módulo é carregado uma vez, na primeira mensagem, e o `pronto` avisa
// o `backend.js` de que este worker já pode receber trabalho -- ele
// espera esse aviso antes de restaurar estado num worker recém-criado.
const carregando = (async () => {
  wasm = await import("./wasm/oderom_wasm.js");
  await wasm.default();
  self.postMessage({ pronto: true });
})();

self.onmessage = async (evento) => {
  const { seq, comando, args } = evento.data;
  try {
    await carregando;
    const fn = wasm[comando];
    if (typeof fn !== "function") {
      throw new Error(
        `comando "${comando}" não existe no backend wasm -- ` +
          `implemente-o em oderom-wasm ou veja LEIA-ME.md neste diretório`
      );
    }
    self.postMessage({ seq, ok: true, valor: JSON.parse(fn(JSON.stringify(args || {}))) });
  } catch (e) {
    // `Error` não atravessa `postMessage` com a mensagem intacta em todo
    // navegador; a string atravessa.
    self.postMessage({ seq, ok: false, erro: String((e && e.message) || e) });
  }
};
