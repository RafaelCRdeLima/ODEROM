# ODEROM — DESIGN-M5.md (Marco 5: IR/JIT, transporte paralelo, holonomia)

**Status: implementado.** Ver [README.md](README.md#marco-5-status). D5.1
e D5.2 confirmados como propostos (interpretador sobre IR em SSA com CSE,
sem geração de código de máquina nativo; RK4 escrito à mão, sem
dependência numérica nova). D5.3 (triângulo octante em S², tolerância
`1e-3`) também seguido como proposto -- o teste de aceitação passou já na
primeira execução completa, com os passos laterais derivados à mão
batendo com os vértices esperados dentro da tolerância.

Mesma regra dos marcos anteriores: proposta, não começo de implementação.

## 0. Isto é uma mudança de categoria maior que qualquer marco anterior

Vale nomear isso antes de mais nada: até aqui, **todo** critério de aceitação foi verificado por **igualdade estrutural exata** — Kretschmann é *literalmente* `48M²/r⁶` depois de normalizar, o pullback da métrica é *literalmente* igual depois de racionalizar, a soma de Bianchi *literalmente* extrai para zero. Nunca precisei de ponto flutuante, tolerância numérica, ou avaliação — tudo ficou no mundo simbólico exato (`Scalar` racional, `Expr` simbólico).

O critério do Marco 5 é outro animal: *"holonomia de um triângulo geodésico em S² igual à área, **dentro da tolerância**"*. Isso exige resolver duas EDOs numericamente (a equação geodésica, para traçar os três lados do triângulo, e a equação de transporte paralelo, para levar um vetor ao longo deles) e comparar um número de ponto flutuante contra outro com uma tolerância — categoria de teste que este projeto nunca teve. E o roteiro original também pede "IR em SSA... JIT" como o mecanismo — ou seja, não é só "resolva a EDO de qualquer jeito", é especificamente "compile a expressão simbólica para algo executável antes de integrar".

Por isso este marco tem **duas** perguntas de escopo, não uma, e quero as duas confirmadas antes de codar.

## 1. Pergunta 1: JIT de verdade (geração de código de máquina) ou um interpretador rápido sobre uma IR em SSA?

"JIT" no sentido literal (gerar código de máquina nativo em tempo de execução) precisaria de um backend tipo `cranelift` — uma dependência nova e pesada, categoricamente maior que qualquer coisa usada até agora (`thiserror`/`smallvec`/`rustc-hash`/`proptest`/`criterion`), e mudaria de novo a política de dependências do projeto (a mesma preocupação que motivou a pergunta sobre SMT no Marco 3, mas aqui o pacote é ainda mais pesado — um compilador JIT completo).

O motivo real de precisar de "IR em SSA + eliminação de subexpressões" é desempenho: para resolver a EDO numericamente, avalio os símbolos de Christoffel (que já são `Expr` do Marco 2) **milhares de vezes** durante a integração (uma vez por passo do integrador, por componente). Percorrer a árvore de `Expr` ingenuamente a cada avaliação é o gargalo real — mas dá pra resolver isso com um **interpretador rápido sobre uma IR linear em SSA** (converter `Expr` uma vez para uma sequência linear de operações sobre registradores temporários, com CSE eliminando subexpressões repetidas, e depois *interpretar* essa sequência repetidamente — sem nunca gerar código de máquina). Isso entrega exatamente o que "SSA + CSE" promete (e ainda é um projeto de implementação real e não-trivial: forma SSA, grafo de dependência, CSE por hash de subexpressão), sem precisar de um backend de geração de código.

**Proponho: interpretador sobre IR em SSA, sem geração de código de máquina.** "JIT" no nome dos artefatos (`oderom-jit`, se for o nome da crate) descreve a intenção arquitetural (compilar a expressão uma vez, executar muitas vezes), não uma promessa literal de código de máquina nativo. Se isso não for o que "JIT" significava no seu roteiro, preciso saber antes de escrever a IR.

## 2. Pergunta 2: qual biblioteca numérica para a EDO (se alguma)?

Integrar a equação geodésica e a de transporte paralelo numericamente é RK4 (Runge-Kutta de 4ª ordem) de manual, ou algo similar — matemática padrão, sem ambiguidade, implementável à mão sem dependência nova (mesma filosofia de Schreier-Sims/CAS/e-grafo: construído na casa). Não vejo necessidade de puxar `nalgebra` ou similar — os vetores aqui são pequenos (dimensão 2, para S²) e RK4 com um vetor de estado como array fixo é direto.

**Proponho: RK4 escrito à mão, sem dependência numérica nova.**

## 3. Estruturas propostas

### 3.1 `oderom-jit` (crate nova): IR em SSA + interpretador

```rust
pub enum Op {
    Const(f64),
    Var(usize),                      // índice na lista de variáveis de entrada (coordenadas correntes)
    Add(usize, usize),               // índices de instruções já computadas (SSA: sempre referem a algo anterior)
    Mul(usize, usize),
    Pow(usize, i32),
    Sin(usize),
    Cos(usize),
    Neg(usize),
}

pub struct Program {
    ops: Vec<Op>,                    // forma SSA: ops[i] só referencia índices < i
    output: usize,                   // qual instrução é o resultado
}

/// Compila `expr` (`oderom_expr::Expr`) para `Program`, com CSE: duas
/// subexpressões estruturalmente iguais viram a MESMA instrução, via
/// hash-consing durante a construção (mesma técnica de `oderom-egraph`,
/// bem mais simples aqui porque não precisa de union-find — só cache).
pub fn compile(expr: &Expr, vars: &[String]) -> Program;

impl Program {
    pub fn eval(&self, inputs: &[f64]) -> f64;   // o "interpretador"
}
```

### 3.2 `oderom-holonomy` (ou dentro de `oderom-components`?): geodésica e transporte paralelo

Proponho **dentro de `oderom-components`** (não uma crate nova) — é uma continuação direta do que já existe lá (`Chart`, `christoffel`), não um conceito novo o bastante para justificar outra crate.

```rust
/// Integra a equação geodésica dx^i/dt=v^i, dv^i/dt=-Gamma^i_jk v^j v^k
/// de (x0,v0) por `duration`, via RK4 com `steps` passos.
pub fn integrate_geodesic(gamma: &[Program], x0: [f64;2], v0: [f64;2], duration: f64, steps: usize) -> Vec<([f64;2],[f64;2])>;

/// Transporta `w0` paralelamente ao longo de uma trajetória já
/// calculada (dw^i/dt = -Gamma^i_jk (dx^j/dt) w^k), via RK4.
pub fn parallel_transport(gamma: &[Program], path: &[([f64;2],[f64;2])], w0: [f64;2]) -> [f64;2];
```

## 4. Teste de aceitação

Triângulo geodésico em S² (coordenadas esféricas `theta,phi`, ou estereográficas — a decidir, estereográficas evitam a singularidade nos polos que `theta,phi` tem). Três vértices formando um triângulo geodésico de área conhecida (ex.: um oitante da esfera, área `pi/2`). Percorro os três lados via `integrate_geodesic`, transporto um vetor tangente inicial via `parallel_transport` em cada lado, e comparo o ângulo entre o vetor final e o inicial contra a área do triângulo (calculada independentemente, por geometria) — dentro de uma tolerância (proponho `1e-3`, a confirmar, dependendo de quantos passos de RK4 forem baratos o bastante para os testes automatizados não ficarem lentos).

## 5. Fora de escopo

Geração de código de máquina de verdade (pergunta 1). Qualquer EDO além de geodésica/transporte paralelo. Adaptação de passo no RK4 (passo fixo já deve bastar para a tolerância do teste). Integração com `oderom-cli`.

## 6. Questões abertas

**D5.1** — confirma a pergunta 1: interpretador sobre IR em SSA, não geração de código de máquina nativo, mesmo o nome do marco dizendo "JIT"?

**D5.2** — confirma a pergunta 2: RK4 de passo fixo, escrito à mão, sem dependência numérica nova?

**D5.3** — confirma o triângulo/tolerância propostos na seção 4, ou tem uma preferência específica de figura/tolerância?

---

Aguardando seu ok, principalmente em D5.1 (é a que mais muda o formato do trabalho).
