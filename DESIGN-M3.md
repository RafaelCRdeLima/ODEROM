# ODEROM — DESIGN-M3.md (Marco 3: atlas)

**Status: implementado.** Ver [README.md](README.md#marco-3-status). D3.1 foi confirmada como proposta (sem SMT de verdade agora). A verificação de invariância da métrica precisou de uma peça a mais do que este documento previa: `oderom-expr` ganhou `rationalize` (numerador/denominador explícitos via recursão, em vez de inferir a divisão por casamento de padrão em `normalize`), depois que uma correção local que resolvia o caso da esfera quebrou o teste de Kretschmann do Marco 2 — os dois precisam de comportamentos que se contradizem dentro do `normalize()` de reescrita local.

Mesma regra dos marcos anteriores: proposta, não começo de implementação. Antes de tudo, uma decisão que quero seu ok explícito porque muda a política de dependências do projeto.

## 0. Escopo declarado

"Marco 3 — atlas com múltiplas cartas, transições, domínios com obrigações via SMT. Aceitação: declarar `S²` com duas cartas estereográficas e verificar que a métrica é invariante na transição."

## 1. A decisão que preciso da sua opinião antes de desenhar o resto

**"Obrigações via SMT" pede um solver SMT de verdade (Z3 ou similar) como dependência externa.** Isso é uma categoria de dependência completamente diferente das usadas até agora (`thiserror`, `smallvec`, `rustc-hash`, `proptest`, `criterion` — todas puras-Rust, sem binário externo). Um binding para Z3 (`z3` crate) normalmente exige a biblioteca Z3 instalada no sistema (ou a feature `bundled`, que compila Z3 do zero em C++ — build muito mais pesado e lento). Isso é uma mudança de peso real no projeto: builds mais lentos, uma dependência de sistema, superfície de erro nova.

Verifiquei o critério de aceitação literal — "declarar S² com duas cartas estereográficas e verificar que a métrica é invariante na transição" — e ele **não exige SMT de verdade**. É uma verificação simbólica direta: pego a métrica declarada na carta B, substituo as coordenadas pela função de transição vinda da carta A, multiplico pelo jacobiano, normalizo, e comparo por igualdade estrutural com a métrica declarada na carta A. Isso é exatamente o tipo de máquina que `oderom-expr` já tem (só falta um primitivo de substituição de variável, que é uma extensão pequena e não-controversa).

Onde SMT de verdade faria diferença é em obrigações mais gerais — por exemplo, "prove que toda carta do atlas cobre a variedade" ou "prove que duas cartas não se sobrepõem indevidamente" — afirmações logicamente não-triviais sobre desigualdades, não uma identidade simbólica pontual. Nenhum teste de aceitação deste marco pede isso.

**Três caminhos possíveis:**

1. **Sem SMT agora.** Domínio de carta = uma lista de predicados simbólicos (`Expr != 0`, `Expr > 0`, ...) guardados como dado — documentação executável de onde a carta vale — sem motor de prova nenhum por trás. A verificação de invariância da métrica (o teste de aceitação de verdade) é feita por substituição + normalização, sem nunca consultar um solver. "Obrigações via SMT" fica adiada para quando um teste de aceitação realmente precisar de prova automática sobre desigualdades — e aí value a pena pagar o custo da dependência.
2. **SMT via crate `z3`, com sinalização.** Implemento a integração agora, mas devo perguntar antes de adicionar a dependência (regra já em vigor desde o Marco 1) — o que estou fazendo aqui, agora.
3. **SMT "de brinquedo"**: um decisor bem pequeno, escrito à mão, só para as formas de predicado que realmente aparecem (comparações de sinal de expressões racionais/trigonométricas simples) — sem trazer um SMT solver de verdade, mas também sem ser um SMT solver de verdade (viraria uma promessa que não cumpre).

Minha recomendação é (1): implementa o que o critério de aceitação pede sem gastar a dependência pesada agora, e deixo a estrutura pronta (`Domain` já carrega predicados desde já) para (2) entrar depois sem reescrever nada — mesmo padrão que usei no Marco 1 com `Domain::Everywhere` já antecipando este marco. Mas quero seu aval, porque "SMT" estava escrito explicitamente no seu roteiro original e eu não quero decidir sozinho que isso não é preciso agora.

## 2. O que é novo em relação ao Marco 2

Marco 2 tinha **uma carta por variedade**, sem domínio restrito (`Domain::Everywhere` fixo). Marco 3 precisa de:

- **Múltiplas cartas por variedade**, cada uma com seu próprio domínio de validade.
- **Funções de transição**: dada uma carta A e uma carta B cujos domínios se sobrepõem, uma função `Expr` por coordenada de B em termos das coordenadas de A (e a inversa).
- **Substituição de variável em `Expr`** — primitivo novo em `oderom-expr`, não existia porque Marco 2 nunca precisou trocar uma coordenada por uma expressão em termos de outra.
- **Domínio como predicado**, não mais só `Everywhere`.

## 3. Estruturas propostas

### 3.1 `oderom-expr`: substituição

```rust
/// Substitui toda ocorrência da variável `var` por `replacement`.
pub fn substitute(expr: &Expr, var: &str, replacement: &Expr) -> Expr;
```

Recursão estrutural direta (igual a `diff`), sem ambiguidade matemática — não deveria precisar de decisão nenhuma da sua parte.

### 3.2 `oderom-components`: domínio como predicado

```rust
pub enum Predicate {
    Ne(Expr),   // expr != 0
    Gt(Expr),   // expr > 0
    // Ge/Lt/Le se algum teste precisar; não adiciono sem precisar.
}

pub enum Domain {
    Everywhere,
    Restricted(Vec<Predicate>),  // conjunção (E lógico) dos predicados
}
```

`Chart` ganha um campo `domain: Domain`. Nenhum motor de prova consome isso neste marco — é dado estrutural, testável por igualdade/inspeção, não por dedução.

### 3.3 `oderom-components`: atlas e transição

```rust
pub struct ChartId(u32);

pub struct TransitionMap {
    pub from: ChartId,
    pub to: ChartId,
    /// to.coords[i] expresso em termos de from.coords
    pub forward: Vec<Expr>,
}

pub struct Atlas {
    pub manifold: ManifoldId,
    charts: Vec<(ChartId, Chart)>,
    transitions: FxHashMap<(ChartId, ChartId), TransitionMap>,
}
```

### 3.4 Verificação de invariância da métrica (o próprio teste de aceitação)

```rust
/// Substitui as coordenadas de `to` pela função de transição, multiplica
/// pelo jacobiano da transição, normaliza, e compara com a métrica
/// declarada na carta `from`.
pub fn metric_agrees_across_transition(
    registry: &Registry,
    g_from: &ComponentTensor,
    g_to: &ComponentTensor,
    transition: &TransitionMap,
) -> Result<bool, ComponentError>;
```

Fórmula padrão de pullback: `g_from[i,j] == sum_{k,l} g_to[k,l] * (d(to_k)/d(from_i)) * (d(to_l)/d(from_j))`, cada `d(to_k)/d(from_i)` vindo de `diff` sobre `transition.forward[k]`.

## 4. Teste de aceitação: S² com duas cartas estereográficas

- Carta N (projeção do polo norte): coordenadas `(u,v)`, domínio `Everywhere` por simplicidade (o polo norte em si — ponto único — não afeta a igualdade simbólica que o teste checa; document isso como simplificação, não estou modelando a variedade removendo um ponto de verdade).
- Métrica em N: `ds² = 4(du²+dv²)/(1+u²+v²)²`.
- Carta S (projeção do polo sul): coordenadas `(u',v')`, mesma forma de métrica por simetria.
- Transição: `u' = u/(u²+v²)`, `v' = v/(u²+v²)` (inversão) — a função de transição estereográfica clássica.
- Verificação: `metric_agrees_across_transition` dá `true`.

## 5. Fora de escopo (repetindo o padrão)

SMT de verdade (adiado por D3.1, se você concordar). Cartas com domínio removendo pontos de fato (o teste de aceitação não distingue "vale no ponto" de "vale simbolicamente ali"). Geodésicas/holonomia (Marco 5). E-grafo (Marco 4).

## 6. Questões abertas

**D3.1** — confirmar a decisão da seção 1: sem SMT de verdade neste marco, `Domain` só como dado estrutural.

**D3.2** — a fórmula de pullback em 3.4 assume que ambas as métricas já estão diagonais/conhecidas nas suas próprias cartas (reaproveita a mesma restrição do Marco 2, D-M2.1, para inversão/contração). Confirma que está certo continuar com essa restrição em vez de generalizar agora?

---

Aguardando seu ok, principalmente em D3.1.
