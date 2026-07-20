# ODEROM — DESIGN-UI.md (UI: exploração simbólica de Geometria Diferencial)

**Status: implementado -- Camada A (biblioteca, seção 5) e Camada A.2
(CLI, seção 6).** Ver [README.md](README.md#ui-status-camada-a2--cli)
para o resultado (`christoffel`/`riemann`/`ricci`/`scalar`/`kretschmann`
lendo `.od`, front-ends ASCII e LaTeX confirmados lado a lado produzindo
o mesmo resultado ponta a ponta). GUI/kernel Jupyter permanecem fora de
escopo, por instrução explícita -- a existência do alvo `json` desde já é
justamente para não fechar essa porta.

Mesma regra dos marcos anteriores: proposta, não começo de implementação.

## 0. O que o levantamento do estado atual mostrou

Fui ver o que já existe de "mostrar resultado para um humano" no projeto, e o
quadro é mais estreito do que eu esperava:

- `oderom-cli` tem **um** subcomando, `canon <expressão>`. Ele só exercita o
  Marco 1: lê um `prelude.od` (declarações de `manifold`/`bundle`/`head`),
  faz *parse* de um monômio abstrato, canonicaliza, e imprime o resultado via
  `format_monomial` — uma função **privada** do crate do CLI, não reutilizável
  em lugar nenhum.
- `oderom_expr::Expr` — o CAS escalar que sustenta Christoffel, Riemann,
  Ricci, Kretschmann e os componentes de métrica dos Marcos 2 a 5 — **não
  tem `Display` nenhum**. Só o `Debug` derivado, que imprime a árvore de
  enum crua (`Add(Mul(Rational(48, 1), Pow(Var("M"), 2)), ...)`), ilegível.
- `Grid`/`ComponentTensor` (Marco 2/3), `Polynomial` (usado na extração do
  e-grafo, Marco 4) e `oderom_jit::Program` (Marco 5) — mesma história,
  nenhum tem forma legível de imprimir o que carregam.
- As únicas coisas com `Display` de verdade hoje são `oderom_core::Scalar`
  (números racionais) e o `format_monomial` do CLI (só Marco 1).

Ou seja: **hoje não existe nenhuma forma de rodar um comando e ler um
símbolo de Christoffel, um componente de Riemann, ou o resultado de uma
integração geodésica** — a única forma de "ver" esses resultados é ler o
código-fonte dos testes e seus `assert_eq!`. "Pensar na UI" tem um
pré-requisito que ainda não existe: uma camada de *pretty-print* para
`Expr`. Sem isso, uma GUI só teria números/árvores ilegíveis para mostrar.

## 1. Proposta: duas camadas, sequenciadas

**Camada A — texto legível (sem UI gráfica ainda).** `impl Display for
Expr` (notação infixa convencional: `48*M^2/r^6`, não a árvore de enum),
mais alguns subcomandos novos no CLI que de fato cheguem aos Marcos 2–5
(hoje só o Marco 1 é alcançável de fora do código Rust). Isso sozinho já é
um salto grande sobre o estado atual (visibilidade zero), e é o
pré-requisito real de qualquer UI gráfica depois.

**Camada B — UI de verdade.** Só depois de existir "peça Christoffel de
tal métrica, leia uma resposta legível" é que faz sentido decidir entre
ficar em CLI+texto, uma GUI nativa (`egui`, dependência nova a aprovar) ou
outra coisa — decisão adiada para depois de ver a Camada A funcionando,
igual discutimos antes.

## 2. Escopo proposto para a Camada A

- `impl Display for Expr` em `oderom-expr`: infixo, agrupado por
  precedência (soma < produto < potência), sem parênteses redundantes;
  frações/negativos tratados como `Scalar::Display` já trata.
- Extrair `format_monomial` do CLI para uma função pública reutilizável
  (provavelmente em `oderom-canon`, que já depende de `oderom-core` e
  conhece `Monomial`/`Registry`).
- Novos subcomandos no CLI, mínimo proposto: `oderom christoffel
  <metric-file>` (imprime os `Γ^i_{jk}` não nulos numa carta). Se isso for
  aprovado, `riemann`/`ricci`/`kretschmann` são extensões diretas do mesmo
  mecanismo, e um comando para o Marco 5 (`oderom geodesic ...`) imprimiria
  a trajetória como tabela de pontos.
- Formato do "metric-file": minha proposta é estender a gramática que
  `prelude.od` já tem (declarativa, mesmo estilo) para também aceitar
  componentes de métrica numa carta, em vez de inventar um formato do
  zero.

## 3. Fora de escopo (por enquanto)

Qualquer UI gráfica (`egui`, web, TUI). Saída em LaTeX/typeset. Qualquer
dependência nova — Camada A inteira é texto puro, zero dependências além
das já aprovadas.

## 4. Questões abertas

**D-UI.1** — confirma a sequência (Camada A primeiro — texto/CLI — antes
de decidir se/qual UI gráfica vem depois)?

**D-UI.2** — o escopo da Camada A na seção 2 é o recorte certo, ou prefere
algo menor/maior para começar (ex.: só `Display for Expr`, sem mexer no
CLI ainda; ou incluir Marco 5 desde já)?

**D-UI.3** — formato do arquivo de métrica: estender `prelude.od` como
proposto, ou prefere um formato separado?

## 5. Ajustes que você pediu, e como ficaram implementados

Você deu "ok" na Camada A com três correções à seção 2 original. Registro
aqui o que mudou de fato, para o doc continuar sendo a referência real:

1. **Trait de renderização com alvos, não só `Display`.** Em vez de só
   `impl Display for Expr`, existe `oderom_core::render::{Render, Target}`
   (`Target::{Unicode, Latex, Json}`), implementado para `Scalar`
   (`oderom-core`) e para `Expr` (`oderom-expr`). `Display for Expr` é
   `render(Target::Unicode)`. O alvo `Latex` não é tratado como opcional:
   `Expr`'s LaTeX usa `\frac{}{}`, `\sin`/`\cos` com `\left(\right)`, e
   reconhece nomes gregos comuns (`theta` -> `\theta`) — comuns como nome
   de coordenada/índice neste projeto. O trait vive em `oderom-core` (não
   em `oderom-expr`) porque é a raiz de dependência comum a todo o
   workspace — reutilizável por qualquer camada futura sem depender de
   `oderom-expr`.

2. **O conteúdo real é elisão, não formatação.**
   `oderom_components::render` (`classify_tensor`/`classify_grid` +
   `render_classes`) mostra só os componentes independentes de um tensor
   sob o grupo de simetria do seu `TensorHead` (reaproveitando o mesmo
   `Bsgs`/`canonical_indices` que `ComponentTensor::set` já usa para
   compressão de órbitas — nenhum mecanismo novo de simetria), anota cada
   linha com o tamanho da órbita ("N componentes por simetria"), suprime
   componentes identicamente nulos num único contador, e trunca
   explicitamente (limite passado pelo chamador, nunca implícito). Isso
   mora em `oderom-components` (junto de `ComponentTensor`, que já
   depende do `Bsgs` do núcleo pelo mesmo motivo) e não na CLI, porque
   "quais componentes são independentes" é uma pergunta sobre o grupo de
   simetria do tensor, não sobre onde o resultado é impresso. `Grid`
   (Christoffel, sem grupo declarado) usa a mesma função de
   renderização, só que toda órbita tem tamanho 1 — supressão de zero e
   truncamento continuam se aplicando.

3. **Disciplina de teste.** Nenhum teste de corretude matemática (Marco
   2-5: Kretschmann, Ricci, transição de carta em S², Bianchi, holonomia)
   foi tocado, e nenhum teste novo compara resultado numérico/simbólico
   via string renderizada — igualdade continua sendo `Expr`
   normalizado/estrutural. Os testes novos em `oderom-expr::render` e
   `oderom-components::render` são golden-strings explicitamente
   documentados como testando *o formato do renderizador*, não a
   matemática (comentário em cada um dizendo isso).

O que ficou de fora desta passada: qualquer coisa na CLI. A seção 2
original propunha `oderom christoffel <metric-file>` e estender a
gramática do `prelude.od`; isso dependia de D-UI.3 (formato do arquivo de
métrica), que você não confirmou explicitamente ao aprovar a Camada A —
preferi não adivinhar um formato de arquivo a implementar algo que talvez
precise ser desfeito.

---

## 6. Camada A.2: CLI (proposta, aguardando ok — nada disto está implementado)

D-UI.3 decidido: estender `.od`, uma linguagem só, sem formato à parte.
`chart`/`metric`/`connection` são declarações novas na mesma gramática de
`manifold`/`bundle`/`head`; o front-end LaTeX-flavored baixa para a
*mesma* AST de expressão que o front-end ASCII, nunca dois back-ends.

### 6.0 Pergunta 1: a camada de componentes aguenta uma métrica arbitrária de arquivo?

Sim, sem nenhuma mudança em `oderom-components`. `christoffel`,
`riemann_mixed`, `lower_first_index`, `ricci_tensor`, `ricci_scalar`,
`kretschmann` (todas em `curvature.rs`) já são genéricas em dimensão,
nomes de coordenada e forma funcional dos componentes — nada ali é
específico de Schwarzschild ou S², os testes só *escolhem* essas métricas,
o código não as conhece. A única restrição real é **métrica diagonal**
(`D-M2.1`, decisão sua de antes desta conversa, não algo que eu esteja
introduzindo agora): `metric_inverse_diagonal` checa todo componente
fora da diagonal e retorna `ComponentError::NonDiagonalMetric` se algum
for não-nulo (depois de `normalize`). Então: uma métrica diagonal nova,
nunca vista, em qualquer dimensão, com qualquer nome de coordenada e
qualquer expressão racional/trigonométrica nos componentes, já passa
pelo pipeline inteiro sem tocar em Rust. Uma métrica com termo cruzado
(Kerr, por exemplo) dá erro limpo, não resultado errado — não é um bug,
é o escopo já combinado no Marco 2. Se isso for um problema para o
critério de aceitação que você tem em mente, preciso saber antes de
seguir — não é trabalho grande consertar (inversão simbólica geral por
cofatores), mas é trabalho novo, fora do que foi pedido agora.

### 6.1 Arquitetura: um parser de expressão, duas grafias de token

Both front-ends produce the same `oderom_expr::Expr` and the same new
`Decl` enum (below); a parsed file becomes one `Model` (a new struct
alongside `Registry`, since charts/metrics/connections are Marco 2-level
concrete objects, not Marco 1's abstract algebra):

```rust
struct Model {
    registry: Registry,
    charts: HashMap<String, Chart>,
    metrics: HashMap<String, (HeadId, ComponentTensor)>,
    connections: HashMap<String, (String /* chart name */, Grid)>,
}
```

A gramática de `SCALAR_EXPR` é uma só, com produções que aceitam duas
grafias de token no mesmo ponto (não são duas gramáticas paralelas — é
uma gramática com sinônimos léxicos):

```text
SUM     := PRODUCT (('+' | '-') PRODUCT)*
PRODUCT := POWER (('*' | '/') POWER
                  | '\frac' '{' SCALAR_EXPR '}' '{' SCALAR_EXPR '}')*
POWER   := UNARY ('^' SIGNED_INT | '^' '{' SIGNED_INT '}')?
UNARY   := '-' UNARY | ATOM
ATOM    := INT | INT '/' INT | VAR
          | ('sin'|'cos') '(' SCALAR_EXPR ')'
          | ('\sin'|'\cos') ('(' SCALAR_EXPR ')' | '{' SCALAR_EXPR '}')
          | ('\sin'|'\cos') '^' INT ('(' SCALAR_EXPR ')' | '{' SCALAR_EXPR '}')   -- sin^2(x) sugar for sin(x)^2
          | '(' SCALAR_EXPR ')' | '\left' '(' SCALAR_EXPR '\right' ')'
VAR     := IDENT | GREEK_MACRO   -- '\theta' etc. -> Var("theta"), same
                                     table `oderom_expr::render`'s LaTeX
                                     target uses, exposed as one shared
                                     `pub const GREEK: &[&str]` so the
                                     parser and renderer can never drift
```

Exponents ficam restritos a inteiro (com sinal), nunca uma subexpressão
geral — é literalmente tudo que `Expr::Pow(Box<Expr>, i32)` consegue
representar, então o parser rejeita `x^y` (`y` não-literal) na gramática,
não em tempo de execução. `sin`/`cos` são as únicas funções -- qualquer
outro identificador seguido de `(`/`{` é erro de parse, pelo mesmo motivo
(`Expr` não tem um nó de "chamada de função" genérico).

### 6.2 Declarações novas, lado a lado

**`chart`** (uma só grafia — não há "LaTeX de declaração de carta"):

```text
chart NAME on MANIFOLD coords (c1, c2, ..., cn)
```

Valida `n == manifold.dim`. Registra `NAME` em `Model.charts` para
`metric`/`connection` referenciarem depois.

**`metric`**, ASCII:

```text
metric NAME on CHART bundle BUNDLE {
  [i1,i2] = SCALAR_EXPR (',' [i1,i2] = SCALAR_EXPR)*
}
-- i1,i2 := nome de coordenada da CHART, ou inteiro 0-based
```

Exemplo (Schwarzschild):

```text
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r),
  [r,r] = 1/(1 - 2*M/r),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
```

**`metric`**, mesma declaração, subconjunto LaTeX (mesmo `Decl::Metric`,
mesma `SCALAR_EXPR`, só a grafia de token muda -- inclusive dá pra
misturar as duas dentro da mesma expressão, já que é uma gramática só):

```text
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  g_{tt} = -\left(1 - \frac{2M}{r}\right),
  g_{rr} = \frac{1}{1 - \frac{2M}{r}},
  g_{\theta\theta} = r^2,
  g_{\phi\phi} = r^2 \sin^2(\theta)
}
```

`g_{tt}` é açúcar para `[t,t]` -- ver 6.3, essa é a única parte que ainda
não decidi sozinho.

**`connection`** (Γ declarado direto, sem passar por métrica -- caminho
alternativo para conexões afins não necessariamente Levi-Civita de
nenhuma métrica, ou só para inspecionar um Γ que você já tem):

```text
connection NAME on CHART {
  [i1,i2,i3] = SCALAR_EXPR (',' ...)*
}
```

Mesma sintaxe de índice (nome de coordenada ou inteiro), mesmas duas
grafias de expressão. Sem grupo de simetria (Christoffel não é tensor,
igual ao `Grid` que `curvature::christoffel` já produz).

### 6.3 Índice colado: decomposição por retrocesso, decidida por token

Decisão final (não é mais modo por carta -- era o problema real do que eu
tinha proposto: renomear uma coordenada, ou acrescentar uma
multi-caractere, invalidaria o arquivo inteiro, inclusive linhas sem
ambiguidade nenhuma). A regra é por token, não por arquivo:

Dentro de um índice colado (sem vírgula), enumere **todas** as
decomposições completas do texto em nomes de coordenada declarados na
carta em questão, por busca com retrocesso (não maximal munch: em uma
carta com coordenadas `r` e `rho`, o texto `rhor` tem que considerar
`rho`+`r` mesmo depois de já ter tentado `r`+`hor` e falhado) --

- **exatamente uma decomposição de comprimento igual à aridade
  esperada** (2 para `metric`, 3 para `connection`) -> aceita;
- **zero** -> erro "`rhor` não é decomponível nas coordenadas de `CHART`
  (`t`, `r`, `theta`, `phi`)";
- **duas ou mais** -> erro listando as leituras (`rhor` pode ser
  `[rho, r]` ou, se a carta também tivesse uma coordenada `h` e `o`
  isoladas, `[r, h, o, r]`) e sugerindo a forma com vírgula, que resolve
  sempre, em qualquer carta, sem ambiguidade -- é o escape hatch.

A forma colada só existe em `_{...}` (subscrito, o jeito LaTeX de
escrever índice); `[...]` (colchete, o jeito ASCII) sempre exige vírgula
-- não é uma convenção natural em texto plano, então não vale a pena dar
a ela o mesmo poder de decomposição.

Macro grega (`\theta`, `\phi`, ...) dentro de um subscrito colado nunca
participa da busca de decomposição -- já é um token léxico inteiro,
delimitado pela própria barra, então `t\theta` (dois tokens: `Ident("t")`
e `Command("theta")`) nunca é ambíguo com nada. Só uma sequência de
caracteres ASCII sem barra entre eles (`rhor`) é uma string que precisa
ser decomposta.

### 6.3b Índice abstrato (Marco 1) vs. índice de coordenada (Marco 2): a regra de resolução

Pergunta que você levantou: numa carta cujas coordenadas se chamam `a` e
`b`, o que significa `_{ab}`/`[a,b]`?

**Regra, declarada aqui e não deixada para o parser inferir:** o tipo de
índice é decidido pelo **contexto gramatical da declaração em que ele
aparece**, nunca pelo texto do nome. Dentro de um bloco `metric`/
`connection` (entre o `{` e o `}` daquela declaração), todo índice --
colado ou com vírgula -- é sempre um índice de **coordenada concreta**,
resolvido contra a `chart` daquela declaração específica (nunca contra
nomes de índice abstrato de nenhum monômio em nenhuma outra parte do
arquivo). Dentro de uma expressão de monômio tensorial (o argumento de
`canon`, sintaxe `HEAD[i1,i2,...]` do Marco 1), todo índice é sempre um
**índice abstrato** (rótulo de contração do `oderom-core::Monomial`), e
nenhuma `chart` é sequer consultada -- essa gramática não sabe que
`chart`s existem.

As duas gramáticas nunca compartilham o mesmo par de colchetes/chaves --
a pergunta "e se o texto coincidir" nunca chega a se colocar, porque a
resposta já está decidida pela posição na gramática, antes de olhar para
o conteúdo. `chart schw on M coords (a, b)` é uma carta legal (não vou
proibir nomes de coordenada que coincidam com letras comuns de índice
abstrato -- a regra de contexto já resolve isso sem precisar de lista de
nomes reservados); dentro de `metric g on schw { ... }`, `g_{ab}` é
`g[a,b]` (posições 0 e 1 da carta `schw`); num `canon "R[a,b,c,d]"`
separado, `a`/`b`/`c`/`d` são rótulos de contração, ponto -- mesmo que
`schw` exista em algum lugar do mesmo arquivo, `canon` não olha para
cartas. Se algum dia for preciso misturar os dois níveis num único lugar
(por exemplo, popular um `ComponentTensor` a partir de uma fórmula que
usa índices abstratos do Marco 1), isso exige uma declaração nova e
distinta -- não uma reinterpretação de `metric`/`connection`/`canon`
como estão.

### 6.4 Subcomandos

```text
oderom christoffel FILE [--metric NAME | --connection NAME] [--target unicode|latex|json] [--max-lines N]
oderom riemann     FILE [--metric NAME | --connection NAME] [--target ...] [--max-lines N]
oderom ricci       FILE [--metric NAME | --connection NAME] [--target ...] [--max-lines N]
oderom scalar      FILE [--metric NAME]                      [--target ...]
oderom kretschmann FILE [--metric NAME]                      [--target ...]
```

`--metric`/`--connection` só são necessários se o arquivo declarar mais
de um; com exatamente um, é implícito (é o caso do critério de aceitação:
um metric, um comando, sem flag nenhuma). Resolução de Γ: se `--metric`
(ou único metric implícito) está presente, Γ sempre vem de
`christoffel()` (Levi-Civita), mesmo que o arquivo também tenha uma
`connection` declarada -- `--connection` só é usado quando não há
metric nenhum no arquivo (ou é pedido explicitamente). `scalar` e
`kretschmann` exigem metric (precisam de `g^ab` para inverter/contrair);
com só `connection` no arquivo, erro claro, não um número sem sentido.

`riemann`/`ricci` registram internamente (não expostos como declaração
do usuário -- é fato matemático, não dado) um head auxiliar com a
simetria certa (Riemann: ordem 8; Ricci: simétrico `(1 2)+`) só para
passar por `classify_tensor`/`render_classes` e mostrar componentes
independentes de verdade, não os `dim^rank` brutos de um `Grid`.
`christoffel` usa `classify_grid` (sem grupo -- Γ não é tensor).
`scalar`/`kretschmann` imprimem só `Expr::render(target)`, sem elisão
(são um número só).

### 6.5 Fora de escopo nesta rodada

GUI/kernel Jupyter (mantido, por instrução sua). LaTeX-flavor para
`manifold`/`bundle`/`head`/`chart` -- essas são declarações estruturais,
sem análogo físico natural em LaTeX, então ficam só na grafia por
palavra-chave já existente. Inversão de métrica não-diagonal (D-M2.1
continua de pé).

### 6.6 Uma extensão de gramática não documentada antes de implementar: justaposição

`SCALAR_EXPR` (seção 6.1) tinha `PRODUCT := UNARY (('*' | '/') UNARY)*`,
exigindo `*` explícito. O próprio exemplo que propus em 6.2
(`\frac{2M}{r}`) só faz sentido se "2M" for "2*M" -- então implementei
justaposição (nenhum operador entre dois átomos adjacentes, desde que o
próximo token não seja `+`/`-`, que ficam sempre no nível de `SUM`:
"2 - M" continua subtração, nunca "2 * (-M)"). Registrando aqui porque
mudou a gramática realmente implementada em relação ao que descrevi
antes -- não é uma decisão nova, é uma correção de uma inconsistência no
que eu tinha escrito.

---

Implementado: `oderom-cli` ganhou `chart`/`metric`/`connection` na mesma
gramática `.od` (`parser::parse_model`, substituindo `parse_prelude`),
`expr_parser` (`SCALAR_EXPR`, ASCII+LaTeX), `index_resolve`
(decomposição por retrocesso, 6.3/6.3b), e `commands`
(christoffel/riemann/ricci/scalar/kretschmann, resolução metric-vs-
connection de 6.4). Teste de ponta a ponta real
(`oderom-cli/tests/end_to_end.rs`) roda o binário compilado contra
arquivos `.od` de fixture (não código Rust construindo a métrica) e
confirma que a mesma métrica escrita em ASCII e em LaTeX produz
exatamente o mesmo Kretschmann renderizado. Workspace inteiro (`cargo
test`/`cargo clippy --all-targets`) limpo.
