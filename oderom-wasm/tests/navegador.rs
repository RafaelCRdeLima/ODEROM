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
fn servir(dir: PathBuf) -> (u16, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let listener = TcpListener::bind("127.0.0.1:0").expect("nao consegui abrir uma porta local");
    let porta = listener.local_addr().unwrap().port();
    let parar = Arc::new(AtomicBool::new(false));
    let sinal = parar.clone();

    std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        while !sinal.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((fluxo, _)) => {
                    let dir = dir.clone();
                    std::thread::spawn(move || atender(fluxo, &dir));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    (porta, parar)
}

fn atender(mut fluxo: TcpStream, dir: &Path) {
    let mut linha = String::new();
    if BufReader::new(fluxo.try_clone().unwrap()).read_line(&mut linha).is_err() {
        return;
    }
    let caminho = linha.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/");
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
const DRIVER: &str = r#"<!doctype html><meta charset="utf-8"><body><pre id="r">
</pre>
<script>
const out = document.getElementById("r");
const p = (m) => { out.textContent += m + "\n"; };
window.onerror = (m, s, l) => p("ERRO: " + m + " @" + s + ":" + l);
window.addEventListener("unhandledrejection", e =>
  p("ERRO: rejeicao nao tratada: " + (e.reason && (e.reason.stack || e.reason.message || e.reason))));
</script>
<script src="backend.js"></script>
<script>
(async () => {
  try {
    const inv = window.ODEROM_invoke;
    p("backend=" + window.ODEROM_backend);

    // O caderno inicial, e cada bloco dele executado de verdade.
    let n = await inv("list_blocks");
    p("blocos=" + n.blocks.length);
    for (const b of n.blocks) {
      const r = await inv("execute_block", { id: b.id });
      if (r.kind !== "Ok") return p("ERRO: execute retornou " + r.kind);
    }
    n = await inv("list_blocks");
    const q = n.blocks.find(b => b.output.kind === "Query");
    p("query.state=" + (q && q.output.state));
    p("latex=" + (q && q.output.latex));

    // Galeria, ida-e-volta de arquivo, e os comandos de estado.
    const g = await inv("gallery_list");
    p("galeria=" + g.length);
    p("load_gallery=" + (await inv("load_gallery", { after: null, name: g[0].name })).length);
    const texto = await inv("notebook_text");
    await inv("new_notebook");
    await inv("load_notebook_text", { texto });
    p("reabriu=" + (await inv("list_blocks")).blocks.length);
    await inv("clear_execution");
    await inv("cancel_block", { id: 0 });
    await inv("frontend_ready");

    // Um comando que so o outro backend tem precisa falhar alto aqui,
    // e nomeando o comando -- e' o modo de falha que o backend.js
    // existe para evitar.
    try { await inv("comando_inexistente"); p("ERRO: deveria ter lancado"); }
    catch (e) { p("desconhecido-lanca=" + /comando_inexistente/.test(e.message)); }

    p("FIM");
  } catch (e) { p("ERRO: " + (e && (e.stack || e.message) || e)); }
})();
</script></body>"#;

/// Carrega uma página no Chrome headless e devolve o DOM final.
///
/// `--virtual-time-budget` faz o Chrome adiantar o relógio dos timers
/// em vez de esperar por eles, então o teste não fica preso a um
/// `sleep` calibrado no olho -- ele termina quando a página termina.
fn carregar(chrome: &str, url: &str) -> String {
    let saida = Command::new(chrome)
        .args([
            "--headless",
            "--disable-gpu",
            "--no-sandbox",
            "--virtual-time-budget=120000",
            "--dump-dom",
            url,
        ])
        .output()
        .expect("nao consegui rodar o navegador");
    String::from_utf8_lossy(&saida.stdout).into_owned()
}

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
    let (porta, parar) = servir(dir.clone());

    let dom = carregar(&chrome, &format!("http://127.0.0.1:{porta}/driver-de-teste.html"));
    let relatorio = entre(&dom, "<pre id=\"r\">", "</pre>").unwrap_or_default();
    let relatorio = descodificar(&relatorio);

    // A pagina REAL, com o notebook.js que o aluno usa -- nao so a
    // ponte. O `class="block"` so aparece se o `refresh()` inicial
    // conseguiu falar com o wasm e desenhar.
    let real = carregar(&chrome, &format!("http://127.0.0.1:{porta}/index.html"));
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
    assert!(relatorio.contains("desconhecido-lanca=true"), "um comando so-do-Tauri deveria falhar alto:\n{relatorio}");

    assert!(real.contains("class=\"block\""), "o index.html real nao desenhou bloco nenhum");
    assert!(real.contains("block-gutter"), "o index.html real nao desenhou os gutters");
}

fn entre<'a>(texto: &'a str, abre: &str, fecha: &str) -> Option<&'a str> {
    let i = texto.find(abre)? + abre.len();
    let j = texto[i..].find(fecha)? + i;
    Some(&texto[i..j])
}

/// O `--dump-dom` devolve HTML, então o relatório vem escapado.
fn descodificar(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}
