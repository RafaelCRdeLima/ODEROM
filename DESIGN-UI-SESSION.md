# ODEROM — DESIGN-UI-SESSION.md (sessão para a interface gráfica)

Respondendo aos seis pontos pedidos em `ODEROM-prompt-ui-sessao.md`, nesta
ordem. Proposta, não começo de implementação — mesma regra de sempre.

## 0. O que já existe e o que não existe (checado no código, não assumido)

Antes das respostas, três coisas que o prompt trata como já resolvidas e
que não estão, porque mudam o tamanho real do trabalho:

1. **Não existe rastreamento de linha/coluna em lugar nenhum do parser.**
   `Tok` (`oderom-cli/src/parser.rs`) não carrega posição; `CliError::Parse`
   é uma `String` nua. O contrato pedido (`Err { message, line, column }`)
   exige instrumentar o lexer com posição (linha/coluna correndo junto do
   char stream) e propagar isso por `TokStream`/`Tok` até todo `CliError`
   que hoje é só mensagem. Trabalho real, pré-requisito de `evaluate_definitions`
   — não é encanamento de UI.

2. **Não existe cache de computação hoje.** O que existe é a propriedade
   estrutural que permitiria construir um: `Expr` já deriva `Hash`
   (`oderom-expr/src/lib.rs`), e o comentário em `oderom-core/src/monomial.rs`
   registra a intenção de chave por forma canônica. Mas `Grid` e
   `ComponentTensor` (`oderom-components/src/grid.rs`, `tensor.rs`) só
   derivam `Clone, Debug` — nenhum `Hash`/`Eq`, e não dá para derivar
   direto porque ambos guardam um `FxHashMap` (ordem de iteração não é
   estável, `#[derive(Hash)]` não compila sobre isso sem mais). A seção 3
   propõe a peça que falta: uma função que itera em ordem canônica
   (ordenada por índice) e hasheia.

3. **`oderom-cli` hoje só tem alvo binário.** `Model`, `parser`, `error`,
   e a infraestrutura de progresso/orçamento/cancelamento em `commands.rs`
   vivem como módulos privados de um binário, não são alcançáveis por
   nenhum outro crate. Para o Tauri linkar isso como biblioteca (exigido
   no prompt), alguma reestruturação de crate é necessária — não é opção,
   é pré-condição. Proposta na seção 6.

Nenhuma das três é grande isoladamente, mas as três juntas são o "custo
de entrada" real deste projeto, maior do que a lista de "Pré-requisitos"
do prompt sozinha sugere. Registro aqui para não aparecer como surpresa
no meio da implementação.

---

## 1. Structs de sessão, entrada e obsolescência

```rust
/// Todo o estado vivo da aplicação. No máximo um `Document` avaliado com
/// sucesso por vez; a planilha é construída em cima dele.
pub struct Session {
    /// Texto fonte do .od tal como o usuário digitou -- mantido mesmo
    /// quando não parseia, para o editor sempre mostrar o que está lá,
    /// não o último estado válido.
    source: String,
    /// `None` só antes da primeira avaliação bem-sucedida da vida da
    /// sessão. Depois disso nunca volta a `None` -- uma avaliação que
    /// falha deixa o `Document` anterior intocado (ver seção 4).
    document: Option<Document>,
    entries: Vec<Entry>,
    next_entry_id: u64,
    cache: ComputeCache,
    /// No máximo uma execução em voo -- o próprio contrato pedido
    /// (`cancel_running()` sem parâmetro) já assume isso; ver seção 5.
    running: Option<RunningEntry>,
}

/// As definições que parsearam e construíram com sucesso, mais o que a
/// obsolescência por nome (seção 2) precisa para comparar "antes" contra
/// "depois".
struct Document {
    model: Model,                                  // oderom-cli, existente
    generation: Generation,
    /// Uma fingerprint por metric/connection/chart declarado, calculada
    /// uma vez aqui, no momento da avaliação -- não recalculada depois.
    fingerprints: HashMap<String, DefFingerprint>,
}

/// Newtype só para não trocar geração com id de entrada por engano num
/// call site -- os dois são `u64` por baixo.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Generation(u64);

/// Hash do conteúdo semântico já normalizado de uma declaração nomeada
/// (o `ComponentTensor` de uma metric, o `Grid` de uma connection) --
/// nunca do texto fonte bruto. Ver seção 3 para por quê isso importa
/// tanto para obsolescência quanto para o cache, com a mesma peça.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DefFingerprint(u64);

pub struct Entry {
    id: EntryId,
    /// O que o usuário digitou -- `"ricci"`, por exemplo. É o único
    /// campo persistido (ver Persistência no prompt original).
    input: String,
    state: EntryState,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId(u64);

enum EntryState {
    /// Nunca rodou -- entrada recém-digitada, ou planilha recém-reaberta
    /// de arquivo (a saída nunca é persistida, ver seção 4).
    Pending,
    Running,
    Done {
        result: EntryResult,
        used: BTreeSet<String>,          // nomes de declaração realmente tocados
        as_of: Generation,
    },
    /// Mesmo payload de `Done`, preservado -- nunca descartado. A
    /// diferença é só a intenção de exibição (seção 4): resultado
    /// visivelmente marcado como não-mais-confiável, não escondido.
    Stale {
        result: EntryResult,
        used: BTreeSet<String>,
        as_of: Generation,
    },
    Failed {
        message: String,
        line: Option<u32>,
        column: Option<u32>,
    },
}

struct EntryResult {
    latex: String,
    unicode: String,
    elapsed_ms: u64,
}
```

`EntryState` carrega o conjunto `used` desde o primeiro commit, inclusive
no branch `Failed` implicitamente vazio -- é a exigência explícita do
prompt ("o tipo já nasce carregando o conjunto de nomes") satisfeita por
construção, não por convenção a lembrar depois.

---

## 2. Como o conjunto de nomes referenciados é coletado

Reaproveitando a mesma forma que `Progress` já usa em `commands.rs`
(struct compartilhada entre a thread de trabalho e quem espera o
resultado): estendo esse tipo em vez de inventar um mecanismo paralelo.

```rust
/// Substitui `Progress` (commands.rs) por um superconjunto: mesma
/// função de relatar estágio, mais duas coisas que a sessão precisa e a
/// CLI hoje não -- ver seção 5 para o campo `cancelled`.
struct ExecutionContext {
    stage: Mutex<String>,
    cancelled: AtomicBool,
    used: Mutex<BTreeSet<String>>,
}

impl ExecutionContext {
    fn record_use(&self, name: &str) {
        self.used.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string());
    }
}
```

`resolve_gamma_source`/`build_from_metric`/`build_from_connection`
(`commands.rs`) são os únicos lugares que hoje decidem "qual
metric/connection/chart nomeada estou usando" -- **correção sobre a
primeira versão deste documento**: não é só a metric. `build_from_metric`
já faz `model.charts.get(chart_name)` -- esse `chart_name` precisa do
próprio `ctx.record_use(chart_name)` tanto quanto o nome da metric, ou
editar só a carta (sem tocar na metric) não invalida nada, e o resultado
obsoleto volta a entrar por baixo da interface exatamente como você
descreveu para o cache -- só que aqui na seção errada do desenho.
`used` de uma entrada `ricci` sobre a metric `g` na carta `schw` fica
`{g, schw}`, nunca só `{g}`. Nenhuma lógica nova de resolução de nome; só
um `record_use` a mais em cada ponto que já faz um `.get(nome)`.

Para o vocabulário de entrada da v1 (`Query::Command`, ver abaixo), isso
sempre resulta em exatamente zero ou um nome. O mecanismo já está pronto
para quando uma entrada puder referenciar mais de uma declaração (uma
comparação entre duas metrics, por exemplo) sem precisar redesenhar nada
-- é só `record_use` sendo chamado mais de uma vez.

### Correção: uma gramática só, não palavra-chave à parte

**Revisado após sua correção.** A primeira versão deste documento tinha
`run_entry` casando a string de entrada contra os cinco nomes de
subcomando à mão -- exatamente a doença que o projeto já rejeitou uma
vez (DESIGN-UI.md 6.1, "uma gramática, não duas", para ASCII/LaTeX). O
princípio vale igual aqui: a entrada da planilha tem que passar pelo
**mesmo lexer e a mesma família de parser** que o `.od`, não um
`match` de string à parte em `oderom-session`.

Concretamente: `oderom-cli/src/parser.rs` ganha uma nova produção de
topo, irmã de `Declaration` (o que `manifold`/`bundle`/`chart`/
`metric`/`connection` já são), em vez de um segundo parser:

```rust
// oderom-cli/src/parser.rs -- mesmo módulo, TokStream/Lexer reaproveitados
// sem mudança (span nos tokens, seção 0.1, beneficia os dois igual).

/// Um dos dois tipos de construto de topo que esta gramática reconhece.
/// `Declaration` é o que `parse_model` já lida (refatorado para
/// devolver isso em vez de mutar o `Model` inline); `Query` é a
/// produção nova, o que uma entrada de planilha parseia.
pub enum TopLevel {
    Declaration(Declaration),
    Query(Query),
}

/// Subconjunto reconhecido pela v1 -- crescer para expressões,
/// contrações, substituições é acrescentar variante aqui, nunca trocar
/// de parser. `target` é a mesma regra de desambiguação que
/// `--metric`/`resolve_choice` já tem hoje, só que dentro da gramática
/// em vez de um flag de linha de comando.
pub enum Query {
    Command { name: Spanned<CommandName>, target: Option<Spanned<String>> },
}

pub enum CommandName { Christoffel, Riemann, Ricci, Scalar, Kretschmann }

pub fn parse_query(tokens: &mut TokStream) -> Result<Query, ParseError> { /* ... */ }
```

`run_entry` chama `parse_query(&mut TokStream::new(&input))` -- mesmo
lexer, mesmo tipo de span, mesmo `ParseError` com linha/coluna que o
`.od` usa. `Spanned<T> { value: T, span: Span }` é um tipo só, definido
uma vez, usado nos nós da AST de `Declaration` **e** de `Query` -- é a
mesma peça pedida em "span nos tokens e posição nos nós da AST" (seção
0.1) fechando dois requisitos ao mesmo tempo, no mesmo espírito de
`DefFingerprint` fechar obsolescência e cache com uma peça só.

Isso corrige a arquitetura sem mudar o escopo já registrado: a v1 ainda
só reconhece `Query::Command` (os cinco subcomandos, com desambiguação
de alvo); o que muda é que esse reconhecimento é uma produção da
gramática, parseada pela função `parse_query` de verdade, não um `if
input.trim() == "ricci"`. Quando entrar uma segunda variante de `Query`
(uma expressão, uma substituição), é uma nova branch em `parse_query` e
uma nova variante do enum -- o pipeline de entrada inteiro (lexer, span,
erro com linha/coluna, `used` sendo coletado durante a resolução)
continua valendo sem tocar.

---

## 3. O que invalida o quê, e a granularidade da v1

**Implementado desde a v1: comparação por nome, não invalidação total.**
Justificativa de não adiar: a atomicidade da troca de `Model` (seção 4)
já exige ter o `Document` antigo e o novo em mãos ao mesmo tempo, no
mesmo escopo, no momento da troca -- comparar fingerprint por nome
custa uma iteração sobre um `HashMap` pequeno nesse mesmo ponto, não é
trabalho extra que justifique adiar para depois do "type já carrega o
conjunto".

Algoritmo, dentro de `Session::evaluate_definitions` logo após o novo
`Document` ser construído com sucesso (ver seção 4 para a construção em
si):

```rust
fn changed_or_removed_names(old: &Document, new: &Document) -> BTreeSet<String> {
    let mut changed = BTreeSet::new();
    for (name, old_fp) in &old.fingerprints {
        match new.fingerprints.get(name) {
            Some(new_fp) if new_fp == old_fp => {}       // sem mudança
            _ => { changed.insert(name.clone()); }        // mudou, ou sumiu
        }
    }
    changed
}
```

Nomes *novos* (existem só no `new`) não entram no conjunto -- ninguém
podia ter usado um nome que não existia. Depois, para cada `Entry` em
`Done` ou já `Stale`:

```rust
if !entry_used.is_disjoint(&changed_or_removed) {
    // transiciona para Stale, preservando result/used/as_of
}
```

`Pending`/`Running`/`Failed` não são tocados por essa passada -- não há
resultado ali para ficar obsoleto.

### `DefFingerprint`: a peça que falta, e por que serve para duas coisas

**Implementado e verificado nesta rodada, não só desenhado** (você pediu
"verifique isso agora" para o ponto 3 abaixo) -- `Grid::canonical_hash`/
`ComponentTensor::canonical_hash` já existem em
`oderom-components/src/grid.rs`/`tensor.rs`, com teste:

```rust
// oderom-components/src/grid.rs
pub fn canonical_hash(&self) -> u64 {
    let mut entries: Vec<(&SmallVec<[u8; 4]>, &Expr)> = self.values.iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.as_slice().cmp(b.as_slice()));
    let mut hasher = FxHasher::default();
    self.dim.hash(&mut hasher);
    self.rank.hash(&mut hasher);
    for (idx, expr) in entries {
        idx.as_slice().hash(&mut hasher);
        expr.hash(&mut hasher);
    }
    hasher.finish()
}
```

`ComponentTensor::canonical_hash` é a mesma ideia sobre `independent`,
sem incluir `head` (um `HeadId` é só um índice num `Registry`
específico, nunca estável entre duas chamadas de `evaluate_definitions`
-- incluí-lo quebraria a própria propriedade que este hash existe para
ter).

**Verificação 3, feita, não só prometida**:
`canonical_hash_is_independent_of_insertion_order` (`grid.rs`/`tensor.rs`)
constrói o mesmo conteúdo em ordem direta e invertida e confirma hash
igual -- e, para provar que o passo de ordenação não é redundante,
também confirma que a ordem de iteração *bruta* do `FxHashMap` por baixo
(sem o `sort_by` primeiro) genuinamente difere entre as duas construções.
Sua preocupação era real: sem o sort, `DefFingerprint` seria instável
"pelo mesmo motivo que impede derivar `Hash`" -- exatamente o que a
segunda metade desse teste demonstra ao vivo, não apenas assume.

`DefFingerprint(tensor.canonical_hash())` é calculado uma vez por nome,
no momento de `evaluate_definitions`, e é a mesma peça usada tanto na
comparação de obsolescência acima quanto na chave do cache abaixo.

### O cache de computação

**Três correções sobre a primeira versão deste documento, todas do seu
verificação 2:**

**1. Chave cobre dependência transitiva.** Não a fingerprint da metric
sozinha -- a fingerprint *composta* de todo `used` (seção 2, agora
incluindo a carta também). Editar só a carta, sem tocar na metric, muda
a fingerprint composta e o cache erra a propósito (cache miss correto),
em vez de devolver um Christoffel calculado para a carta antiga:

```rust
/// Combina a fingerprint de cada nome em `used` (ordem determinística:
/// `BTreeSet` já itera ordenado) -- nunca só a da metric. É essa
/// composição, não a fingerprint de uma única declaração, que serve de
/// chave abaixo.
fn composite_fingerprint(document: &Document, used: &BTreeSet<String>) -> DefFingerprint {
    let mut hasher = FxHasher::default();
    for name in used {
        name.hash(&mut hasher);
        document.fingerprints[name].hash(&mut hasher);
    }
    DefFingerprint(hasher.finish())
}
```

Cobertura verificada no código, não suposta por cautela: `curvature.rs`
inteiro só lê `chart.dim()`/`chart.coord(..)` e o que
`ComponentTensor::get` precisa (a simetria da metric, mas essa é sempre
o gerador simétrico fixo que `parse_metric_decl` grava, nunca escolhida
pelo usuário -- não varia, não precisa entrar na fingerprint). `manifold`/
`bundle` não alimentam esse cálculo em nenhum ponto -- se isso mudar no
futuro (uma função de `curvature.rs` passar a ler algo do `bundle`
diretamente), a composição cresce naquele mesmo commit, não antes.

**2. Cacheia o objeto semântico, não a string renderizada** -- já era
essa a intenção (`ComputeCache` sempre guardou `Grid`/`Expr`), agora
dito sem ambiguidade: `run_entry` **nunca cacheia `latex`/`unicode`
diretamente**. A separação é em duas etapas, sempre:

```rust
fn run_query(session: &mut Session, query: &Query, ctx: &mut ExecutionContext) -> Result<EntryResult, RunError> {
    let key = composite_fingerprint(session.document(), &ctx.used);
    let grid_or_expr = session.cache.get_or_compute(key, || {
        // só entra aqui num cache miss -- christoffel_checkpointed etc.,
        // seção 5, com `ctx` fornecendo o checkpoint por componente
    })?;
    // renderização é sempre a partir do valor cacheado, nunca refeita
    // do zero -- trocar unicode/latex ou max_lines nunca invalida isto
    let latex = render(&grid_or_expr, Target::Latex, session.max_lines);
    let unicode = render(&grid_or_expr, Target::Unicode, session.max_lines);
    Ok(EntryResult { latex, unicode, elapsed_ms: ctx.elapsed_ms() })
}
```

`max_lines` na v1 continua fixo (o default da CLI, 20) -- não é um
parâmetro por entrada ainda -- mas a separação em duas etapas já deixa
isso pronto para virar ajustável sem tocar no cache no dia em que
precisar.

**3. Limite de memória: LRU com teto configurável, não ilimitado.**
Sem dependência nova (mesma razão de sempre -- e-grafo, Schreier-Sims,
o CAS inteiro são feitos à mão neste projeto): despejo por varredura
linear pelo `last_used` mais antigo, não a lista duplamente encadeada
O(1) clássica -- numa sessão real (um punhado de metrics distintas x um
punhado de estágios), o teto fica na casa de dezenas a poucas centenas
de entradas, onde O(n) não custa nada de mensurável e a implementação
mais simples tem menos lugar para esconder um bug:

```rust
struct LruCache<K, V> {
    entries: HashMap<K, (V, u64)>, // valor, "relógio" de último uso
    clock: u64,
    capacity: usize,
}

impl<K: Eq + Hash + Clone, V> LruCache<K, V> {
    fn get_or_compute(&mut self, key: K, compute: impl FnOnce() -> Result<V, ComponentError>) -> Result<&V, ComponentError>
    where V: Clone {
        self.clock += 1;
        if !self.entries.contains_key(&key) {
            let value = compute()?;
            if self.entries.len() >= self.capacity {
                if let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone()) {
                    self.entries.remove(&oldest);
                }
            }
            self.entries.insert(key.clone(), (value, self.clock));
        } else if let Some(entry) = self.entries.get_mut(&key) {
            entry.1 = self.clock;
        }
        Ok(&self.entries[&key].0)
    }
}
```

`ComputeCache` vira um `LruCache<DefFingerprint, _>` por estágio, com
uma capacidade padrão pequena (a definir com números reais de uso, não
chutada agora -- "grids de Riemann de RN são grandes" é exatamente o
motivo para não fixar um número sem medir primeiro; ponto a revisitar
quando a sessão estiver rodando de verdade e houver memória real para
olhar, não antes).

---

## 4. Atomicidade da troca de `Model`

Já é essencialmente garantida pelo formato de `parse_model(src) ->
Result<Model, CliError>` (`oderom-cli/src/parser.rs`): a função constrói
um `Model` inteiro localmente e só o devolve em caso de sucesso, nunca
muta nada fora de si. `Session::evaluate_definitions` só precisa não
estragar essa propriedade:

```rust
pub fn evaluate_definitions(&mut self, source: String) -> Result<EvalSummary, EvalError> {
    let start = Instant::now();
    let model = parser::parse_model(&source).map_err(|e| to_eval_error(e))?; // seção 0.1: precisa de linha/coluna
    let fingerprints = compute_fingerprints(&model);  // seção 3
    let new_document = Document { model, generation: self.next_generation(), fingerprints };

    if let Some(old_document) = &self.document {
        let changed = changed_or_removed_names(old_document, &new_document);
        self.mark_entries_stale(&changed);
    }

    let names: Vec<String> = new_document.fingerprints.keys().cloned().collect();
    self.source = source;
    self.document = Some(new_document);   // única atribuição -- é o "commit"
    Ok(EvalSummary { names, elapsed_ms: start.elapsed().as_millis() as u64 })
}
```

Se `parse_model` falha, a função retorna no `?` antes de tocar
`self.document` -- o `Document` anterior (se havia um) continua sendo o
que `run_entry` enxerga, palavra por palavra o "o estado anterior
continua válido e utilizável" pedido. Não precisa de transação/rollback
porque nunca há um `Model` parcialmente construído visível de fora:
`parse_model` já tem essa propriedade, `Session` só evita jogá-la fora
construindo o novo valor localmente antes de sobrescrever.

Síncrono, sem thread: `parse_model` já chama `normalize()` por
componente durante o parsing (confirmado no código, `parser.rs:322` e
`:345`), mas cada componente de metric é uma expressão fechada simples
(`1 - 2M/r`, `r^2`, ...) -- nada perto do custo do somatório de
Kretschmann. `elapsed_ms` sai de cronometrar a chamada síncrona direto,
sem precisar da infraestrutura de progresso/cancelamento da seção 5.
Se um exemplo futuro tiver uma expressão de componente cara o bastante
para isso deixar de valer, é hora de revisitar -- não antecipar agora.

---

## 5. Cancelamento e progresso sobre a infraestrutura que já existe

`run_with_budget` (`commands.rs`) já faz a maior parte do que
`run_entry` precisa: spawna a computação numa thread, devolve via canal,
timeout com `recv_timeout`. Duas mudanças, nenhuma delas trocando o
mecanismo:

1. **Cancelamento é cooperativo, no mesmo padrão que o timeout já usa** --
   e por um motivo estrutural, não de preguiça: o próprio comentário de
   `run_with_budget` já registra que "Rust has no safe way to force-stop"
   uma thread. `cancel_running()` seta
   `ExecutionContext::cancelled` (um `AtomicBool`); os pontos que já
   existem para checar orçamento de nós (`check_grid_budget`, depois de
   cada estágio que produz um `Grid` inteiro) e o laço termo-a-termo do
   Kretschmann (já checa depois de cada `normalize()`, TODA iteração, não
   só a cada `PROGRESS_STRIDE`) passam a checar `cancelled` no mesmo
   lugar, devolvendo um erro `Cancelled` em vez de continuar.

2. **Checkpoint por componente -- implementado nesta rodada, não só
   desenhado.** `oderom-components/src/curvature.rs` ganhou uma variante
   `_checkpointed` de cada estágio (`christoffel_checkpointed`,
   `riemann_mixed_checkpointed`, `lower_first_index_checkpointed`,
   `raise_index_checkpointed`, `ricci_tensor_checkpointed`,
   `kretschmann_checkpointed`), todas aceitando
   `checkpoint: &mut dyn FnMut() -> bool` chamado uma vez por componente
   independente, devolvendo `ComponentError::Cancelled` assim que ele
   reportar `true`. As funções antigas (`christoffel`, `riemann_mixed`,
   ...) viraram wrappers de uma linha passando `&mut || false` -- nunca
   cancelam, então continuam com a assinatura e o comportamento exatos de
   antes (as que eram infalíveis, como `riemann_mixed`, seguem
   infalíveis: o `.expect("never cancelled")` no wrapper é
   comprovadamente seguro, já que o checkpoint que ele passa nunca
   devolve `true`). Nenhum call site existente mudou.

   Verificado, não só assumido que "está chamando o checkpoint":
   `christoffel_checkpointed_stops_the_loop_immediately`/
   `riemann_mixed_checkpointed_stops_the_loop_immediately`
   (`curvature.rs`) plantam um checkpoint que cancela na 5ª chamada e
   confirmam que ele foi chamado exatamente 5 vezes -- prova que a
   função devolve assim que mandada parar, não que ela termina o laço
   inteiro e só then descarta o resultado.

   `ExecutionContext::cancelled` (`oderom-session`, ainda não
   implementado) é o `AtomicBool` que um `checkpoint` de verdade lê; o
   que existe agora é a metade que importava verificar primeiro -- que
   as funções de estágio genuinamente conseguem ser interrompidas no
   meio, com número medido, antes de construir a camada de sessão em
   cima.

### Medido, não assumido (`oderom-components/tests/diagnostic_cancel_latency.rs`,
`--release`, Reissner-Nordström -- o pior caso real que este projeto tem)

| | total do estágio | pior componente único |
|---|---|---|
| `christoffel` (64 componentes) | ~50-65ms | ~13-23ms |
| `riemann_mixed` (256 componentes) | ~450-500ms | ~31-41ms |

(`lower_first_index`/`raise_index` não medidos à parte: mesmo formato de
laço rank-4/dim-4 que `riemann_mixed`, porém sem chamada a `diff()` por
termo -- estritamente mais baratos por componente, então os números de
`riemann_mixed` já são o pior caso representativo entre os quatro.)

**Hoje** (checkpoint só entre estágios): pior caso de latência de
cancelamento em RN é ~500ms -- dentro dos 1-2s que você pediu, mas por
pouco, e só porque RN é o caso mais pesado que já temos rodando com
sucesso. **Com checkpoint por componente**: cai para ~40ms, o pior
componente único -- a latência passa a escalar com o componente mais
lento, não com o estágio inteiro, que é o invariante certo para um botão
de cancelar (o "conserto provável e barato" que você sugeriu -- medido
antes e depois, confirmado barato: são quatro laços que já existem,
ganhando uma checagem de `AtomicBool` cada iteração).

**O que isso não conserta, dito sem rodeio**: se um único componente
(uma única chamada de `normalize()` dentro do laço) não terminar ou
demorar minutos -- exatamente o teto de 3+ parâmetros livres já
registrado em DESIGN-RATIONAL-FORM.md seção 6 -- nenhum checkpoint entre
componentes ajuda, porque o checkpoint só é visto *depois* que a chamada
que está presa devolve. Nesse caso `cancel_running()` fica esperando
até essa chamada terminar (ou até o guarda-corpo de timeout/grau de
denominador que já existe abortar por conta própria), do mesmo jeito que
o timeout da CLI já se comporta hoje. Registrado como limite real, à
mesma altura do teto de 3+ parâmetros -- não escondido atrás do nome
"cancelar".

Progresso continua sendo a mesma string de estágio que `Progress::set`
já produz, só que em vez de `eprintln!` vira um evento Tauri
(`app.emit("progress", stage)`) -- unicidade de execução (seção 1,
`Session.running: Option<RunningEntry>`) significa que o evento nunca
precisa dizer *qual* entrada está progredindo, só o estágio, porque só
uma pode estar rodando por vez (é o que a própria assinatura de
`cancel_running()` sem parâmetro já assume, e é a leitura que estou
adotando: uma segunda chamada a `run_entry` enquanto uma está `Running`
devolve erro, não enfileira).

```rust
struct RunningEntry {
    entry_id: EntryId,
    ctx: Arc<ExecutionContext>,
}

pub fn cancel_running(&mut self) {
    if let Some(running) = &self.running {
        running.ctx.cancelled.store(true, Ordering::Relaxed);
    }
}
```

---

## 6. Onde este código vai morar

`oderom-cli` hoje só declara alvo binário -- nada em `model.rs`/
`parser.rs`/`commands.rs` é alcançável por outro crate. Proposta mínima,
sem mover arquivo nenhum de lugar:

- `oderom-cli` ganha um `src/lib.rs` expondo `model`, `parser`, `error`, e
  as partes reaproveitáveis de `commands.rs` (`resolve_gamma_source`,
  `ExecutionContext`/`Progress`, `check_grid_budget`, as cinco funções de
  estágio) como `pub`. `main.rs` passa a chamar a própria lib -- o
  binário da CLI continua fazendo exatamente o que faz hoje, mesmo
  comportamento, mesmos testes de `end_to_end.rs` intocados.
- **`oderom-session`**, crate novo, sem UI nenhuma dentro: `Session`,
  `Entry`, `EntryState`, `ComputeCache`, `evaluate_definitions`/
  `run_entry`/`cancel_running`/`session_snapshot`. Depende de
  `oderom-cli` (a lib) e `oderom-components`/`oderom-expr` diretamente. É
  este crate que ganha os testes pedidos no item 4 do "Como trabalhar"
  -- definir, consultar, redefinir, checar obsolescência, recalcular,
  checar que voltou -- tudo sem abrir janela, exatamente porque
  `Session` não sabe que Tauri existe.
- O crate do app Tauri (fora do escopo deste documento) depende só de
  `oderom-session`, traduz os quatro comandos da seção "Superfície do
  backend" em `#[tauri::command]` de uma linha cada, e emite os eventos
  de progresso. Nenhuma lógica de geometria ali -- é exatamente a regra
  dura do prompt, e a separação de crate torna impossível violá-la por
  acidente (o crate do app nem depende de `oderom-expr` diretamente, só
  vê `String`s prontas vindas de `EntryResult`).

---

## Resumo do que fica para o próximo passo, se aprovado

Três itens dos verificados nesta rodada já saíram do papel, adiantando a
ordem original (marcados abaixo). O resto continua proposta.

1. Span nos tokens (`Tok`) e posição nos nós da AST (`Spanned<T>`) no
   lexer/parser de `oderom-cli` -- pré-requisito isolado, testável
   sozinho, sem tocar em `Session`. `Spanned<T>` e o `ParseError` com
   linha/coluna são a base que a seção 2 também usa para `Query`.
2. `parse_model` refatorado para devolver `Declaration` (seção 2) em vez
   de mutar o `Model` inline; `TopLevel`/`Query`/`parse_query` somam-se
   como produção irmã, mesmo módulo. Ainda sem `run_entry` chamando
   nada -- só a gramática existindo e testada isoladamente.
3. ~~`Grid::canonical_hash`/`ComponentTensor::canonical_hash`.~~ **Feito
   e verificado** (seção 3): implementados, com teste provando
   independência de ordem de inserção e, junto, provando que a ordem
   bruta do `FxHashMap` por baixo genuinamente NÃO tem essa propriedade
   (a razão de o passo de sort existir, demonstrada, não só citada).
4. `oderom-cli` ganha `src/lib.rs`; `main.rs` migra para consumi-la;
   suíte e `end_to_end.rs` continuam verdes sem mudança de comportamento.
5. Crate `oderom-session`: `Document`, `DefFingerprint`,
   `changed_or_removed_names`, `evaluate_definitions` -- testado sozinho
   (definir, redefinir, checar quais nomes mudaram).
6. `Entry`/`EntryState`/`ExecutionContext`/`run_entry`, chamando
   `parse_query` do passo 2 -- testado (rodar, ficar obsoleto,
   recalcular, ficar atual de novo).
7. `ComputeCache` (`LruCache<DefFingerprint, _>` por estágio, chave
   composta sobre `used` inteiro) -- testado (duas entradas sobre a
   mesma metric não recomputam Christoffel; mudar a metric OU a carta
   invalida; teto de capacidade despeja o mais antigo).
8. ~~Checkpoint por componente em `christoffel`/`riemann_mixed`/
   `ricci_tensor`/`lower_first_index`/`raise_index`.~~ **Feito e
   verificado** (seção 5): variantes `_checkpointed`, wrappers antigos
   intocados em assinatura e comportamento, teste provando que a
   interrupção é imediata (conta chamadas do checkpoint, não só o
   resultado final). Falta só `cancel_running`/`ExecutionContext` de
   verdade os alimentando -- isso continua no passo 6.
9. Só então o crate Tauri e o front-end.

Nenhuma decisão de escopo em aberto no momento -- a única que havia
(vocabulário de entrada da v1) foi corrigida de "cinco palavras-chave"
para "cinco produções reconhecidas por uma gramática que já nasce
extensível", que é exatamente o pedido.

**Fora deste documento, mas pré-requisito que você marcou como bloqueante
antes de qualquer UI**: o oráculo diferencial (`v1_and_v2_agree`/
`v2_is_idempotent`, `oderom-expr/src/normalize.rs`) foi reescrito de
`f64` para racional exato (parametrização racional do círculo unitário
para os argumentos de seno/cosseno) -- o caso `(1+sin(M))^-3` perto de
`M=-1.6` que quebrava por cancelamento catastrófico agora é impossível
por construção (aritmética exata, sem ponto flutuante em lugar nenhum).
Achado e corrigido no processo: a primeira versão dessa reescrita
chaveava a memoização de seno/cosseno por `normalize(arg)`, e isso
descobriu (não assumiu) que `normalize()` não canonicaliza onde um fator
de escala constante fica em relação a `Pow(-1)` -- `(1/2)*x^-1` e
`(2*x)^-1` ficam formas diferentes, mesmo valor. Não é bug (as duas
formas são reduções válidas), registrado como teste de regressão
(`normalize_does_not_canonicalize_scale_factor_placement_around_pow_neg1`);
o oráculo foi corrigido para chavear pelo *valor avaliado* do argumento
em vez da forma simbólica, o que também sidesteps a questão por completo.
Verificado a 50.000 casos (`PROPTEST_CASES=50000`), limpo.
