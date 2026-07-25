# ODEROM — DESIGN-M6-PREP.md (Preparação de arquitetura, não implementação)

**Status: investigação, nenhum comportamento alterado.** Este documento não propõe nenhuma `struct` nova nem começa Marco 6 (ou qualquer marco). É a resposta a um pedido explícito: antes de implementar qualquer capacidade simbólica nova — (1) resolver componente para um parâmetro, (2) substituição/simplificação com hipóteses, (3) derivadas e expansão explícitas, (4) álgebra diferencial com `f(r)`/`f'`/`f''` indeterminadas, (5) EDOs simbólicas, (6) índices abstratos como subsistema separado — quais decisões de arquitetura de hoje, se erradas, tornam esses passos caros ou impossíveis depois. Cada seção abaixo cita o tipo/função real do código, não uma resposta genérica.

**Correção de premissa, adiantada:** nenhuma das cinco premissas do pedido estava errada. Duas ficaram mais afiadas do que a formulação original sugeria, e ambas mudam a recomendação final:

- (Pergunta 1) A parte difícil de acrescentar `f`/`f'`/`f''` não é só o match exaustivo — é real, mas mecânico. A parte que a premissa não antecipava é que, na camada de anel polinomial (`poly.rs`), `f`/`f'`/`f''` são estruturalmente **mais simples** que `sin`/`cos` já são hoje: não precisam de nenhuma identidade algébrica cruzada como `sin²+cos²=1` (`TrigRewriteSuppressor`, ver seção 1). Isso não muda a resposta ("motor fechado, não aberto"), mas muda o tamanho do trabalho futuro.
- (Pergunta 4) A premissa era "cancelamento é caro de retrofitar, então vamos fixar a regra agora". Verificação no código real mostrou que a regra **ainda não é seguida de forma consistente hoje**: existem dois pontos de entrada de computação, e só um dos dois arma o cancelamento profundo (seção 4). Isso não é uma correção da premissa — é uma confirmação mais forte do que ela supunha, com um exemplo concreto já em produção (`oderom-components/src/holonomy.rs`, zero cancelamento, Marco 5).

Nada abaixo foi corrigido no código. Onde encontrei algo que "dava vontade de consertar", ficou anotado como achado, não como mudança.

---

## 1. Representação de expressão

**A pergunta:** `normalize_via_rational_form` admite um átomo novo sem reescrever a normalização, ou faz correspondência exaustiva sobre um enum fechado?

**Resposta: enum fechado, correspondência exaustiva — a premissa está certa.** `Expr` (`oderom-expr/src/lib.rs:47`) é:

```rust
pub enum Expr {
    Rational(BigScalar), Var(String), Add(Vec<Expr>), Mul(Vec<Expr>),
    Pow(Box<Expr>, i32), Sin(Box<Expr>), Cos(Box<Expr>),
}
```

sete variantes, nenhum campo de extensão (nenhum `Other(...)`, nenhum `Box<dyn Trait>`). Todo `match` exaustivo sobre `Expr` no workspace hoje é um ponto que um oitavo átomo (`f`/`f'`/`f''`) obrigaria a tocar. Inventário completo, feito por grep, não por amostragem:

| Arquivo | Função | O que faz com cada variante |
|---|---|---|
| `oderom-expr/src/lib.rs` | `variant_rank` (`Ord`) | ordena variantes para canonicalizar `Add`/`Mul` |
| `oderom-expr/src/lib.rs` | `Expr::node_count` | soma de nós |
| `oderom-expr/src/canonical.rs` | `expr_to_rational` | **o motor real** (`normalize()` público chama isto, não o legado) — converte para `RationalFunction` |
| `oderom-expr/src/normalize.rs` | `legacy_v1::step` | motor antigo, mantido só como oráculo diferencial (`ODEROM_ENGINE=legacy`) |
| `oderom-expr/src/diff.rs` | `diff` | regra da cadeia |
| `oderom-expr/src/substitute.rs` | `substitute` | substituição de variável |
| `oderom-expr/src/rationalize.rs` | `to_fraction`, `degree`, `has_negative_exponent` | usado por `denominator_degree` (guarda de orçamento do CLI) |
| `oderom-expr/src/render.rs` | `unicode`, `latex`, `json` (três) | renderização |
| `oderom-jit/src/compile.rs` | `compile` | Marco 5, `Expr` → IR SSA |
| `oderom-expr/src/poly.rs` | `AtomTable::to_expr` | reconstrói `Expr` a partir de um `AtomId` interno |

Nove arquivos, ~11 pontos de match. Cada um é um erro de compilação (não um bug silencioso) no dia em que uma variante nova for acrescentada — Rust obriga a tratar todos antes de compilar. Isso é o oposto de "impossível depois": é "mecânico e guiado pelo compilador", o que é uma categoria de custo bem mais barata do que a pergunta parecia temer, mas ainda não é grátis, e ainda é hoje, não nunca.

**A parte que muda o tamanho do trabalho:** o motor real (`canonical.rs::expr_to_rational`) já trata `Sin`/`Cos` exatamente como um átomo genérico interno via `AtomTable`/`AtomKey` (`poly.rs:79`):

```rust
enum AtomKey { Var(String), Sin(Expr), Cos(Expr) }
```

`AtomKey::Sin(arg)`/`Cos(arg)` já são "funções indeterminadas de um argumento canonicalizado" do ponto de vista do anel — a única coisa que os torna `sin`/`cos` especificamente, e não uma função qualquer, é uma reescrita algébrica cruzada (`sin²+cos²=1`, D-RF.7) implementada à parte, em `rational_function.rs`, guardada por `TrigRewriteSuppressor` (`poly.rs:56`) durante `poly_gcd`. Uma função indeterminada `f` (sem identidade conhecida ligando `f`, `f'`, `f''` no nível do anel — elas só são "aparentadas" pela regra de derivação, não por álgebra) **não precisa dessa reescrita cruzada nenhuma**. Ou seja: a forma `AtomKey::Sin`/`Cos` já é o molde de como `f`/`f'`/`f''` entrariam nessa camada — e entrariam mais simples do que `sin`/`cos` entraram, não mais complicadas.

A parte genuinamente nova fica confinada a `diff.rs`: hoje, `Expr::Sin(inner) => Expr::Mul(vec![Expr::Cos(inner.clone()), diff(inner, var)])` — a regra da cadeia "a derivada de `sin` é `cos`" está codificada à mão porque `sin`/`cos` são funções conhecidas. Para `f` de ordem `k`, a mesma forma vale trocando "a derivada de `sin` é `cos`" por "a derivada de `f` de ordem `k` é `f` de ordem `k+1`" — genérico sobre a ordem, não uma função irmã fixa. Essa é a peça que realmente é trabalho novo, não mecânico; as outras ~10 são.

**Recomendação: não construir nada agora** (instrução explícita). O valor deste levantamento é que, quando o passo 4 começar, a lista de pontos a tocar já está pronta — nenhum será redescoberto por acidente no meio da implementação.

---

## 2. Contexto de hipóteses

**A pergunta:** `normalize` é uma função pura sem contexto? Quantas assinaturas mudam se um contexto de hipóteses precisar ser carregado, e vale a pena introduzir hoje um parâmetro vazio só para não mexer nelas depois?

**Confirmado: pura, sem contexto.** `pub fn normalize(e: &Expr) -> Expr` (`oderom-expr/src/normalize.rs:92`) e `pub(crate) fn normalize_via_rational_form(e: &Expr) -> Expr` (`canonical.rs:29`) — nenhum parâmetro além da própria expressão. Nada em `Expr`, `BigScalar`, `Poly` ou `RationalFunction` tem qualquer noção de sinal/domínio hoje — não existe `sqrt`, `abs`, nem operador de comparação em `Expr`, então não há sequer um ponto onde uma hipótese *poderia* ser consultada, mesmo que existisse.

**Sítios de chamada reais** (não teste, grep isolado de `normalize(&...)` em código de produção):

- `oderom-expr/src/rationalize.rs` — uso interno, dentro do próprio crate.
- `oderom-components/src/curvature.rs` — ~13 chamadas, dentro de `metric_inverse_diagonal`, `christoffel_checkpointed`, `riemann_mixed_checkpointed`, `lower_first_index_checkpointed`, `raise_index_checkpointed`, `ricci_tensor_checkpointed`, `ricci_scalar`, `kretschmann_checkpointed`.
- `oderom-components/src/atlas.rs` — 2 chamadas (checagem de consistência de pullback entre cartas).
- `oderom-cli/src/parser.rs` — 2 chamadas, no momento de declarar `metric`/`connection`.
- `oderom-components/src/render.rs` — 2 chamadas, no momento de exibir um componente.

Ponto que importa para o custo: **essas funções já não são "puras com nada mais"** — `christoffel_checkpointed` já recebe `registry: &Registry, chart: &Chart, g: &ComponentTensor, ginv: &Grid, checkpoint: Checkpoint`. Um parâmetro de hipóteses entraria na mesma convenção de chamada que já existe, não contra ela. O número de assinaturas a tocar é del ordem de ~15, em 4 arquivos de produção (`curvature.rs`, `atlas.rs`, `parser.rs`, `render.rs`) mais as duas funções do próprio `normalize`.

**Sobre introduzir um parâmetro vazio agora:** o custo de fazer isso hoje é *igual* ao custo de fazer isso quando o passo 2 realmente chegar — mesmos ~15 sítios, mesma mudança mecânica, guiada pelo compilador (um parâmetro novo obrigatório quebra a build em cada chamador até ser passado). Não há economia em adiantar. Só há um custo extra concreto de adiantar: um parâmetro de contexto que hoje seria sempre vazio e sempre ignorado é pior que a ausência dele — o tipo `normalize(e: &Expr, hyps: &Context)` afirma, pela assinatura, que a função *respeita* hipóteses, o que seria falso até o passo 2 realmente implementar a lógica que as consulta. Isso é exatamente o tipo de abstração prematura que a instrução do pedido já pede para evitar.

**Recomendação: não introduzir agora.** Sem assimetria de custo entre agora e depois, e com um custo real (assinatura mentirosa) de adiantar, a decisão certa é esperar o passo 2 chegar.

---

## 3. Chave do cache

**A pergunta:** o `ComputeCache` indexa por quê hoje? A chave prevê espaço para hipóteses, ou seria fonte de bug silencioso?

**Confirmado, e é a consequência direta da seção 2: não há espaço reservado, e seria bug silencioso se hipóteses existissem hoje sem tocar a chave.** `ComputeCache` (`oderom-session/src/cache.rs:101`) é `LruCache<DefFingerprint, V>` por estágio (`riemann_mixed`, `riemann_cov`, `ricci_tensor`, `ricci_scalar`, `kretschmann`). `DefFingerprint` (`oderom-session/src/fingerprint.rs:20`) é um `u64` puro — hash do **conteúdo semântico já normalizado** de cada declaração usada (`chart`/`metric`/`connection`), nunca do texto fonte, nunca de um id interno:

```rust
pub fn composite_fingerprint(fingerprints: &HashMap<String, DefFingerprint>, used: &BTreeSet<String>) -> DefFingerprint {
    let mut hasher = FxHasher::default();
    for name in used {
        name.hash(&mut hasher);
        if let Some(fp) = fingerprints.get(name) { fp.hash(&mut hasher); }
    }
    DefFingerprint(hasher.finish())
}
```

Chamada em exatamente **um lugar** (`oderom-session/src/run.rs:85`), logo depois de `ctx.used()`. Não é um hash com campo reservado nem estruturado (nem um `struct { content: u64, hyps: u64 }`) — é opaco por construção. Se hipóteses passarem a afetar o resultado de um estágio (o que hoje **não pode acontecer**, seção 2) sem entrar em `composite_fingerprint`, duas consultas contra o mesmo `metric` sob hipóteses diferentes colidem na mesma chave e a segunda recebe silenciosamente o `Grid`/`Expr` cacheado da primeira — resultado errado, sem erro, sem log. Exatamente o cenário que a pergunta descreveu, confirmado como real *assim que* hipóteses existirem, não hoje.

**Recomendação: não tocar a chave agora** — não há nada para incluir ainda (é geometricamente honesto que a chave de hoje só reflita o que hoje pode variar). Mas isto é o único item das cinco perguntas onde a ordem de execução importa de um jeito diferente dos outros: quando o passo 2 (hipóteses) for implementado, estender `composite_fingerprint` para hashear também o conjunto de hipóteses ativas **não pode ser um follow-up** — tem que entrar no mesmo PR que introduz hipóteses, porque o modo de falha é resposta errada silenciosa, não pane. Isso é uma condição de aceitação para o passo 2, registrada aqui para não ser esquecida quando aquele round chegar, não trabalho a fazer agora.

---

## 4. Cancelamento como pré-requisito

**A pergunta:** onde a verificação precisa entrar para que todo cálculo novo a herde por construção? Existe hoje um ponto único?

**Resposta real, mais precisa que a premissa: não, hoje não existe um único ponto — existem dois runners de topo, e só um arma cancelamento profundo.** Isto é o achado mais importante desta rodada.

**Mecanismo 1 — thread-local, alcança qualquer profundidade** (`oderom-expr/src/cancel.rs`): `run_cancellable(token, f)` arma um `CancelToken` numa thread-local (`CancellationScope`) e roda `f` sob `catch_unwind`; `check_cancelled()` (`pub(crate)`, não `pub`) faz `panic::panic_any(Cancelled(()))` se o token foi cancelado. Chamado, sem condição, na primeiríssima linha de `normalize_via_rational_form` (`canonical.rs:37`) — **toda** chamada, incluindo as recursivas de canonicalização de argumento de `sin`/`cos` — mais dentro de `poly_gcd_bounded` e da iteração do PRS de subresultantes (`rational_function.rs`). Overhead medido, não estimado: 1,44 ns/chamada (`cancel.rs`, teste `check_cancelled_overhead_is_negligible`).

**Mecanismo 2 — closure explícita, por componente** (`oderom-components/src/curvature.rs:66`): `pub type Checkpoint<'a> = &'a mut dyn FnMut() -> bool`, passada como último parâmetro de cada função `*_checkpointed` (`christoffel_checkpointed`, `riemann_mixed_checkpointed`, etc.), checada uma vez por tupla de índice independente.

**Quem arma o Mecanismo 1 hoje:** só `oderom-session::run.rs::run_query` (`run.rs:63`): `oderom_expr::run_cancellable(ctx.token(), || run_query_inner(...))`. Esse é o caminho do REPL e do notebook/GUI — o único lugar onde o `CancelToken` do `ExecutionContext` também vira o token do `cancel.rs`.

**O que NÃO arma o Mecanismo 1:** `oderom-cli::commands.rs::run_with_budget` (`commands.rs:277`) — o caminho do binário `oderom` de linha de comando (as cinco subcommands `christoffel`/`riemann`/`ricci`/`scalar`/`kretschmann` standalone). Essa função só faz uma corrida de timeout de parede (`mpsc::recv_timeout`) contra uma thread desanexada — se o timeout dispara, a thread continua rodando, órfã, até o processo terminar; `run_cancellable` nunca é chamado nesse caminho (grep confirma: `run_cancellable` só aparece em `oderom-session/src/run.rs` no workspace inteiro fora de comentários). O Mecanismo 2 também fica efetivamente inerte aqui: os closures recebem `&mut || ctx.is_cancelled()`, mas nada nesse caminho jamais chama `ctx.cancel()` — não há Ctrl+C, não há botão de cancelar, é um binário de tiro único.

**Precedente real do custo de não seguir a regra:** `oderom-components/src/holonomy.rs` (Marco 5, o integrador RK4 de geodésica/transporte paralelo) não tem **nenhum** dos dois mecanismos — nem `Checkpoint`, nem qualquer verificação de cancelamento. É um laço numérico já em produção, já fora do motor `Expr`/`normalize()`, construído sem o gancho. Isto não é hipotético: é exatamente a dívida que a pergunta pede para não repetir, já paga uma vez.

**A regra, como política (documentada aqui, sem código novo neste round):**

1. Todo caminho de consulta novo deve ser alcançável exclusivamente através de `oderom-session::run_query` (ou equivalente que já chame `run_cancellable`) — nunca conectado apenas ao caminho do binário `oderom` standalone (`run_with_budget`), que hoje não arma cancelamento profundo. Se algum dia o binário standalone precisar da mesma garantia, isso é uma lacuna separada e pré-existente (`run_with_budget` não chama `run_cancellable`) — não é deste round, mas fica registrada.
2. Todo laço caro novo precisa, desde a primeira versão, de pelo menos um dos dois: (a) expressar sua aritmética via `Expr`/`normalize()` (herda o Mecanismo 1 de graça, já que toda chamada de `normalize()` já verifica), ou (b) se for um laço numérico fora de `Expr` (o próximo `holonomy.rs`, por exemplo, para o passo 5), aceitar um parâmetro no formato `Checkpoint` desde a assinatura inicial — copiar a forma que `curvature.rs` já estabeleceu, não inventar uma nova.
3. `check_cancelled` é `pub(crate)` dentro de `oderom-expr` — um algoritmo novo pesado que viva **fora** desse crate (um futuro `oderom-solve`, ou código novo dentro de `oderom-components` que não passe por `Expr`) não consegue chamá-lo hoje. Ou o trabalho pesado do passo 4/5 fica dentro de `oderom-expr` (onde já pode chamar), ou `check_cancelled` precisa de uma exposição pública estreita antes desse trabalho começar — sinalizado aqui, não feito agora, porque não há ainda nenhum chamador concreto esperando por ela.

**Recomendação:** nada de código agora. O documento — esta regra, por escrito — é a preparação. Custo de registrar a regra: zero. Custo de não a seguir quando o passo 4/5 começar: o mesmo que já foi pago uma vez em `holonomy.rs`.

---

## 5. Sintaxe como contrato

**A pergunta:** quais formas sintáticas os passos 1–5 vão exigir, e alguma colide com o que a gramática já usa para outra coisa?

O bloco/consulta de hoje é despachado por `classify_block` (`oderom-cli/src/parser.rs:363`) puramente pela palavra-chave inicial, contra duas listas fixas e disjuntas: `DECLARATION_KEYWORDS` (`manifold`, `bundle`, `head`, `chart`, `metric`, `connection`) e as cinco de `CommandName::from_str` (`christoffel`, `riemann`, `ricci`, `scalar`, `kretschmann`). A gramática de consulta hoje é só `QUERY := COMMAND_NAME IDENT?` (`parser.rs:297`) — nenhuma consulta aceita uma expressão como argumento ainda; os passos 1–5 vão precisar disso (ex.: "resolva `g_tt = 0` para `M`", "derive `g_tt` em `r`"), o que é uma variante nova em `Query` e um ramo novo em `parse_query_tokens` — aditivo, não uma reescrita, mas fora do escopo deste round.

Verificação forma a forma, contra o léxico/parser reais (`Lexer::next_tok`, `parser.rs:67`; `expr_parser.rs`):

| Forma | Estado hoje | Evidência |
|---|---|---|
| **Função indeterminada** `f(r)` | **Livre, e já rejeitada com erro limpo.** `parse_atom`'s `Tok::Ident` já trata `IDENT (` como chamada de função (`parse_ascii_function`, `expr_parser.rs:131`) — mas só aceita `sin`/`cos` (`trig_of`); qualquer outro nome já produz `"unknown function `{name}` (only sin/cos are supported)"`. Não é interpretado como multiplicação por justaposição (a regra de justaposição só entra depois, em `parse_product`, e o ramo `Ident` já desviou para `parse_ascii_function` antes disso). | `expr_parser.rs:112-138` |
| **Derivada** `f'`, `f''` (marca de prima) | **Livre.** `'` não tem tratamento em lugar nenhum da gramática — cai no ramo genérico do léxer (`Some(c) => Tok::Sym(c)`, `parser.rs:142-145`), vira `Sym('\'')`, e não é consumido por `parse_atom`/`parse_power`/`parse_product`/`parse_sum`. Grep confirma zero referências a `Sym('\'')` em `parser.rs` ou `expr_parser.rs`. Hoje, usar `'` produz token sobrando → erro de parse explícito, nunca reinterpretação silenciosa. | grep, sem resultado |
| **Equação** `lhs = rhs` | **Livre no nível de consulta.** `Sym('=')` só é consumido em um lugar hoje: `index_resolve.rs::parse_component_line` (`toks.expect_sym('=')`, linha 48), dentro do corpo de `metric`/`connection` (`[idx] = EXPR`). Uma variante nova de `Query` que use `=` como operador de equação seria uma produção de topo separada, nunca aninhada dentro de um corpo de `metric`/`connection` — mesmo token, contextos disjuntos, exatamente como `on`/`dim`/`coords` já são palavras-chave sensíveis ao contexto (não reservadas globalmente) nesta gramática. | `index_resolve.rs:23-49` |
| **Hipótese** (ex. `assume r > 0`) | **Livre.** Nenhuma das palavras candidatas (`assume`, `given`, `hypothesis`, `where`, `solve`, `let`, `for`, `diff`) colide com `DECLARATION_KEYWORDS`, `CommandName`, ou `GREEK_LETTERS` (`oderom-expr/src/render.rs:165`) — grep sem resultado para todas. `>`/`<` (`Sym('>')`/`Sym('<')`) não aparecem em lugar nenhum da gramática hoje — livres para um predicado de comparação. | grep, sem resultado |

**Veredito: toda forma listada está livre hoje, e cada uma, se usada mal-formada agora, já produz erro de parse explícito** — porque `classify_block`, `parse_query_tokens` e `parse_ascii_function` já rejeitam token/nome desconhecido em vez de reinterpretar silenciosamente. Isso já é, em espírito, o "reconhece e rejeita com not-yet-implemented" que o pedido descreveu — só falta a mensagem dizer isso explicitamente em vez de "unknown".

**Recomendação:** esta é a única das cinco perguntas onde uma checagem *foi* o trabalho que valia a pena adiantar — e já está feita, por escrito, acima: nenhuma forma precisa ser resgatada de outro uso depois. O que fica para depois é a reserva *ativa* (fazer o parser reconhecer `f'(r)`/`assume ... > 0`/uma consulta de equação e rejeitar com uma mensagem específica "ainda não implementado"), porque isso exige decidir a sintaxe exata agora (`f'(r)` ou `df/dr`? `assume` ou `given`?) — e a própria instrução do pedido é para não desenhar os passos 4–6 ainda. Reservar a forma errada tem o mesmo custo de nunca ter reservado nada.

---

## Proibição: vazamento de índice abstrato

Nenhum desenho do subsistema de índices abstratos (passo 6) — só onde algo dele poderia vazar para o motor de componentes hoje, e como impedir.

**A fronteira já é limpa, por dependência de crate, não por convenção.** `oderom-expr/Cargo.toml` depende só de `oderom-core`, e só usa três itens de lá: `Render`/`Target` (`render.rs`, `big_scalar.rs`) e `CancelToken` (`cancel.rs`) — nada de `Registry`, `HeadId`, `AbstractIndex`, `Monomial`, `SlotSig`, símetria. `Expr` não tem, em lugar nenhum, uma variante tipada por índice abstrato. Quem consome `Registry`/`Chart`/`HeadId` é `oderom-components::curvature.rs` — e faz isso para resolver tuplas de índice **concretas** (`Grid`, indexado por `u8` cru) antes de qualquer coisa tocar `Expr`. `index_resolve.rs` documenta essa separação diretamente: "Marco 1's abstract tensor-index syntax (`R[a,b,c,d]`, parsed by `parser::parse_monomial`) is a separate grammar that never consults a `Chart` at all."

Dois lugares concretos onde a tentação de vazar aparece quando o passo 4 for implementado:

1. **`Expr`/`AtomKey` (`poly.rs`)** — a tentação, ao construir o átomo de função indeterminada, é fazê-lo genérico o bastante para também carregar um slot de tensor/`AbstractIndex` "para aproveitar depois". Recomendação: o átomo novo só deve envolver um `Expr` escalar como argumento, no mesmo molde exato de `Sin`/`Cos` — nunca uma referência a `HeadId`/`AbstractIndex`/`Registry`.
2. **`expr_parser.rs` (SCALAR_EXPR) vs `parser.rs` (`parse_monomial`, gramática abstrata)** — qualquer forma nova de SCALAR_EXPR (chamada de função, marca de derivada, equação, hipótese) deve entrar só em `expr_parser.rs`. Nunca implementar estendendo `parse_monomial`, e nunca fazer SCALAR_EXPR aceitar um token de índice abstrato.

Nenhuma mudança feita; ambos os pontos já estão corretos hoje. Isto é só o mapa de onde vigiar quando o passo 4 chegar.

---

## Recomendação final de ordem

**Fazer agora** (barato, e caro ou arriscado se adiado):

1. Este documento — feito.
2. A checagem de colisão de sintaxe da seção 5 — feita, registrada; nenhuma ação de parser necessária ainda, mas o resultado ("nenhuma forma precisa ser resgatada depois") é o tipo de fato que só vale a pena estabelecer uma vez, com evidência, não reconferir a cada rodada.
3. A regra de cancelamento da seção 4, como política escrita, não como código: todo cálculo novo entra por `oderom-session::run_query` (ou equivalente já armado), e todo laço não-`Expr` novo nasce com um parâmetro `Checkpoint`-shaped. Custo de registrar: zero. Custo de não seguir: já pago uma vez (`holonomy.rs`).

**Adiar para quando o passo correspondente chegar** (reversível, especulativo, ou sem economia em adiantar):

1. A variante nova de `Expr` (seção 1) — os ~11 pontos de match já estão listados; nenhum será redescoberto por acidente quando o passo 4 começar.
2. O parâmetro de contexto de hipóteses em `normalize` (seção 2) — mesmo custo mecânico agora ou depois, e um parâmetro vazio hoje seria uma assinatura enganosa.
3. A dimensão de hipóteses na chave do cache (seção 3) — mudança de uma função, um sítio de chamada; só precisa entrar no mesmo round que introduzir hipóteses, não antes (e não depois, por causa do risco de resposta errada silenciosa).
4. A reserva ativa de sintaxe (`f'(r)`, `assume ... > 0`, consulta de equação) — exige decidir a forma exata, que é desenho do passo 1/4, fora do escopo deste round.
5. Expor `check_cancelled` como `pub` fora de `oderom-expr` — só quando um chamador concreto (fora do crate) precisar dele.
