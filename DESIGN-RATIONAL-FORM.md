# ODEROM — DESIGN-RATIONAL-FORM.md (forma normal racional para Scalar)

Mesma regra de sempre: proposta, não começo de implementação. Ver
DESIGN-M2.md para a restrição já registrada (só métrica diagonal) e o
guarda-corpo (item 2 do seu pedido) já implementado independentemente
disto em `oderom-cli` — este documento é só o item 3.

## 0. O diagnóstico medido (não a hipótese)

`oderom-components/tests/diagnostic_rn.rs` (rode com `--ignored
--nocapture`) mediu, para Reissner-Nordström (`f(r) = 1 - 2M/r + Q²/r²`,
três termos):

- Todo estágio de `Grid` (Christoffel, Riemann misto, abaixamento de
  índice, os quatro `raise_index` do Kretschmann) é barato: 100-140ms
  cada, 1000-1250 nós no total sobre 256 componentes.
- A soma bruta de 256 termos, antes de qualquer `normalize`, tem **2889
  nós** — pequena.
- `normalize()` de um único termo `R_cov·R_contra` isolado (91→96 nós)
  já leva **924ms** — sozinho, sem soma nenhuma.
- Dobrando o número de termos somados: 16 termos ainda é microssegundos
  (a maioria zero); **32 termos → 1,8s; 64 → 3,6s; 128 → não terminou em
  20s.**

Ou seja: **o estouro é de tempo de computação sobre uma árvore pequena,
não de tamanho de árvore.** Isso já mudou o desenho do guarda-corpo
(item 2) de "só contagem de nós" para "tempo de parede + contagem de
nós" — e importa aqui também: qualquer correção precisa atacar *custo
por chamada de `normalize()`*, não só *tamanho final da expressão*.

## 1. Por que o mecanismo atual não generaliza (com base no código, não só no sintoma)

Três coisas em `oderom-expr/src/normalize.rs` e `rationalize.rs`,
todas já documentadas no próprio código como específicas do caso de
2 termos:

1. `combine_over_common_denominators` só combina termos que compartilham
   **uma única** base de denominador (`Expr::Add` idêntica
   estruturalmente). O próprio comentário diz: *"Two or more distinct
   denominator sums in the same sum is left uncombined (out of scope:
   nothing in the Christoffel/Riemann/Kretschmann pipeline this exists
   for produces that case)"* — Reissner-Nordström produz exatamente
   esse caso agora.

2. `divide_by_expanded_power` (usada para reconhecer que um numerador já
   expandido é `Q * base^n` e colapsar de volta) escolhe expoentes
   candidatos (`-min_k` e `contagem_de_termos - 1`) que o próprio
   comentário chama de *"the only other exponent an expanded **2-term**
   base^p could possibly match"* — uma heurística combinatória amarrada
   a denominador de 2 termos. Para um denominador de 3 termos a
   contagem de termos da expansão segue outra combinatória
   (multinomial, não binomial), então a heurística tende a simplesmente
   não achar o expoente certo, deixando a expressão sem colapsar.

3. `rationalize()` já carrega numerador/denominador explícitos pela
   recursão (`a/b + c/d = (ad+bc)/(bd)`) — a ideia certa — **mas nunca
   reduz por MDC**: `den = normalize(&(den * td))` multiplica
   denominadores sem cancelar nada em comum. Ao longo de uma soma de
   256 termos, cada um contribuindo o mesmo `f(r)` (ou uma potência
   dele) como denominador, o grau do denominador acumulado cresce sem
   limite em vez de ficar em `f(r)^4`. Isso bate exatamente com o que
   você viu na saída do `riemann` — "nove termos, cada um com
   denominador próprio" — e com a curva medida acima.

O item 3 é o mais importante: a arquitetura certa (numerador/denominador
explícitos) **já existe**. Falta exatamente uma peça: redução por MDC
de verdade, no lugar de esperar que o pattern-matching do `normalize()`
tropece na simplificação.

## 2. Proposta: completar `rationalize` com uma forma canônica + MDC

### 2.1 `Poly`: polinômio canônico sobre "átomos"

```rust
/// Uma folha que normalize() não decompõe mais algebricamente --
/// inclui sin/cos de um argumento já canônico, não só Var: ver 2.4.
enum Atom { Var(String), Sin(Box<Expr>), Cos(Box<Expr>) }

/// Um monômio: coeficiente vezes átomos com expoente inteiro >= 0,
/// em ordem canônica, sem átomo repetido, sem expoente zero.
struct Term { coeff: Scalar, atoms: Vec<(Atom, u32)> }

/// Soma de termos com assinatura de átomos distinta -- polinômio
/// multivariado de verdade, não um Expr::Add cru.
struct Poly(Vec<Term>);
```

`Poly` ganha `add`, `mul`, `pow(u32)` — operações de manual, sem
mistério (a mesma ideia do `simplify_add`/`simplify_mul` atuais, só que
sobre uma representação que já é canônica por construção em vez de
precisar de reescrita ponto-fixo para chegar lá).

### 2.2 `RationalFunction`: numerador/denominador, sempre reduzidos

```rust
struct RationalFunction { num: Poly, den: Poly }  // gcd(num, den) = 1
```

Toda operação (`add`, `mul`, `pow`) que combina duas
`RationalFunction`s termina chamando `reduce`, que divide `num` e `den`
pelo MDC deles antes de devolver o resultado — esta é a peça que falta
em `rationalize()` hoje. `a/b + c/d` continua sendo `(ad+bc)/(bd)`, mas
agora seguido de MDC, então somar 256 termos com o mesmo `f(r)` no
denominador (ou potências dele) mantém o denominador em `f(r)^k` em vez
de crescer para `f(r)^256`.

### 2.3 O algoritmo de MDC: univariado por "variável de polo", não multivariado geral

Esta é a decisão de escopo real, e é a que mais preciso da sua
confirmação. MDC multivariado geral (bases de Gröbner ou variantes) é
um projeto por si só — pesado, e este projeto não parece precisar dele
ainda. Toda métrica que este projeto já tratou (Schwarzschild, RN) tem
denominador que é polinômio numa **única** variável de fato problemática
(`r`) — `M`, `Q` entram só como coeficientes, nunca dividindo nada.

Proponho: identificar, por denominador, qual átomo é a "variável de
polo" (o único em que o denominador é não-trivial; se o denominador só
tem UM átomo com expoente não-nulo, é ele; erro explícito — não
resultado errado — se houver mais de um). Com isso, MDC vira o algoritmo
de Euclides padrão para polinômios univariados sobre um corpo:

```text
gcd(a, b):
    enquanto b != 0: a, b = b, a mod b   (resto da divisão longa)
    devolve a, normalizado (coeficiente líder = 1)
```

Divisão longa de polinômios univariados é aritmética de manual, sem
dependência nova — mesma filosofia de Schreier-Sims/CAS/e-grafo. O
truque de que **precisa**: o "corpo" aqui não é só `Q` (racionais) --
os coeficientes de cada potência de `r` são eles mesmos expressões em
`M`, `Q` (ex.: `48*M²`). Para a divisão longa funcionar sempre (não só
quando o coeficiente líder por acaso divide exatamente), esses
coeficientes precisam ser tratados como elementos de um corpo -- ou
seja, "divisão de coeficiente" é sempre permitida (multiplicar pelo
inverso formal), o que é seguro porque o resultado final já é uma
função racional mesmo, não um polinômio.

**O que isso NÃO resolve**: um denominador genuinamente bivariado, tipo
o `Σ = r² + a²cos²(theta)` de Kerr, não tem uma única variável de polo
-- precisaria de MDC multivariado de verdade. Fica de fora desta
proposta, registrado como o próximo limite (mesmo espírito do registro
de D-M2.1 sobre métrica não-diagonal).

### 2.4 `sin`/`cos`: átomos opacos, não uma capacidade nova

`Sin`/`Cos` já são tratados como bases opacas por `simplify_mul` hoje
(agrupadas por igualdade estrutural do argumento, nunca expandidas,
nunca relacionadas por `sin²+cos²=1`). A proposta não muda esse
comportamento -- só formaliza: o alfabeto de átomos de `Poly` inclui
`Sin(argumento_já_canônico)`/`Cos(argumento_já_canônico)` além de
`Var`, com a mesma álgebra de expoente inteiro que qualquer variável.
Nenhuma identidade trigonométrica entra em jogo, hoje ou nesta proposta.

### 2.5 Onde isso entra: por dentro de `normalize()`, `Expr` não muda por fora

Ponto importante para avaliar o risco: `normalize(e: &Expr) -> Expr`
mantém exatamente a assinatura e o contrato de hoje. A mudança é só
*como* o resultado é calculado internamente: converte `Expr` para
`RationalFunction` (`Poly`/`Poly`), faz a álgebra lá (com MDC a cada
combinação), converte de volta para `Expr` no final. Todo `assert_eq!`
contra `Expr` normalizado nos testes existentes continua funcionando
sem mudança -- é o mesmo tipo de garantia que o Marco 5 teve ao trocar
"JIT" por interpretador de IR: interface pública intocada, motor por
dentro diferente.

"Aplicada durante a contração, não no fim": para `christoffel`,
`riemann_mixed`, `ricci_tensor`, `lower_first_index`, `raise_index` --
que já chamam `normalize()` por componente, imediatamente após montar a
soma daquele componente -- isso já é "durante a contração" hoje, e
continua sendo, de graça, sem tocar em `oderom-components`. **Uma
função precisa mudar de verdade**: `kretschmann()`, que hoje acumula os
256 termos crus (`sum = sum + term`, 256 vezes) e só chama `normalize`
uma vez no final. Proponho trocar para reduzir incrementalmente (somar
e reduzir termo a termo, mantendo o conjunto de trabalho pequeno o
tempo todo) -- pequena mudança, local a essa função, na mesma direção
do que o guarda-corpo do CLI já faz hoje (que reimplementa esse loop no
lado do `oderom-cli` só para poder medir/abortar; com o núcleo
corrigido, dá pra voltar a usar `curvature::kretschmann` direto).

## 3. Fora de escopo

MDC multivariado geral (deixa Kerr-like fora, ver 2.3). Identidades
trigonométricas. Qualquer mudança na API pública de `Expr` ou de
`oderom-components::curvature` além de `kretschmann`'s acumulação
interna. Dependência nova (tudo aqui é aritmética de manual, mesmo
espírito do resto do projeto).

## 4. Plano de implementação, se aprovado (ordem de trabalho)

1. `Poly` (soma/produto/potência) + testes unitários, incluindo `f(r)`
   de Reissner-Nordström explicitamente como caso de teste.
2. MDC univariado (Euclides) + testes, incluindo o caso que hoje trava
   (`(1-2M/r+Q²/r²)^4` como denominador de um numerador que deveria
   colapsar).
3. `RationalFunction` (`add`/`mul`/`pow`, sempre reduzido).
4. Conversão `Expr <-> RationalFunction` nas duas direções.
5. `normalize()` trocado por dentro para rotear por aqui -- suíte de
   testes existente roda sem alteração, é o critério de "não quebrei
   nada".
6. `curvature::kretschmann` trocado para reduzir incrementalmente.
7. Rodar `diagnostic_rn.rs` de novo -- número novo, não a confirmação de
   que "deve" ter melhorado.
8. Reissner-Nordström vira fixture de aceitação de verdade (seu
   registro do item 1 da rodada anterior).

## 5. Perguntas antes de eu implementar

**D-RF.1** — confirma a restrição de MDC univariado por variável de
polo (não multivariado geral)? Resolve Reissner-Nordström; não
resolveria de graça um denominador genuinamente bivariado como o `Σ` de
Kerr.

**D-RF.2** — confirma que `sin`/`cos` continuam átomos opacos, sem
identidade trigonométrica nenhuma -- igual hoje?

**D-RF.3** — o guarda-corpo (timeout + limite de nós, já implementado
em `oderom-cli`) fica valendo em produção enquanto isto é construído --
presumo que sim, já que você pediu que continuasse valendo depois do
desempenho melhorar.

---

Aguardando seu ok antes de tocar em `oderom-expr`.
