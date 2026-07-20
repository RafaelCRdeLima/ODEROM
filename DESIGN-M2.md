# ODEROM — DESIGN-M2.md (Marco 2: componentes)

**Status: implementado.** Ver [README.md](README.md#marco-2-status) para o resultado (Kretschmann de Schwarzschild = 48M²/r⁶, Ricci = 0) e para o que o design original abaixo subestimou — o normalizador de `oderom-expr` precisou de bem mais que "colige termos semelhantes" para dar conta da soma de Kretschmann (denominador comum + divisão por potência expandida), documentado em `oderom-expr/src/normalize.rs`. Decisões D-M2.1–D-M2.3 abaixo foram tomadas como propostas (métrica só diagonal; `Expr` em crate própria; teste por igualdade estrutural pós-normalização) sem objeção explícita antes de eu prosseguir.

Mesma regra do Marco 1: isto é uma proposta, não um começo de implementação. Ao final há decisões e questões abertas — quero seu ok (e respostas às questões) antes de escrever qualquer `struct`.

## 0. O que muda de patamar em relação ao Marco 1

Marco 1 nunca avaliou um tensor em coordenada nenhuma — `Scalar` era só `Q`, e um `Monomial` era puramente combinatório (grafo de contração + cabeças declaradas). Marco 2 pede componentes de verdade: `g_{tt} = -(1 - 2M/r)`. Isso não é "mais um campo na struct" — é um objeto matemático novo, porque **a componente de um tensor numa carta é uma função das coordenadas**, um elemento do anel `C^∞(U)`. Isso força a primeira decisão grande do marco: `oderom-core::Scalar` (racional) não serve para isto, e não deve ser esticado para servir — ele continua sendo exatamente o que é (coeficiente de monômio abstrato). Preciso de um tipo novo.

## 1. `oderom-expr` (crate nova): escalares simbólicos

```
pub enum Expr {
    Rational(Scalar),              // reaproveita oderom_core::Scalar
    Var(CoordId),                  // uma coordenada da carta corrente
    Add(Vec<Expr>),
    Mul(Vec<Expr>),
    Pow(Box<Expr>, i32),           // expoente inteiro (racional fica para depois, se precisar)
    Sin(Box<Expr>),
    Cos(Box<Expr>),
}
```

Operações necessárias, nesta ordem de importância para o teste de aceitação:
- **Diferenciação simbólica** `d/dvar`, via regras padrão (soma, produto, cadeia, potência, `sin`/`cos`). Isto é mecânico — não há ambiguidade matemática aqui, ao contrário da simplificação.
- **Simplificação até forma normal de função racional**: combinar termos semelhantes, `Pow` com expoente 0/1, fatorar denominador comum. Isto é o que faz `12 * (2M/r³)²` virar `48M²/r⁶` e é o único jeito de o teste de aceitação bater com igualdade estrutural (não numérica).

Não incluo: variáveis com expoente racional, funções especiais além de `sin`/`cos` (que já bastam para Schwarzschild em coordenadas de Schwarzschild), identidades trigonométricas (`sin²+cos²=1`) — o cancelamento do ângulo no escalar de Kretschmann de Schwarzschild é puramente algébrico (cada termo carrega `sin²θ` ou `1/sin²θ` de forma que se cancela multiplicativamente, não precisa de identidade pitagórica; conferi isso antes de propor). Se eu estiver errado sobre isso quando for implementar, paro e aviso — é exatamente o tipo de coisa que quero verificar com um teste antes de assumir.

**Não é um e-grafo.** Simplificação aqui é um `simplify(&Expr) -> Expr` recursivo de baixo para cima, com regras fixas. E-grafo e saturação por igualdade continuam Marco 4, como já era.

## 2. Carta e coordenadas (`oderom-components`, crate nova)

Marco 2 não tem atlas (isso é Marco 3) — só **uma carta por variedade**, o suficiente para computar componentes:

```
pub struct Chart {
    pub id: ChartId,
    pub manifold: ManifoldId,
    pub coords: SmallVec<[CoordId; 4]>,   // uma por dimensão da variedade
}
```

## 3. Armazenamento de componentes — reaproveitando o Marco 1 inteiro

Este é o ponto que mais me anima do marco: "armazenamento só das componentes independentes" **não precisa de nenhuma lógica nova de simetria** — já construí exatamente essa máquina em `oderom-core::Bsgs` para canonicalizar índices abstratos. A ideia:

```
pub struct ComponentTensor {
    pub head: HeadId,
    pub chart: ChartId,
    // uma entrada por órbita do grupo de simetria do head sobre as
    // n^arity tuplas de índices (n = dimensão da carta) — não por tupla.
    independent: FxHashMap<SmallVec<[u8; 4]>, Expr>,
}

impl ComponentTensor {
    // Reduz `indices` ao representante canônico da sua órbita via o
    // MESMO Bsgs::strip/enumerate de oderom-core, devolve
    // sign * independent[representative], ou Expr::Rational(ZERO) se a
    // órbita força antissimetria total sobre índices repetidos.
    pub fn get(&self, indices: &[u8]) -> Expr { ... }
}
```

Ou seja: para popular Riemann de Schwarzschild eu só preciso calcular as componentes de **um representante por órbita** (para `n=4` e a simetria do Riemann, são 20 órbitas não-triviais em vez de 256 tuplas) e a struct resolve o resto via grupo. Isso é literalmente reusar `TensorHead::symmetry` e a maquinaria de `oderom-core::symmetry` já testada.

## 4. Cálculo: Christoffel, Riemann, Ricci

Fórmulas padrão, sem grau de liberdade de design real:

```
Γ^a_{bc} = ½ g^{ad} (∂_b g_{dc} + ∂_c g_{db} − ∂_d g_{bc})
R^a_{bcd} = ∂_c Γ^a_{bd} − ∂_d Γ^a_{bc} + Γ^a_{ce} Γ^e_{bd} − Γ^a_{de} Γ^e_{bc}
R_{bd} = R^a_{bad}         (Ricci)
```

A única peça de engenharia real aqui é `g^{ad}`: preciso da inversa da matriz de componentes da métrica. Para Schwarzschild ela é diagonal (inversa = recíproco componente a componente), mas não quero fincar essa suposição na API — decisão D-M2.1 abaixo.

## 5. Cache indexado por forma canônica

"Cache indexado pelo hash da forma canônica" — outro reaproveitamento direto: `oderom_canon::canonicalize` já produz uma `Monomial` canônica; um `Hash`/`Eq` sobre ela (mais a carta) é a chave natural:

```
pub struct ComponentCache {
    entries: FxHashMap<(CanonicalKey, ChartId), ComponentTensor>,
}
```

`CanonicalKey` = hash estrutural da `Monomial` canônica devolvida por `oderom-canon` (preciso adicionar `Hash`/`Eq` a `Monomial` em `oderom-core`, que hoje não tem — pequena extensão, não mudança de design).

## 6. Layout de crates proposto

```
oderom-expr/          escalares simbólicos: Expr, diferenciação, simplificação
oderom-components/    Chart, ComponentTensor, Christoffel/Riemann/Ricci, ComponentCache
  tests/
    schwarzschild.rs    -- Kretschmann = 48 M^2 / r^6
```

`oderom-components` depende de `oderom-core` (para `Bsgs`/`TensorHead`), `oderom-canon` (para a chave do cache) e `oderom-expr`. Não toca `oderom-types` nem `oderom-cli` neste marco (CLI ganhar um comando de componentes fica para depois, se você quiser).

## 7. Decisões que preciso confirmar antes de codar

**D-M2.1 — inversão de métrica: geral ou só diagonal?**
Inversão simbólica geral de matriz `n x n` (cofatores, `n=4`) é factível mas gera expressões grandes mesmo quando a métrica é diagonal (o determinante de uma matriz 4x4 simbólica genérica não é trivial de simplificar de volta a algo limpo). Para bater o teste de Schwarzschild eu só preciso do caso diagonal. Duas opções:
1. Implementar só o caso diagonal agora (checagem explícita + erro se a métrica não for diagonal na carta), documentando cofatores gerais como próximo passo.
2. Implementar cofatores gerais já, aceitando expressões maiores e um simplificador mais exigente.
Minha recomendação é (1) — é exatamente o "prefira a estrutura correta ao invés de anexar generalidade que o teste de aceitação não pede", mas quero seu aval porque é o tipo de escolha que weakens a generalidade do sistema se ficar esquecida.

**Nota (2026-07-19, discussão da UI/CLI, DESIGN-UI.md §6.0):** registrando
explicitamente o que a restrição diagonal exclui, para não ficar
esquecido -- você pediu que isto fique documentado antes de qualquer
outra coisa se apoiar em cima: coordenadas nulas (tipo
Eddington-Finkelstein, `du dv` cruzado), Kerr (`g_{t phi}` não-nulo), e
principalmente **teoria de perturbação** -- `g + h` é genericamente
não-diagonal mesmo quando `g` de fundo é diagonal, já que `h` é um tensor
simétrico qualquer, sem motivo para respeitar a estrutura diagonal do
fundo. Nada disto está sendo implementado agora; cofatores gerais
continuam sendo o próximo passo natural quando algum desses casos vier a
ser pedido de verdade.

**D-M2.2 — `Expr` fica em crate própria (`oderom-expr`) ou dentro de `oderom-components`?**
Proponho crate própria porque `Expr` é reutilizável fora de tensores (é só um CAS escalar) e mantém `oderom-components` focada em geometria, não em álgebra simbólica. Confirma?

**D-M2.3 — forma do teste de aceitação.**
"Kretschmann de Schwarzschild = `48M²/r⁶`" — vou representar isso como: declarar a métrica de Schwarzschild como `ComponentTensor` com as 4 componentes diagonais conhecidas, computar Riemann via as fórmulas acima, calcular `R_{abcd}R^{abcd}` (contração completa, subindo os 4 índices com a métrica), simplificar, e comparar contra `Expr` representando `48*M^2/r^6` por igualdade estrutural pós-simplificação (não numérica/avaliada em pontos). Confirma que é isso que você tinha em mente?

## 8. Fora de escopo (repetindo o padrão do Marco 1)

Múltiplas cartas e transições (Marco 3). SMT/domínios não-triviais (Marco 3). E-grafo/identidades multi-termo, incluindo Bianchi (Marco 4). Geodésicas/holonomia (Marco 5). Front-end gráfico. Se durante a implementação eu achar que preciso de algo dessa lista, paro e aviso, como da última vez.

---

Aguardando seu ok e respostas a D-M2.1–D-M2.3.
