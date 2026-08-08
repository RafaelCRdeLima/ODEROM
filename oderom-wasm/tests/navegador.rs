//! Roda a versão web num navegador de verdade e verifica que ela
//! funciona -- o análogo, para o navegador, do que `oderom-app`'s
//! `tests/keymap.rs` faz para o app de desktop.
//!
//! Este teste existe porque os testes de `src/cmd.rs` rodam no host, e
//! passar lá não prova nada sobre wasm. A primeira versão deste backend
//! passava em todos eles e mesmo assim abortava no primeiro
//! Shift+Enter do aluno: `std::time::Instant::now()`, no meio do
//! `oderom-session`, é um `unreachable` em `wasm32-unknown-unknown` (a
//! razão de `oderom-core::clock` existir). Nenhum teste de host podia
//! encontrar isso. Este encontra.
//!
//! O que ele exercita é o caminho inteiro e real: o `construir.sh`, o
//! `.wasm` de release, a ponte gerada pelo `wasm-bindgen`, o
//! `backend.js`, e o `notebook.js`/`index.html` que o aluno vê -- nunca
//! uma reimplementação de nenhuma dessas peças.
//!
//! # Quando o teste não pode rodar
//!
//! Precisa de duas coisas fora do cargo: o `wasm-bindgen` CLI e um
//! Chrome/Chromium. Faltando qualquer uma, o teste passa com uma
//! mensagem dizendo o que instalar, em vez de falhar. Um teste de
//! ambiente que falha vermelho na máquina de quem não mexe na versão
//! web treina todo mundo a ignorar vermelho -- e aí ele deixa de
//! proteger o dia em que quebrar de verdade.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;

fn raiz() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// O primeiro Chrome/Chromium encontrado, ou `None`.
fn navegador() -> Option<&'static str> {
    ["google-chrome", "chromium", "chromium-browser", "google-chrome-stable"]
        .into_iter()
        .find(|nome| Command::new("which").arg(nome).output().is_ok_and(|o| o.status.success()))
}

/// Serve `dir` numa porta livre até `parar` virar verdadeiro.
///
/// Sessenta linhas de servidor em vez de uma dependência: o teste
/// precisa de HTTP porque `import()` de módulo ES não funciona por
/// `file://`, e precisa dos Content-Type certos porque o navegador
/// recusa um módulo ES servido como `text/plain` e recusa
/// `instantiateStreaming` sobre qualquer coisa que não seja
/// `application/wasm`. Nada além disso.
/// `encodeURIComponent` do lado da página, desfeito aqui. Só o que essa
/// função de fato produz: `%XX` e `+` nunca aparece (ela codifica espaço
/// como `%20`).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut saida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                saida.push(b);
                i += 3;
                continue;
            }
        }
        saida.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&saida).into_owned()
}

/// O que a página envia de volta por `GET /relatorio?d=...`.
type Caixa = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

fn servir(dir: PathBuf) -> (u16, std::sync::Arc<std::sync::atomic::AtomicBool>, Caixa) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    let listener = TcpListener::bind("127.0.0.1:0").expect("nao consegui abrir uma porta local");
    let porta = listener.local_addr().unwrap().port();
    let parar = Arc::new(AtomicBool::new(false));
    let sinal = parar.clone();
    let caixa: Caixa = Arc::new(Mutex::new(Vec::new()));
    let minha = caixa.clone();

    std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !sinal.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((fluxo, _)) => {
                    let dir = dir.clone();
                    let caixa = minha.clone();
                    std::thread::spawn(move || atender(fluxo, &dir, &caixa));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    (porta, parar, caixa)
}

/// Abre `url` num Chrome de verdade e espera a página reportar.
///
/// Substituiu o par `--virtual-time-budget`/`--dump-dom` que este
/// arquivo usava, e a troca não foi por gosto: o relógio virtual do
/// Chrome adianta os temporizadores **da página**, e o `oderom-wasm`
/// passou a rodar num Web Worker, que é outra thread e continua no tempo
/// real. O `--dump-dom` disparava antes de o worker responder qualquer
/// coisa, e o teste lia uma página vazia -- um falso negativo silencioso
/// que custou meia hora de diagnóstico.
///
/// Agora a página avisa quando terminou, por uma requisição ao mesmo
/// servidor que a serviu, e o teste espera por esse aviso.
fn rodar_e_esperar(chrome: &str, url: &str, caixa: &Caixa, limite: std::time::Duration) -> String {
    // Perfil próprio, e descartável, por execução. Sem isto o Chrome
    // reaproveita o perfil padrão, e uma segunda instância (ou uma
    // sobra de execução anterior que não morreu) recusa-se a subir --
    // falha que aparece como "a pagina nao reportou nada", sem dizer
    // por quê.
    let perfil = std::env::temp_dir().join(format!("oderom-teste-chrome-{}-{}", std::process::id(), url.len()));
    let _ = std::fs::remove_dir_all(&perfil);
    // O que o Chrome escreve é guardado, não descartado: quando a página
    // não reporta, a razão costuma estar aqui (um módulo que não carregou,
    // um MIME recusado), e sem isto o teste só sabe dizer "não veio nada".
    let log = perfil.with_extension("log");
    let saida = std::fs::File::create(&log).expect("nao consegui criar o log do navegador");
    let mut filho = Command::new(chrome)
        .args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--enable-logging=stderr",
            "--v=0",
            &format!("--user-data-dir={}", perfil.display()),
            url,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(saida))
        .spawn()
        .expect("nao consegui rodar o navegador");

    let inicio = std::time::Instant::now();
    let relato = loop {
        if let Some(r) = caixa.lock().unwrap().first().cloned() {
            break r;
        }
        if inicio.elapsed() > limite {
            let diagnostico = std::fs::read_to_string(&log).unwrap_or_default();
            let ultimas: Vec<&str> = diagnostico.lines().rev().take(25).collect();
            break format!(
                "ERRO: a pagina nao reportou nada dentro do limite.\nUltimas linhas do navegador:\n{}",
                ultimas.into_iter().rev().collect::<Vec<_>>().join("\n")
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let _ = filho.kill();
    let _ = filho.wait();
    let _ = std::fs::remove_dir_all(&perfil);
    let _ = std::fs::remove_file(&log);
    relato
}

fn atender(mut fluxo: TcpStream, dir: &Path, caixa: &Caixa) {
    let mut linha = String::new();
    if BufReader::new(fluxo.try_clone().unwrap()).read_line(&mut linha).is_err() {
        return;
    }
    let alvo = linha.split_whitespace().nth(1).unwrap_or("/").to_string();

    // O canal de volta da página para o teste.
    if let Some(consulta) = alvo.strip_prefix("/relatorio?d=") {
        caixa.lock().unwrap().push(percent_decode(consulta));
        let _ = fluxo.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        return;
    }

    let caminho = alvo.split('?').next().unwrap_or("/");
    let caminho = if caminho == "/" { "/index.html" } else { caminho };

    // Sem `..`: este servidor só existe dentro do teste, mas um path
    // traversal aqui serviria qualquer arquivo da máquina de quem roda
    // `cargo test`, e "é só um teste" nao e' motivo para escrever isso.
    let relativo = caminho.trim_start_matches('/');
    if relativo.split('/').any(|p| p == ".." || p.is_empty()) {
        let _ = fluxo.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        return;
    }

    let arquivo = dir.join(relativo);
    let Ok(mut f) = std::fs::File::open(&arquivo) else {
        let _ = fluxo.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        return;
    };
    let mut corpo = Vec::new();
    if f.read_to_end(&mut corpo).is_err() {
        return;
    }

    let tipo = match arquivo.extension().and_then(|e| e.to_str()) {
        // Os dois que o navegador realmente exige estar certos.
        Some("js") | Some("mjs") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css",
        _ => "application/octet-stream",
    };
    let cabecalho = format!("HTTP/1.1 200 OK\r\nContent-Type: {tipo}\r\nContent-Length: {}\r\n\r\n", corpo.len());
    let _ = fluxo.write_all(cabecalho.as_bytes());
    let _ = fluxo.write_all(&corpo);
}

/// O driver: carrega os MESMOS scripts que o `index.html` real carrega
/// (nunca uma reimplementação -- a mesma convenção do `keytest.html` do
/// desktop), exercita os comandos e escreve um relatório no `<body>`,
/// que o `--dump-dom` do Chrome devolve para o Rust.
const DRIVER: &str = r#"<!doctype html><meta charset="utf-8"><body>
<script>
const L = [];
const p = (m) => L.push(m);
const enviar = () => fetch("/relatorio?d=" + encodeURIComponent(L.join(" | ")));
window.onerror = (m, s, l) => { p("ERRO: " + m + " @" + s + ":" + l); enviar(); };
window.addEventListener("unhandledrejection", e => {
  p("ERRO: rejeicao nao tratada: " + (e.reason && (e.reason.message || e.reason))); enviar();
});
</script>
<script src="backend.js"></script>
<script>
(async () => {
  try {
    const inv = window.ODEROM_invoke;
    p("backend=" + window.ODEROM_backend);

    // Espera um bloco sair de `running`. Necessario porque, com o worker,
    // `execute_block` volta ANTES de a conta acabar (igual ao desktop):
    // disparar o proximo sem esperar levaria `Blocked`, de propriedade.
    const assentar = async (id) => {
      const t0 = Date.now();
      while (Date.now() - t0 < 120000) {
        const x = await inv("list_blocks");
        const bb = x.blocks.find(y => y.id === id);
        if (!(bb && bb.output.kind === "Attempt" && bb.output.state === "running")) return true;
        await new Promise(r => setTimeout(r, 40));
      }
      p("ERRO: bloco " + id + " nunca assentou");
      return false;
    };

    // O caderno inicial, e cada bloco dele executado de verdade.
    let n = await inv("list_blocks");
    p("blocos=" + n.blocks.length);
    for (const b of n.blocks) {
      const r = await inv("execute_block", { id: b.id });
      if (r.kind !== "Ok") { p("ERRO: execute retornou " + r.kind); break; }
      await assentar(b.id);
    }
    n = await inv("list_blocks");
    const q = n.blocks.find(b => b.output.kind === "Query");
    p("query.state=" + (q && q.output.state));
    p("latex=" + (q && q.output.latex));

    // As opcoes do seletor "Exportar" vem da gramatica, nao de uma
    // lista escrita no JS -- se `export_options` sumir ou vier vazio, o
    // botao abre um painel sem nada dentro.
    const eo = await inv("export_options");
    p("alvos=" + eo.targets.map(t => t.keyword).sort().join(","));
    p("consultas=" + eo.queries.length);
    p("param=" + eo.queries.filter(q => q.needs_param).map(q => q.keyword).sort().join(","));

    // Galeria, ida-e-volta de arquivo, e os comandos de estado.
    const g = await inv("gallery_list");
    p("galeria=" + g.length);
    p("load_gallery=" + (await inv("load_gallery", { after: null, name: g[0].name })).length);
    const texto = await inv("notebook_text");
    await inv("new_notebook");
    await inv("load_notebook_text", { texto });
    p("reabriu=" + (await inv("list_blocks")).blocks.length);

    // O nome do caderno: no navegador nao ha caminho de arquivo, so o
    // nome, e ele precisa chegar ao Rust pelos dois caminhos (abrir e
    // salvar) senao o cabecalho fica em "sem titulo" para sempre.
    p("sem-nome=" + JSON.stringify((await inv("list_blocks")).current_path));
    await inv("set_current_name", { nome: "salvo.od" });
    p("apos-salvar=" + JSON.stringify((await inv("list_blocks")).current_path));
    await inv("load_notebook_text", { texto, nome: "aberto.od" });
    p("apos-abrir=" + JSON.stringify((await inv("list_blocks")).current_path));
    await inv("clear_execution");
    await inv("frontend_ready");

    // O worker: `execute_block` volta na hora, o bloco fica `running`, a
    // pagina segue respondendo, e cancelar de fato interrompe. Sem o
    // worker nada disso e' possivel -- ver `dist/worker.js`.
    let l = await inv("list_blocks");
    for (const b of l.blocks.slice(0, 3)) {
      await inv("execute_block", { id: b.id });
      await assentar(b.id);
    }
    const pesado = await inv("create_block", { after: null, source: "gaussbonnet" });
    const t0 = Date.now();
    const saida = await inv("execute_block", { id: pesado });
    p("execute-imediato=" + (Date.now() - t0 < 500) + " kind=" + saida.kind);
    const t1 = Date.now();
    const durante = (await inv("list_blocks")).blocks.find(b => b.id === pesado);
    p("responde-durante=" + (Date.now() - t1 < 500));
    p("marca-running=" + (durante.output.kind === "Attempt" && durante.output.state === "running"));
    const segundo = await inv("execute_block", { id: l.blocks[0].id });
    p("segundo-recusado=" + (segundo.kind === "Blocked"));
    // Pela POSICAO, nao pelo id: cancelar reconstroi o caderno a partir
    // do texto, e o `Notebook` novo numera os blocos de zero. O
    // `notebook.js` nao se importa (ele redesenha tudo a cada
    // `refresh()`), mas quem guardar um id do lado de fora precisa saber.
    const posicao = (await inv("list_blocks")).blocks.findIndex(b => b.id === pesado);
    await inv("cancel_block", { id: pesado });
    const depois = await inv("list_blocks");
    const cancelado = depois.blocks[posicao];
    p("cancelado=" + (cancelado.output.kind === "Attempt" && cancelado.output.state === "cancelled"));
    p("texto-sobreviveu=" + depois.blocks.length);
    p("texto-do-cancelado=" + JSON.stringify(cancelado.source));

    // Um comando que so o outro backend tem precisa falhar alto aqui,
    // e nomeando o comando -- e' o modo de falha que o backend.js
    // existe para evitar.
    try { await inv("comando_inexistente"); p("ERRO: deveria ter lancado"); }
    catch (e) { p("desconhecido-lanca=" + /comando_inexistente/.test(e.message)); }

    p("FIM");
  } catch (e) {
    p("ERRO: " + (e && (e.message) || e));
  } finally {
    // `finally`, e nao depois do `catch`: um `return` antecipado dentro
    // do `try` pularia o envio, e o teste veria "a pagina nao reportou
    // nada" -- que nao diz nada sobre a causa. Aconteceu.
    enviar();
  }
})();
</script></body>"#;

/// Dirige o seletor "Exportar" da página REAL com cliques sintéticos.
///
/// Anexado ao `index.html` de verdade (a mesma convenção do
/// `keytest.html` do desktop: os scripts são os que o aluno carrega,
/// não uma reimplementação), porque o que este teste precisa provar é
/// que os `addEventListener` do `notebook.js` estão ligados aos ids do
/// `index.html` -- e isso um teste sobre o backend nunca veria.
///
/// **Nunca espere um número de milissegundos aqui, espere a condição**
/// (`ateQue`). A primeira versão deste driver dormia 1200 ms depois do
/// clique e lia o texto do bloco: passava sozinha e falhava dentro de
/// `cargo test --workspace`, onde a CPU está ocupada com o resto da
/// suíte e o CodeMirror demora mais para pintar. Um `sleep` calibrado
/// na máquina de quem escreveu é um teste que reprova por carga, não
/// por defeito -- e um teste que reprova à toa é um teste que as
/// pessoas aprendem a ignorar.
const DRIVER_UI: &str = r#"
<script>
window.__r = [];
const p = m => window.__r.push(m);
const enviar = () => fetch("/relatorio?d=" + encodeURIComponent(window.__r.join(" | ")));
window.onerror = (m, s, l) => { p("ERRO: " + m + " @" + s + ":" + l); enviar(); };
window.addEventListener("unhandledrejection", e => {
  p("ERRO: " + (e.reason && (e.reason.message) || e.reason)); enviar();
});
const esperar = ms => new Promise(r => setTimeout(r, ms));
// Espera a CONDICAO, nao um numero de milissegundos. Um `sleep`
// calibrado passa na maquina de quem o escreveu e falha na suite
// inteira, quando a CPU esta ocupada com o resto dos testes -- foi
// exatamente assim que este arquivo falhou da primeira vez, esperando
// o CodeMirror pintar o texto de um bloco recem-criado.
async function ateQue(nome, cond, limite = 15000) {
  const t0 = Date.now();
  while (Date.now() - t0 < limite) {
    if (cond()) return true;
    await esperar(50);
  }
  p("ERRO: tempo esgotado esperando " + nome);
  return false;
}
window.addEventListener("load", async () => {
  try {
    await ateQue("o caderno desenhar", () => document.querySelectorAll(".block-editor").length > 0);
    document.getElementById("export-btn").click();
    await ateQue("o painel abrir", () => document.querySelectorAll(".export-query").length > 0);
    p("aberto=" + !document.getElementById("export-panel").hidden);
    const previas = () => [...document.querySelectorAll(".export-query-preview")].map(c => c.textContent);
    p("alvos=" + [...document.querySelectorAll(".export-target")].map(b => b.textContent).sort().join(","));
    p("consultas=" + previas().length);
    p("geodesic=" + previas().find(c => c.includes("geodesic")));
    // Trocar o formato reescreve TODAS as previas, nao so a primeira.
    const sympy = [...document.querySelectorAll(".export-target")].find(b => b.textContent === "sympy");
    sympy.click();
    await ateQue("as previas virarem sympy", () => previas().every(c => c.startsWith("export sympy ")));
    p("todas-sympy=" + previas().every(c => c.startsWith("export sympy ")));

    // Clicar numa consulta insere o bloco e fecha o painel.
    const antes = document.querySelectorAll(".block-editor").length;
    [...document.querySelectorAll(".export-query")].find(b => b.textContent.includes("kretschmann")).click();
    await ateQue("o painel fechar", () => document.getElementById("export-panel").hidden);
    p("fechado=" + document.getElementById("export-panel").hidden);

    // O RENDER primeiro, e so depois o backend -- nao por gosto, por
    // ordem: `insertExportBlock` fecha o painel na primeira linha, antes
    // de criar o bloco, entao "o painel fechou" nao significa que o
    // bloco ja existe. Esperar o bloco aparecer na tela espera o
    // `refresh()`, que so acontece depois do `create_block`.
    await ateQue("o bloco novo aparecer na tela", () => {
      const eds = document.querySelectorAll(".block-editor");
      return eds.length === antes + 1 && eds[eds.length - 1].innerText.includes("export sympy kretschmann");
    });
    const eds = [...document.querySelectorAll(".block-editor")];
    p("ultimo-tem-comando=" + eds[eds.length - 1].innerText.includes("export sympy kretschmann"));

    // Agora o fato, direto do backend: o texto exato do bloco, e que
    // ele nao foi executado.
    const blocos = await window.ODEROM_invoke("list_blocks");
    const ultimo = blocos.blocks[blocos.blocks.length - 1];
    p("n-blocos=" + blocos.blocks.length);
    p("ultimo-source=" + JSON.stringify(ultimo.source));
    p("ultimo-nunca-rodou=" + (ultimo.output.kind === "NeverRun"));

    // Escape fecha sem inserir nada.
    document.getElementById("export-btn").click();
    await ateQue("o painel reabrir", () => !document.getElementById("export-panel").hidden);
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await ateQue("o painel fechar com Escape", () => document.getElementById("export-panel").hidden);
    p("escape-fecha=" + document.getElementById("export-panel").hidden);
    p("escape-nao-inseriu=" + (document.querySelectorAll(".block-editor").length === antes + 1));

    // O cabecalho comeca em "sem titulo" e passa a mostrar o nome
    // depois de salvar -- pelo botao de verdade, nao pela API. E' o
    // caminho inteiro: campo -> #save-btn -> backend.js -> wasm ->
    // list_blocks -> renderHeader.
    p("nome-inicial=" + JSON.stringify(document.getElementById("doc-name").textContent));
    document.getElementById("path-input").value = "meucaderno";
    document.getElementById("save-btn").click();
    await ateQue("o cabecalho mostrar o nome salvo",
      () => document.getElementById("doc-name").textContent === "meucaderno.od");
    p("nome-apos-salvar=" + JSON.stringify(document.getElementById("doc-name").textContent));
    p("FIM");
  } catch (e) {
    p("ERRO: " + (e && (e.message) || e));
  } finally {
    enviar();
  }
});
</script>
"#;

fn construir_ou_pular() -> Option<PathBuf> {
    let saida = raiz().join("target").join("oderom-web-teste");
    let r = Command::new("bash")
        .arg(raiz().join("oderom-wasm").join("construir.sh"))
        .arg(&saida)
        .output()
        .expect("nao consegui rodar o construir.sh");
    if r.status.success() {
        return Some(saida);
    }
    // O `construir.sh` já explica o que instalar e como; repassar a
    // mensagem dele é melhor que escrever uma segunda versão dela aqui,
    // que envelheceria em separado.
    eprintln!(
        "PULANDO o teste de navegador: nao consegui construir a versao web.\n{}",
        String::from_utf8_lossy(&r.stderr)
    );
    None
}

#[test]
fn a_versao_web_roda_de_verdade_num_navegador() {
    let Some(chrome) = navegador() else {
        eprintln!("PULANDO o teste de navegador: nenhum Chrome/Chromium no PATH.");
        return;
    };
    let Some(dir) = construir_ou_pular() else { return };

    std::fs::write(dir.join("driver-de-teste.html"), DRIVER).unwrap();
    let indice = std::fs::read_to_string(dir.join("index.html")).unwrap();
    std::fs::write(dir.join("driver-ui.html"), indice.replace("</body>", &format!("{DRIVER_UI}</body>"))).unwrap();
    let (porta, parar, caixa) = servir(dir.clone());
    let limite = std::time::Duration::from_secs(180);

    // Uma pagina de cada vez, esvaziando a caixa entre elas: as duas
    // reportam pela mesma rota.
    let relatorio = rodar_e_esperar(&chrome, &format!("http://127.0.0.1:{porta}/driver-de-teste.html"), &caixa, limite);
    caixa.lock().unwrap().clear();
    let ui = rodar_e_esperar(&chrome, &format!("http://127.0.0.1:{porta}/driver-ui.html"), &caixa, limite);

    parar.store(true, std::sync::atomic::Ordering::Relaxed);

    assert!(!relatorio.contains("ERRO:"), "o driver reportou erro:\n{relatorio}");
    assert!(relatorio.contains("FIM"), "o driver nao chegou ao fim:\n{relatorio}");
    assert!(relatorio.contains("backend=wasm"), "o backend escolhido nao foi o wasm:\n{relatorio}");
    assert!(relatorio.contains("blocos=4"), "o caderno inicial deveria ter 4 blocos:\n{relatorio}");
    assert!(relatorio.contains("query.state=done"), "a consulta deveria ter terminado:\n{relatorio}");

    // O Kretschmann de Reissner-Nordstrom, que e' o que o caderno
    // inicial calcula: K = 48M^2/r^6 - 96MQ^2/r^7 + 56Q^4/r^8. Conferir
    // o VALOR, e nao so que "algo apareceu", e' o que separa "o wasm
    // respondeu" de "o wasm respondeu certo" -- a mesma fixture de
    // aceitacao de `oderom-components/tests/reissner_nordstrom.rs`.
    for termo in ["48 M^{2} r^{2}", "-96 M Q^{2} r", "56 Q^{4}", "r^{8}"] {
        assert!(relatorio.contains(termo), "faltou {termo} no Kretschmann:\n{relatorio}");
    }

    assert!(relatorio.contains("galeria=5"), "a galeria deveria ter 5 entradas:\n{relatorio}");
    assert!(relatorio.contains("reabriu="), "o ida-e-volta de arquivo falhou:\n{relatorio}");
    // Sem nome nenhum o caderno e' "sem titulo" (nao ha arquivo); com
    // nome, ele passa a se chamar assim. As duas metades importam: e' a
    // segunda que faltava, e por isso o cabecalho da versao web ficava
    // preso em "sem titulo" mesmo depois de abrir um arquivo.
    // O worker, que e' a razao de a pagina nao congelar mais.
    assert!(relatorio.contains("execute-imediato=true"), "execute_block deveria voltar na hora:\n{relatorio}");
    assert!(relatorio.contains("responde-durante=true"), "a pagina deveria responder durante a conta:\n{relatorio}");
    assert!(relatorio.contains("marca-running=true"), "o bloco deveria aparecer executando:\n{relatorio}");
    assert!(relatorio.contains("segundo-recusado=true"), "duas execucoes ao mesmo tempo deveriam ser recusadas:\n{relatorio}");
    assert!(relatorio.contains("cancelado=true"), "cancelar deveria marcar o bloco:\n{relatorio}");
    assert!(relatorio.contains("texto-sobreviveu="), "o caderno deveria sobreviver ao cancelamento:\n{relatorio}");
    assert!(
        relatorio.contains(r#"texto-do-cancelado="gaussbonnet""#),
        "o bloco cancelado deveria manter o seu texto:\n{relatorio}"
    );

    assert!(relatorio.contains("sem-nome=null"), "um caderno sem arquivo deveria nao ter nome:\n{relatorio}");
    assert!(relatorio.contains(r#"apos-salvar="salvo.od""#), "salvar deveria nomear o caderno:\n{relatorio}");
    assert!(relatorio.contains(r#"apos-abrir="aberto.od""#), "abrir deveria nomear o caderno:\n{relatorio}");
    assert!(relatorio.contains("desconhecido-lanca=true"), "um comando so-do-Tauri deveria falhar alto:\n{relatorio}");

    assert!(relatorio.contains("alvos=mathematica,sympy"), "os alvos do export vieram errados:\n{relatorio}");
    assert!(relatorio.contains("consultas=12"), "o export deveria oferecer 12 consultas:\n{relatorio}");
    // `geodesic`/`accel` sao as unicas que exigem parametro afim, e o
    // seletor precisa saber disso para nao escrever uma linha invalida.
    assert!(relatorio.contains("param=accel,geodesic"), "quem precisa de parametro mudou:\n{relatorio}");

    // O seletor "Exportar" na pagina real, dirigido por cliques.
    assert!(!ui.contains("ERRO:"), "o driver de UI reportou erro:\n{ui}");
    assert!(ui.contains("FIM"), "o driver de UI nao chegou ao fim:\n{ui}");
    assert!(ui.contains("aberto=true"), "o botao Exportar nao abriu o painel:\n{ui}");
    assert!(ui.contains("alvos=mathematica,sympy"), "o painel listou outros formatos:\n{ui}");
    assert!(ui.contains("consultas=12"), "o painel listou outro numero de consultas:\n{ui}");
    assert!(ui.contains("geodesic=export mathematica geodesic tau"), "geodesic deveria vir com parametro:\n{ui}");
    assert!(ui.contains("todas-sympy=true"), "trocar o formato deveria reescrever todas as previas:\n{ui}");
    assert!(ui.contains("fechado=true"), "escolher uma consulta deveria fechar o painel:\n{ui}");
    assert!(ui.contains("n-blocos=5"), "deveria ter inserido exatamente um bloco novo:\n{ui}");
    assert!(ui.contains(r#"ultimo-source="export sympy kretschmann""#), "o bloco inserido tem outro texto:\n{ui}");
    // O seletor escreve o bloco e para ai -- rodar continua sendo
    // Shift+Enter, como em qualquer outro bloco. Um botao que tambem
    // executasse seria o unico lugar da pagina onde clicar dispara
    // conta, e "nada recalcula sozinho" vale aqui como no resto.
    assert!(ui.contains("ultimo-nunca-rodou=true"), "o seletor nao deveria executar nada:\n{ui}");
    assert!(ui.contains("ultimo-tem-comando=true"), "o bloco inserido nao apareceu na tela:\n{ui}");
    assert!(ui.contains("escape-fecha=true"), "Escape deveria fechar o painel:\n{ui}");
    assert!(ui.contains("escape-nao-inseriu=true"), "Escape nao deveria criar bloco nenhum:\n{ui}");
    assert!(ui.contains(r#"nome-inicial="sem título""#), "o caderno novo deveria abrir sem titulo:\n{ui}");
    assert!(ui.contains(r#"nome-apos-salvar="meucaderno.od""#), "o cabecalho nao mostrou o nome salvo:\n{ui}");
}

/// O servidor de teste responde, e a rota do relatório funciona.
///
/// Existe porque, quando o teste de navegador falha com "a pagina nao
/// reportou nada", há dois suspeitos -- o servidor e a página -- e sem
/// este teste não há como saber qual. Ele não precisa de Chrome.
#[test]
fn o_servidor_do_teste_serve_arquivos_e_recebe_relatorios() {
    let dir = std::env::temp_dir().join(format!("oderom-teste-servidor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<h1>ola</h1>").unwrap();
    let (porta, parar, caixa) = servir(dir.clone());

    let pedir = |caminho: &str| -> String {
        use std::io::Read;
        let mut c = TcpStream::connect(("127.0.0.1", porta)).expect("nao conectou");
        c.write_all(format!("GET {caminho} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
        let mut s = String::new();
        let _ = c.read_to_string(&mut s);
        s
    };

    assert!(pedir("/index.html").contains("ola"), "o servidor nao serviu o arquivo");
    assert!(pedir("/").contains("ola"), "a raiz deveria virar index.html");

    let resposta = pedir("/relatorio?d=oi%20mundo%20%7C%20FIM");
    assert!(resposta.contains("200 OK"), "a rota do relatorio nao respondeu: {resposta}");
    let recebido = caixa.lock().unwrap().clone();
    assert_eq!(recebido, vec!["oi mundo | FIM"], "o relatorio chegou errado");

    parar.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = std::fs::remove_dir_all(&dir);
}
