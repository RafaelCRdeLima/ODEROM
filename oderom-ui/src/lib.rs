//! O contrato JSON entre o Rust e o frontend (`oderom-app/dist`).
//!
//! Este crate existe por um motivo só: **dois** backends respondem ao
//! mesmo frontend -- o app de desktop (`oderom-app/src-tauri`, via
//! `#[tauri::command]`) e a versão que roda no navegador do aluno
//! (`oderom-wasm`, compilada para `wasm32-unknown-unknown`). Os dois
//! precisam produzir exatamente o mesmo JSON, e enquanto estes tipos
//! moravam dentro do crate do Tauri isso era garantido por lembrança
//! humana. Aqui, é garantido pelo compilador: há uma definição só, e
//! quem mudar um campo quebra o build dos dois lados na mesma hora.
//! (Ver `oderom-app/dist/LEIA-ME.md`.)
//!
//! O que este crate deliberadamente **não** contém é o despacho dos
//! comandos. `execute_block` roda numa thread no desktop e num Web
//! Worker no navegador; `open_notebook` abre um diálogo do sistema lá e
//! um seletor do navegador aqui. Essas diferenças são reais, não
//! acidentais, e forçá-las numa abstração comum trocaria uma duplicação
//! honesta por uma indireção que mente. O que é obrigatoriamente igual
//! -- a *forma* dos dados -- é o que está aqui.
//!
//! Nenhum tipo do `oderom-notebook` atravessa a fronteira diretamente:
//! `Block`, `BlockOutput`, `EntryState` e `GalleryEntry` não são
//! `Serialize`, e de propósito -- a forma que o frontend consome é
//! decisão desta camada, não parte da API pública daqueles crates.

use oderom_notebook::{Block, BlockOutput, DeclarationStatus, EntryState, Notebook};

/// O que o frontend precisa para desenhar um bloco.
///
/// A Etapa 3a-2 não distinguia `confirmed`/`divergent` visualmente
/// (isso é a 3b), mas o DTO já carregava a string de status real de
/// qualquer forma, então a 3b foi uma mudança só de frontend, não um
/// comando novo.
#[derive(serde::Serialize)]
pub struct BlockDto {
    pub id: u64,
    pub source: String,
    pub output: OutputDto,
    /// Numeração `In [n]` no estilo Jupyter -- `None` antes de este
    /// bloco específico ter sido, ele mesmo, alvo direto de um execute
    /// (ver o doc comment do `oderom_notebook::Block::execution_count`
    /// para por que é por-bloco-explicitamente-rodado e não
    /// por-reconstrução-arrastado-junto).
    pub execution_count: Option<u64>,
    /// Etapa 3b (DESIGN-NOTEBOOK.md seção 9): `true` quando o resultado
    /// exibido deste bloco já não reflete confiavelmente o que está
    /// vivo -- puramente um fato a renderizar (marca âmbar, nunca
    /// vermelha: isto não é um erro), nunca algo que este crate ou o
    /// frontend decida ou sobre o que aja. Sempre `false` quando
    /// `execution_count` é `None`.
    pub obsolete: bool,
}

/// Tudo o que uma chamada de `list_blocks` entrega ao frontend para
/// redesenhar o notebook inteiro, cabeçalho (nome do arquivo atual)
/// incluído -- uma ida e volta só, em vez de um segundo comando que o
/// frontend teria de lembrar de chamar em sincronia com o primeiro.
#[derive(serde::Serialize)]
pub struct NotebookDto {
    pub blocks: Vec<BlockDto>,
    pub current_path: Option<String>,
}

/// Um pedaço clicável e copiável de um resultado -- espelha o
/// `oderom_components::RenderedComponent` campo a campo. `latex` aqui é
/// deliberadamente a mesma string limpa "nome = valor" que um clique no
/// componente deve pôr na área de transferência: o frontend nunca
/// remonta nem re-deriva o que copiar a partir de outra coisa na tela,
/// ele pega este campo literalmente (o handler de clique do
/// `notebook.js`). `orbit_note`, quando presente, é mostrado
/// subordinado (menor, ao lado ou abaixo do `latex`) mas nunca faz
/// parte do que é copiado.
#[derive(serde::Serialize, Clone)]
pub struct ComponentDto {
    pub latex: String,
    pub orbit_note: Option<String>,
}

pub fn component_dto(c: &oderom_components::RenderedComponent) -> ComponentDto {
    ComponentDto { latex: c.formula.clone(), orbit_note: c.orbit_note.clone() }
}

#[derive(serde::Serialize)]
#[serde(tag = "kind")]
pub enum OutputDto {
    NeverRun,
    Declaration { status: String, message: Option<String> },
    Query {
        state: String,
        latex: Option<String>,
        /// O mesmo resultado de `latex`, dividido por componente -- ver
        /// `ComponentDto`. Vazio quando `state` não é `"done"`/`"stale"`
        /// (não há nada a mostrar ainda), do mesmo modo que `latex` é
        /// `None` nesse caso.
        components: Vec<ComponentDto>,
        /// Linhas em português que pertencem ao resultado como um todo,
        /// não a um componente (contagem de truncamento, contagem de
        /// identicamente nulos) -- renderizadas como texto simples,
        /// nunca pelo KaTeX, nunca copiáveis por clique de componente.
        summary: Vec<String>,
        message: Option<String>,
    },
    /// Etapa 3b (cancelamento, DESIGN-NOTEBOOK.md): a tentativa de
    /// execução mais recente deste bloco está rodando ou terminou em
    /// cancelamento -- `state` é `"running"` ou `"cancelled"`.
    /// `previous`, se presente, é um resultado *anterior* já assentado
    /// (nunca ele mesmo running/cancelled) a renderizar ao lado --
    /// sempre obsoleto quando mostrado assim, incondicionalmente:
    /// qualquer `previous` presente aqui está, por construção,
    /// superado por uma tentativa mais nova, diga o que disser o
    /// `BlockDto::obsolete` (um sinal separado, baseado em
    /// posição/auto-edição).
    Attempt { state: String, previous: Option<PreviousResultDto> },
    Unrecognized { message: String },
}

#[derive(serde::Serialize)]
pub struct PreviousResultDto {
    pub state: String,
    pub latex: Option<String>,
    pub components: Vec<ComponentDto>,
    pub summary: Vec<String>,
    pub message: Option<String>,
}

/// Um `EntryState` como DTO -- compartilhado pelo `state` do próprio
/// `Query` e pelo `previous` do `Attempt`, para que os dois nunca
/// possam discordar em silêncio sobre o mesmo vocabulário
/// (`"done"`/`"stale"`/`"failed"`/...) nem sobre quais de
/// `components`/`summary` são preenchidos junto com `latex`.
pub struct EntryDto {
    pub state: &'static str,
    pub latex: Option<String>,
    pub components: Vec<ComponentDto>,
    pub summary: Vec<String>,
    pub message: Option<String>,
}

pub fn entry_state_dto(state: &EntryState) -> EntryDto {
    let empty = || EntryDto { state: "", latex: None, components: Vec::new(), summary: Vec::new(), message: None };
    match state {
        EntryState::Pending => EntryDto { state: "pending", ..empty() },
        EntryState::Running => EntryDto { state: "running", ..empty() },
        EntryState::Done { result, .. } => EntryDto {
            state: "done",
            latex: Some(result.latex.clone()),
            components: result.components.iter().map(component_dto).collect(),
            summary: result.summary.clone(),
            ..empty()
        },
        EntryState::Stale { result, .. } => EntryDto {
            state: "stale",
            latex: Some(result.latex.clone()),
            components: result.components.iter().map(component_dto).collect(),
            summary: result.summary.clone(),
            ..empty()
        },
        EntryState::Cancelled => EntryDto { state: "cancelled", ..empty() },
        EntryState::Failed { message, .. } => EntryDto { state: "failed", message: Some(message.clone()), ..empty() },
    }
}

pub fn block_to_dto(block: &Block, notebook: &Notebook) -> BlockDto {
    let output = match &block.output {
        BlockOutput::NeverRun => OutputDto::NeverRun,
        BlockOutput::Declaration(status) => {
            let (status, message) = match status {
                DeclarationStatus::Confirmed => ("confirmed", None),
                DeclarationStatus::Divergent => ("divergent", None),
                DeclarationStatus::Error(msg) => ("error", Some(msg.clone())),
            };
            OutputDto::Declaration { status: status.to_string(), message }
        }
        BlockOutput::Query(entry_id) => match notebook.session().entries().iter().find(|e| e.id == *entry_id) {
            Some(entry) => {
                let e = entry_state_dto(&entry.state);
                OutputDto::Query { state: e.state.to_string(), latex: e.latex, components: e.components, summary: e.summary, message: e.message }
            }
            // Não deveria acontecer (um bloco só chega a segurar um
            // EntryId que esta mesma Session criou), mas uma camada de
            // DTO reporta um "não sei" honesto em vez de entrar em
            // pânico num caminho de exibição.
            None => OutputDto::Query {
                state: "missing".to_string(),
                latex: None,
                components: Vec::new(),
                summary: Vec::new(),
                message: Some("no matching session entry".to_string()),
            },
        },
        BlockOutput::Attempt { attempt, previous } => {
            let state = match notebook.session().entries().iter().find(|e| e.id == *attempt).map(|e| &e.state) {
                Some(EntryState::Cancelled) => "cancelled",
                // Running é o caso esperado de longe; qualquer outro
                // estado aqui significaria que `finish_query` chegou
                // sem que o output do próprio bloco fosse atualizado
                // junto, o que nada no `oderom-notebook` faz --
                // "running" é o padrão honesto, não um palpite
                // disfarçado de um dos outros estados nomeados.
                _ => "running",
            };
            let previous = previous.and_then(|prev_id| {
                notebook.session().entries().iter().find(|e| e.id == prev_id).map(|e| {
                    let e = entry_state_dto(&e.state);
                    PreviousResultDto { state: e.state.to_string(), latex: e.latex, components: e.components, summary: e.summary, message: e.message }
                })
            });
            OutputDto::Attempt { state: state.to_string(), previous }
        }
        BlockOutput::Unrecognized(message) => OutputDto::Unrecognized { message: message.clone() },
    };
    BlockDto { id: block.id.0, source: block.source.clone(), output, execution_count: block.execution_count, obsolete: block.is_obsolete() }
}

/// A resposta inteira de `list_blocks`, montada aqui e não em cada
/// backend: é o comando que o frontend chama depois de *toda* mudança
/// de estado, então é o que teria mais a perder se os dois hospedeiros
/// divergissem.
pub fn notebook_dto(notebook: &Notebook) -> NotebookDto {
    NotebookDto {
        blocks: notebook.blocks().iter().map(|b| block_to_dto(b, notebook)).collect(),
        current_path: notebook.current_path().map(|p| p.display().to_string()),
    }
}

/// Etapa 3b, segunda parte (exclusão mútua, DESIGN-NOTEBOOK.md seção
/// 10.8): o que `execute_block` de fato fez, para o frontend distinguir
/// um início real de uma recusa em vez de os dois parecerem sucesso
/// silencioso. `Blocked` carrega o id do bloco que está realmente
/// rodando, para a barra de status poder nomeá-lo (nunca um modal -- a
/// exigência do próprio usuário) em vez de só dizer "não".
#[derive(serde::Serialize)]
#[serde(tag = "kind")]
pub enum ExecuteOutcomeDto {
    Ok,
    Blocked { by: u64 },
    NotFound,
}

/// Uma entrada do seletor "Galeria" -- o frontend nunca vê a
/// [`oderom_notebook::gallery::GalleryEntry`] em si (aquele tipo tem
/// campos `&'static str` pensados para chamadores Rust, não
/// `Serialize`), só o suficiente para preencher um dropdown e mostrar
/// as duas linhas de descrição (os campos
/// `title`/`description`/`invariant` do
/// `oderom-notebook/src/gallery.rs`).
#[derive(serde::Serialize)]
pub struct GalleryEntryDto {
    pub name: String,
    pub title: String,
    pub description: String,
    pub invariant: String,
}

/// Um formato para o qual o `export` sabe traduzir.
#[derive(serde::Serialize)]
pub struct ExportTargetDto {
    /// A palavra que vai no bloco: `"sympy"`, `"mathematica"`.
    pub keyword: String,
}

/// Uma consulta que pode ser exportada, e o que ela exige para ser uma
/// linha completa.
#[derive(serde::Serialize)]
pub struct ExportQueryDto {
    pub keyword: String,
    /// `geodesic`/`accel` precisam de um nome de parâmetro afim depois
    /// da palavra (`geodesic tau`); as demais não. Vem do parser
    /// (`CommandName::needs_affine_parameter`), nunca de uma lista
    /// mantida à mão do lado do frontend.
    pub needs_param: bool,
}

/// O que o seletor "Exportar" oferece: os formatos e as consultas que
/// `export` aceita, direto das listas do parser.
#[derive(serde::Serialize)]
pub struct ExportOptionsDto {
    pub targets: Vec<ExportTargetDto>,
    pub queries: Vec<ExportQueryDto>,
}

/// Tudo o que o seletor "Exportar" precisa, derivado do parser.
///
/// Existe porque a exportação para SymPy/Mathematica funcionava mas
/// era invisível: nada na interface dizia que `export sympy
/// kretschmann` era possível, e quem não soubesse a sintaxe de cor não
/// tinha como descobri-la. O seletor escreve a linha; esta função diz o
/// que ele pode escrever.
///
/// As duas listas vêm de `oderom_cli::parser` e nunca de uma cópia:
/// um alvo ou uma consulta acrescentados lá aparecem aqui na mesma
/// compilação. A alternativa -- enumerá-las no JavaScript -- é
/// exatamente o que já deixou o realce de sintaxe desta mesma página
/// desatualizado duas vezes.
pub fn export_options() -> ExportOptionsDto {
    use oderom_cli::parser::{export_target_keywords, CommandName};
    ExportOptionsDto {
        targets: export_target_keywords().map(|keyword| ExportTargetDto { keyword: keyword.to_string() }).collect(),
        queries: CommandName::keywords_with_command()
            .map(|(keyword, command)| ExportQueryDto {
                keyword: keyword.to_string(),
                needs_param: command.needs_affine_parameter(),
            })
            .collect(),
    }
}

/// Toda entrada conhecida da galeria, em ordem de catálogo -- dado
/// estático, então isto não precisa de estado nenhum.
pub fn gallery_entries() -> Vec<GalleryEntryDto> {
    oderom_notebook::gallery::ENTRIES
        .iter()
        .map(|e| GalleryEntryDto {
            name: e.name.to_string(),
            title: e.title.to_string(),
            description: e.description.to_string(),
            invariant: e.invariant.to_string(),
        })
        .collect()
}
