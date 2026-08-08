//! Os comandos, com toda a lógica e nenhuma dependência de wasm.
//!
//! Cada função recebe o objeto de argumentos como string JSON e devolve
//! a resposta como string JSON, ou uma mensagem de erro. Os invólucros
//! `#[wasm_bindgen]` do módulo pai só trocam o tipo do erro -- ver o
//! doc comment de lá para por que a divisão existe.

use oderom_notebook::{BeginExecution, BlockId, Notebook};
use oderom_ui::ExecuteOutcomeDto;
use std::cell::RefCell;

thread_local! {
    static NOTEBOOK: RefCell<Notebook> = RefCell::new(seed_example());
}

/// O mesmo notebook inicial do app de desktop -- Reissner-Nordström, a
/// fixture de aceitação do próprio projeto
/// (`oderom-components/tests/reissner_nordstrom.rs`), não um exemplo
/// inventado. Nada é executado na abertura: os blocos mostram seu
/// texto, e Shift+Enter continua sendo a única coisa que roda qualquer
/// coisa.
fn seed_example() -> Notebook {
    let mut notebook = Notebook::new();
    let a = notebook.create_block_after(None, "manifold M dim 4\nbundle TM on M dim 4".to_string());
    let b = notebook.create_block_after(Some(a), "chart schw on M coords (t, r, theta, phi)".to_string());
    notebook.create_block_after(
        Some(b),
        "metric g on schw bundle TM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}"
            .to_string(),
    );
    notebook.create_block_after(None, "kretschmann".to_string());
    notebook
}

/// Um caderno genuinamente em branco -- um bloco vazio (o mesmo
/// placeholder tracejado que o `notebook.js` já mostra), nunca a
/// demonstração do [`seed_example`]: "em branco" quer dizer em branco,
/// com algum lugar para começar a digitar.
fn blank_notebook() -> Notebook {
    let mut notebook = Notebook::new();
    notebook.create_block_after(None, String::new());
    notebook
}

// ---------------------------------------------------------------
// Plumbing: JSON de entrada -> args tipados, saída -> JSON.
// ---------------------------------------------------------------

fn args<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, String> {
    // O `notebook.js` passa `undefined` para comandos sem argumentos, e
    // o `backend.js` transforma isso em `"{}"` -- mas aceitar a string
    // vazia também custa uma linha e evita um modo de falha bobo.
    let json = if json.trim().is_empty() { "{}" } else { json };
    serde_json::from_str(json).map_err(|e| format!("argumentos invalidos: {e}"))
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("falha ao serializar resposta: {e}"))
}

/// O retorno de um comando que, no Tauri, não devolve nada. O `invoke`
/// do Tauri resolve a promise com `null` nesse caso, e o frontend
/// compartilhado espera exatamente isso.
fn nada() -> Result<String, String> {
    Ok("null".to_string())
}

#[derive(serde::Deserialize)]
struct SemArgs {}

#[derive(serde::Deserialize)]
struct IdArgs {
    id: u64,
}

#[derive(serde::Deserialize)]
struct CreateArgs {
    after: Option<u64>,
    source: String,
}

#[derive(serde::Deserialize)]
struct EditArgs {
    id: u64,
    source: String,
}

#[derive(serde::Deserialize)]
struct LoadGalleryArgs {
    after: Option<u64>,
    name: String,
}

#[derive(serde::Deserialize)]
struct TextoArgs {
    texto: String,
}

// ---------------------------------------------------------------
// Os comandos.
// ---------------------------------------------------------------

pub fn list_blocks(a: &str) -> Result<String, String> {
    let _: SemArgs = args(a)?;
    NOTEBOOK.with(|n| json(&oderom_ui::notebook_dto(&n.borrow())))
}

pub fn create_block(a: &str) -> Result<String, String> {
    let a: CreateArgs = args(a)?;
    NOTEBOOK.with(|n| json(&n.borrow_mut().create_block_after(a.after.map(BlockId), a.source).0))
}

pub fn edit_block(a: &str) -> Result<String, String> {
    let a: EditArgs = args(a)?;
    NOTEBOOK.with(|n| n.borrow_mut().edit_block(BlockId(a.id), a.source));
    nada()
}

pub fn delete_block(a: &str) -> Result<String, String> {
    let a: IdArgs = args(a)?;
    NOTEBOOK.with(|n| n.borrow_mut().delete_block(BlockId(a.id)));
    nada()
}

/// Executa o bloco e **só então** retorna.
///
/// Esta é a diferença de comportamento real entre os dois hospedeiros,
/// e ela é imposta pela plataforma, não escolhida por conveniência: no
/// desktop, `execute_block` entrega a conta a uma thread e volta na
/// hora, para a janela seguir respondendo e o "Cancelar" continuar
/// alcançável. Aqui não há thread -- a página tem uma só, e é a mesma
/// que desenha a tela. Enquanto a conta roda, a aba fica parada.
///
/// O contrato visto pelo frontend continua idêntico: ele chama
/// `execute_block`, recebe `Ok`/`Blocked`/`NotFound`, e em seguida
/// chama `list_blocks` para redesenhar. A única coisa que muda é que o
/// resultado já está pronto quando o `list_blocks` chega, em vez de
/// aparecer num poll seguinte. Nenhum caminho de código do frontend
/// precisa saber disso.
///
/// O passo que devolve a responsividade é mover este crate para dentro
/// de um Web Worker: aí a thread que trava é a do worker, e a página
/// segue viva. Está registrado em `dist/LEIA-ME.md` como o próximo
/// passo, e não foi feito junto porque worker é uma mudança de
/// *transporte* (toda chamada vira mensagem assíncrona), e misturá-la
/// com a estreia do backend faria com que qualquer falha tivesse duas
/// causas possíveis.
pub fn execute_block(a: &str) -> Result<String, String> {
    let a: IdArgs = args(a)?;
    let id = BlockId(a.id);
    let pending = NOTEBOOK.with(|n| match n.borrow_mut().begin_execute(id) {
        BeginExecution::Started(pending) => Ok(Some(pending)),
        BeginExecution::Done => Ok(None),
        BeginExecution::Blocked { by } => Err(ExecuteOutcomeDto::Blocked { by: by.0 }),
        BeginExecution::NotFound => Err(ExecuteOutcomeDto::NotFound),
    });
    // O `borrow_mut` acima termina aqui, antes de `pending.run()`, pelo
    // mesmo motivo que o lado Tauri solta o Mutex antes de calcular: o
    // `run` pode reentrar no notebook, e um `RefCell` emprestado duas
    // vezes entra em pânico em vez de esperar.
    match pending {
        Err(recusa) => json(&recusa),
        Ok(pending) => {
            if let Some(pending) = pending {
                let result = pending.run();
                NOTEBOOK.with(|n| n.borrow_mut().finish_query(id, result));
            }
            json(&ExecuteOutcomeDto::Ok)
        }
    }
}

/// Existe, e não faz nada -- de propósito, e este é o comentário que
/// explica por quê em vez de deixar o leitor concluir que foi
/// esquecimento.
///
/// No desktop, cancelar funciona porque a conta roda em outra thread e
/// o `oderom-expr::cancel` a desenrola por unwind num checkpoint. Em
/// `wasm32` nenhuma das duas peças existe: o alvo é `panic = "abort"`
/// por construção (então não há unwind a desenrolar -- ver o comentário
/// em `oderom-expr/src/cancel.rs`, onde a cancelação profunda é
/// compilada fora neste alvo), e a conta roda na única thread que a
/// página tem, que é justamente a que estaria processando este clique.
/// Quando `cancel_block` consegue rodar, não há nada em voo para
/// cancelar; enquanto há, ele não consegue rodar.
///
/// Retornar silenciosamente é o comportamento certo para o frontend
/// compartilhado -- `cancel_block` no Tauri também só *pede* o
/// cancelamento e volta na hora, sem prometer que ele aconteceu. Quem
/// vai poder de fato cancelar aqui é o Web Worker, encerrando o worker
/// de fora; até lá, a saída do aluno para uma conta que não termina é
/// recarregar a aba, e os limites do `oderom-cli` (`--max-nodes`,
/// `--timeout`) continuam valendo dentro do wasm exatamente como no
/// desktop.
pub fn cancel_block(a: &str) -> Result<String, String> {
    let _: IdArgs = args(a)?;
    nada()
}

pub fn clear_execution(a: &str) -> Result<String, String> {
    let _: SemArgs = args(a)?;
    NOTEBOOK.with(|n| n.borrow_mut().clear_execution());
    nada()
}

pub fn new_notebook(a: &str) -> Result<String, String> {
    let _: SemArgs = args(a)?;
    NOTEBOOK.with(|n| *n.borrow_mut() = blank_notebook());
    nada()
}

pub fn gallery_list(a: &str) -> Result<String, String> {
    let _: SemArgs = args(a)?;
    json(&oderom_ui::gallery_entries())
}

/// O que o seletor "Exportar" pode oferecer -- como `gallery_list`,
/// dado estático, derivado da própria gramática.
pub fn export_options(a: &str) -> Result<String, String> {
    let _: SemArgs = args(a)?;
    json(&oderom_ui::export_options())
}

pub fn load_gallery(a: &str) -> Result<String, String> {
    let a: LoadGalleryArgs = args(a)?;
    let ids = NOTEBOOK.with(|n| {
        n.borrow_mut()
            .load_gallery_entry(a.after.map(BlockId), &a.name)
            .map(|ids| ids.into_iter().map(|id| id.0).collect::<Vec<_>>())
            .map_err(|e| e.to_string())
    })?;
    json(&ids)
}

/// O texto `.oderom` do notebook atual, para o JavaScript oferecer como
/// download -- o análogo de `save_notebook` aqui, onde não existe
/// caminho de arquivo para gravar.
///
/// Usa o mesmo `oderom_notebook::render` que o `save` do desktop usa
/// antes de escrever no disco, então os dois produzem o mesmo arquivo,
/// e um caderno salvo no navegador abre no app e vice-versa. O nome é
/// diferente do comando do Tauri de propósito: isto *não* é
/// `save_notebook`, é a metade que o Rust consegue fazer; a outra
/// metade (perguntar o nome, disparar o download) é do navegador, e
/// está no `backend.js`.
pub fn notebook_text(a: &str) -> Result<String, String> {
    let _: SemArgs = args(a)?;
    let texto = NOTEBOOK.with(|n| oderom_notebook::render(n.borrow().blocks()).map_err(|e| e.to_string()))?;
    json(&texto)
}

/// Substitui o notebook atual pelo conteúdo de um arquivo `.oderom` que
/// o aluno escolheu no seletor do navegador -- o análogo de
/// `open_notebook`, pela mesma razão e com a mesma divisão de trabalho
/// que o [`notebook_text`] acima.
///
/// Nada é executado ao abrir, igual ao desktop
/// (`oderom_notebook::load`): os blocos mostram seu texto e pronto.
pub fn load_notebook_text(a: &str) -> Result<String, String> {
    let a: TextoArgs = args(a)?;
    let mut notebook = Notebook::new();
    for source in oderom_notebook::parse_sources(&a.texto) {
        notebook.create_block_after(None, source);
    }
    NOTEBOOK.with(|n| *n.borrow_mut() = notebook);
    nada()
}

/// Prova, verificável de fora, de que a página carregou e rodou seu
/// próprio JS -- o mesmo papel do comando homônimo no desktop, que
/// grava um arquivo em `temp_dir`. Aqui não há onde gravar e o valor
/// diagnóstico é menor (o console do navegador já mostra), então isto
/// apenas responde, confirmando que a ponte JS↔wasm está de pé.
pub fn frontend_ready(a: &str) -> Result<String, String> {
    let _: SemArgs = args(a)?;
    nada()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Os testes rodam no host, não no navegador: tudo aqui é lógica de
    // despacho e serialização, que não depende de wasm nenhum -- ver o
    // doc comment do `lib.rs` sobre por que a divisão existe.
    //
    // O estado é `thread_local`, e o test runner do Rust dá uma thread
    // por teste, então cada teste abaixo começa do seu próprio
    // `seed_example` e não pode interferir nos outros.

    fn v(s: String) -> serde_json::Value {
        serde_json::from_str(&s).expect("resposta deveria ser JSON valido")
    }

    #[test]
    fn cada_comando_devolve_json_valido_e_o_contrato_do_tauri() {
        // `list_blocks` traz o mesmo seed do desktop, com os 4 blocos.
        let blocos = v(list_blocks("{}").unwrap());
        assert_eq!(blocos["blocks"].as_array().unwrap().len(), 4);
        assert!(blocos["current_path"].is_null());
        // Comandos sem retorno respondem `null`, como o invoke do Tauri.
        assert_eq!(clear_execution("{}").unwrap(), "null");
        assert_eq!(frontend_ready("{}").unwrap(), "null");
        // A galeria e' dado estatico e nao pode vir vazia.
        assert!(!v(gallery_list("{}").unwrap()).as_array().unwrap().is_empty());
    }

    #[test]
    fn editar_criar_e_apagar_bloco_mexem_no_mesmo_notebook() {
        new_notebook("{}").unwrap();
        let id = v(create_block(r#"{"after":null,"source":"kretschmann"}"#).unwrap()).as_u64().unwrap();
        edit_block(&format!(r#"{{"id":{id},"source":"ricci"}}"#)).unwrap();
        let blocos = v(list_blocks("{}").unwrap());
        let bloco = blocos["blocks"].as_array().unwrap().iter().find(|b| b["id"] == id).unwrap();
        assert_eq!(bloco["source"], "ricci");
        delete_block(&format!(r#"{{"id":{id}}}"#)).unwrap();
        let blocos = v(list_blocks("{}").unwrap());
        assert!(blocos["blocks"].as_array().unwrap().iter().all(|b| b["id"] != id));
    }

    #[test]
    fn salvar_e_reabrir_preserva_o_texto_dos_blocos() {
        // O ida-e-volta que garante que um caderno baixado do navegador
        // abre de novo -- no navegador ou no app, e' o mesmo formato.
        new_notebook("{}").unwrap();
        create_block(r#"{"after":null,"source":"manifold M dim 4"}"#).unwrap();
        let texto: String = serde_json::from_str(&notebook_text("{}").unwrap()).unwrap();
        new_notebook("{}").unwrap();
        load_notebook_text(&serde_json::json!({ "texto": texto }).to_string()).unwrap();
        let blocos = v(list_blocks("{}").unwrap());
        let fontes: Vec<&str> =
            blocos["blocks"].as_array().unwrap().iter().map(|b| b["source"].as_str().unwrap()).collect();
        assert!(fontes.contains(&"manifold M dim 4"), "fontes: {fontes:?}");
    }

    #[test]
    fn executar_um_bloco_deixa_o_resultado_pronto_antes_de_retornar() {
        // A diferenca de comportamento documentada em `execute_block`:
        // aqui, ao contrario do desktop, `list_blocks` logo depois ja
        // ve o resultado -- nao um "running".
        new_notebook("{}").unwrap();
        let id = v(create_block(r#"{"after":null,"source":"manifold M dim 4"}"#).unwrap()).as_u64().unwrap();
        assert_eq!(v(execute_block(&format!(r#"{{"id":{id}}}"#)).unwrap())["kind"], "Ok");
        let blocos = v(list_blocks("{}").unwrap());
        let bloco = blocos["blocks"].as_array().unwrap().iter().find(|b| b["id"] == id).unwrap();
        assert_ne!(bloco["output"]["kind"], "NeverRun", "o bloco deveria ter rodado antes do retorno");
    }

    #[test]
    fn executar_um_bloco_inexistente_e_recusado_sem_entrar_em_panico() {
        assert_eq!(v(execute_block(r#"{"id":999999}"#).unwrap())["kind"], "NotFound");
    }

    #[test]
    fn a_galeria_carrega_de_verdade_e_um_nome_errado_vira_erro() {
        new_notebook("{}").unwrap();
        let entradas = v(gallery_list("{}").unwrap());
        let nome = entradas[0]["name"].as_str().unwrap().to_string();
        let ids = v(load_gallery(&serde_json::json!({ "after": null, "name": nome }).to_string()).unwrap());
        assert!(!ids.as_array().unwrap().is_empty());
        assert!(load_gallery(r#"{"after":null,"name":"nao-existe-essa-entrada"}"#).is_err());
    }

    #[test]
    fn argumentos_invalidos_viram_erro_e_nao_panico() {
        assert!(edit_block(r#"{"id":"nao e um numero"}"#).is_err());
        assert!(edit_block("{}").is_err());
        assert!(execute_block("nao e json").is_err());
    }
}
