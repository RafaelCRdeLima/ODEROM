# ODEROM — DESIGN-RATIONAL-FORM.md (forma normal racional para Scalar)

Mesma regra de sempre: proposta, não começo de implementação. Ver
DESIGN-M2.md para a restrição já registrada (só métrica diagonal) e o
guarda-corpo (timeout + `--max-nodes` + `--max-denominator-degree`, os
três já implementados em `oderom-cli`) — este documento é só a correção
de verdade.

**D-RF.1 aprovado** (restrição a MDC univariado por variável de polo,
seção 2.3). Três exigências recebidas antes de eu tocar em código,
incorporadas abaixo: o anel de coeficientes também precisa ser canônico
(2.1/2.3), `sin`/`cos` entram como geradores internados do anel, não
subárvore comparada por igualdade estrutural (2.1/2.4), e todo caminho
que não sabe reduzir tem que devolver resultado correto e não-reduzido,
nunca errado, com os casos documentados explicitamente (2.3/2.6).

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

Ou seja: **o estouro é de tempo de computação dentro do `normalize()`,
localizado numa árvore que nunca fica grande** — correção sua ao meu
relato anterior: isso não contradiz expression swell, localiza onde ele
está (dentro da chamada, não no tamanho de entrada). Uma segunda medição
confirmou o mecanismo: `oderom_expr::denominator_degree` (nova função,
`rationalize()` seguido da definição recursiva padrão de grau
polinomial) cresce exatamente onde a lentidão começa — 0 em 16 termos
somados, **111 em 32** — enquanto a contagem de nós fica achatada. Por
isso o guarda-corpo agora tem três limites, não dois:
tempo de parede (`--timeout`), nós (`--max-nodes`, não pega este caso
sozinho) e grau de denominador (`--max-denominator-degree`, pega). Uma
ressalva medida, não assumida: `denominator_degree` custa
aproximadamente o mesmo que `normalize()` (usa o mesmo `rationalize()`
por baixo), então não é uma checagem barata — no CLI ela só roda na
mesma cadência do relatório de progresso, não a cada termo. Isso importa
para a correção também: qualquer correção precisa atacar *custo por
chamada de `normalize()`*, não só *tamanho final da expressão*.

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

### 2.1 `Poly`: polinômio canônico sobre "átomos" -- **um só nível, tudo achatado**

Correção em relação à primeira versão deste documento (sua exigência 1):
não existe "coeficiente" que fica de fora do canônico. `r`, `M`, `Q`, e
`sin(theta)` (uma vez internado, ver abaixo) são todos geradores do
*mesmo* anel, no *mesmo* monômio -- não há um nível "de fora" (variável
de polo) e outro "de dentro" (coeficiente, tratado como árvore `Expr`
opaca). Se coeficientes ficassem como árvores `Expr` cruas,
`M²Q²*A + M²Q²*B` não coletaria e o estouro reapareceria um nível
abaixo, em `M`/`Q`, exatamente como você apontou.

```rust
/// Um gerador do anel: nome de variável, OU um sin/cos já internado
/// (ver AtomTable, 2.4) -- os dois são um índice pequeno e Copy, nunca
/// uma sub-árvore comparada por igualdade estrutural a cada operação.
enum Atom { Var(u32), Trig(TrigId) }  // TrigId: índice em AtomTable

/// Um monômio: coeficiente racional vezes geradores com expoente
/// inteiro >= 0, em ordem canônica, sem gerador repetido, sem expoente
/// zero. TODOS os geradores relevantes (r, M, Q, sin(theta), ...) vivem
/// na MESMA lista -- não existe coeficiente "de fora".
struct Term { coeff: Scalar, generators: Vec<(Atom, u32)> }

/// Soma de termos com assinatura de geradores distinta -- polinômio
/// multivariado de verdade, canônico por construção.
struct Poly(Vec<Term>);
```

`Poly` ganha `add`, `mul`, `pow(u32)` — operações de manual, sem
mistério (a mesma ideia do `simplify_add`/`simplify_mul` atuais, só que
sobre uma representação que já é canônica por construção em vez de
precisar de reescrita ponto-fixo para chegar lá). Toda aritmética --
inclusive a que o algoritmo de MDC (2.3) faz internamente sobre
"coeficiente de r^k" -- usa `Poly::add`/`Poly::mul`, nunca `Expr` cru com
`normalize()` por trás. Essa é a regra que fecha a exigência 1: nenhum
ponto do algoritmo, do topo à divisão longa por dentro do MDC, sai do
tipo canônico para fazer uma conta.

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

Proponho: identificar, por denominador, qual gerador é a "variável de
polo" (o único em que o denominador é não-trivial). Com isso, MDC vira o
algoritmo de Euclides padrão para polinômios univariados **cujos
coeficientes são, eles mesmos, elementos de `Poly`** nos geradores
restantes (`M`, `Q`, `sin(theta)`, ...) -- não `Scalar`, não `Expr` cru:

```text
gcd(a, b):                    -- a, b: Poly-em-r, coeficientes em Poly
    enquanto b != 0: a, b = b, a rem b   (resto da divisão longa,
                                          coeficientes via Poly::add/mul/div)
    devolve a, normalizado (coeficiente líder = 1 via Poly)
```

Divisão longa de polinômios univariados é aritmética de manual, sem
dependência nova — mesma filosofia de Schreier-Sims/CAS/e-grafo. O
"corpo" de coeficientes (`M`, `Q` misturados) precisa suportar divisão
formal (multiplicar pelo inverso) para a divisão longa sempre funcionar
-- seguro, porque o resultado final já é uma função racional mesmo, não
um polinômio -- mas essa divisão formal ainda acontece **sobre `Poly`**,
nunca voltando para `Expr`/`normalize()` no meio do caminho (exigência
1: nenhum nível do algoritmo escapa do tipo canônico).

**Quando não há uma única variável de polo** (denominador genuinamente
bivariado, tipo o `Σ = r² + a²cos²(theta)` de Kerr): **não é erro**
(correção à primeira versão deste documento, sua exigência 3) -- o MDC
simplesmente não roda, e a `RationalFunction` fica como está, correta e
não-reduzida (`num`/`den` ambos em `Poly` canônico, só sem cancelamento
entre eles). O valor computado nunca fica errado por causa disso, só
maior do que precisaria. Registrado aqui como o próximo limite real
(mesmo espírito do registro de D-M2.1 sobre métrica não-diagonal), não
como algo que aborta a conta.

### 2.4 `sin`/`cos`: geradores internados, não subárvore comparada por igualdade

Ajuste à primeira versão (sua exigência 2): `Sin`/`Cos` não podem viver
no monômio como um `Box<Expr>` comparado estruturalmente a cada
operação de `Poly` -- além de caro, isso é "subárvore", não "variável".
Proponho uma `AtomTable` (uma por computação, ou por chamada de
`normalize()`): a primeira vez que um `Sin(arg)`/`Cos(arg)` com
`arg` já em forma canônica aparece, ganha um `TrigId` (índice pequeno,
`Copy`) memoizado por igualdade estrutural do argumento *uma única vez*
-- toda ocorrência seguinte do mesmo `sin`/`cos` reaproveita o mesmo
`TrigId`, e daí pra frente `Atom::Trig(id)` se comporta exatamente como
`Atom::Var` em toda a aritmética de `Poly` (comparação/hash O(1), não
O(tamanho da subárvore)). `sin²(theta)*A + sin²(theta)*B` coleta porque
os dois têm o mesmo `TrigId` na mesma posição do monômio -- exatamente o
caso que você apontou que a primeira versão não resolvia. Continua sem
identidade trigonométrica nenhuma (`sin²+cos²=1` não entra em jogo, hoje
ou nesta proposta) -- isso é um problema diferente, deliberadamente fora
de escopo.

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

### 2.6 Contrato de correção (exigência 3): reduzido ou não, nunca errado

Regra única, vale para todo o algoritmo: **qualquer caminho que decide
não tentar (ou não consegue terminar) uma redução devolve
`RationalFunction{num, den}` correta e não-reduzida -- nunca um valor
errado, nunca um erro que aborta a conta.** Os casos conhecidos onde
isso se aplica, documentados aqui para não ficarem implícitos no código:

1. **Denominador sem variável de polo única** (2.3) -- o caso Kerr-like.
   MDC não roda; `num`/`den` ficam como estão.
2. **MDC de coeficiente que não termina em tempo/passos razoáveis** --
   se o algoritmo de Euclides sobre os coeficientes (eles mesmos `Poly`,
   exigência 1) precisar de um limite de passos por segurança, o
   resultado ao atingir o limite é o par não-reduzido no ponto em que
   parou, não uma tentativa de "adivinhar" o resto.
3. **Conversão `Expr -> RationalFunction` de algo fora do fragmento
   coberto** (nenhum caso conhecido hoje, já que `Expr` só tem
   `Rational`/`Var`/`Add`/`Mul`/`Pow`/`Sin`/`Cos` -- registrado por
   completude, caso um variante novo apareça no futuro).

O teste diferencial (4) é o que verifica esse contrato na prática: se o
`normalize()` antigo terminava com um valor `V`, o novo tem que
terminar com o mesmo `V` -- reduzido ou não, o *valor* nunca muda.

## 3. Fora de escopo

MDC multivariado geral (deixa Kerr-like sem reduzir, não sem funcionar
-- ver 2.3/2.6). Identidades trigonométricas. Qualquer mudança na API
pública de `Expr` ou de `oderom-components::curvature` além de
`kretschmann`'s acumulação interna. Dependência nova (tudo aqui é
aritmética de manual, mesmo espírito do resto do projeto).

## 4. Plano de implementação, se aprovado (ordem de trabalho)

1. `Poly` (soma/produto/potência) + testes unitários, incluindo `f(r)`
   de Reissner-Nordström explicitamente como caso de teste, e um caso
   com `sin`/`cos` internados (exigência 2) confirmando que
   `sin²(theta)*A + sin²(theta)*B` coleta.
2. MDC univariado sobre coeficientes-`Poly` (Euclides) + testes,
   incluindo o caso que hoje trava (`(1-2M/r+Q²/r²)^4` como denominador
   de um numerador que deveria colapsar) e um caso sem variável de polo
   única confirmando o fallback não-reduzido do 2.6 (nunca erro).
3. `RationalFunction` (`add`/`mul`/`pow`, reduz quando sabe, nunca
   errado quando não sabe).
4. Conversão `Expr <-> RationalFunction` nas duas direções.
5. **Teste diferencial, o cinto de segurança principal** (seu pedido):
   gera `Expr` aleatórias pequenas (`proptest`, já dependência aprovada
   -- mesmo mecanismo do teste de propriedade do Marco 1), roda o
   `normalize()` *antigo* (mantido temporariamente sob outro nome,
   ex. `normalize_v1`, só para este teste, removido depois que a
   confiança estiver estabelecida) e o novo lado a lado, com um limite
   de iterações/tempo no antigo -- toda entrada em que o antigo termina,
   os dois têm que concordar. Isso corre *antes* do passo 6.
6. `normalize()` trocado por dentro para rotear por aqui -- suíte de
   testes existente (Kretschmann Schwarzschild, Bianchi, holonomia, S²,
   de Sitter) roda **sem alteração nenhuma**, esse é o critério de "não
   quebrei nada" que você pediu.
7. `curvature::kretschmann` trocado para reduzir incrementalmente.
8. Reissner-Nordström vira fixture de aceitação de verdade, verificada
   contra a forma fechada que você deu:
   `48M²/r⁶ - 96MQ²/r⁷ + 56Q⁴/r⁸`.
9. **Sonda de escala de verdade**: um `f(r)` de quatro termos (proponho
   `1 - 2M/r + Q²/r² - L²/r³`, sem significado físico -- é só para
   testar se a curva domou ou só empurrou o teto), rodado em
   `diagnostic_rn.rs`-style contra o `normalize` antigo *e* o novo,
   reportando os dois tempos lado a lado -- não só o novo.

## 5. Perguntas restantes

**D-RF.1 já aprovado** (restrição a MDC univariado por variável de
polo).

**D-RF.2** — as exigências 1-3 desta rodada mudam a resposta original
("sin/cos átomos opacos") para "geradores internados, sem identidade
trigonométrica" (2.4). Confirma essa versão revisada?

**D-RF.3** — o guarda-corpo (timeout + `--max-nodes` +
`--max-denominator-degree`, os três já implementados em `oderom-cli`)
fica valendo em produção enquanto isto é construído -- presumo que sim.

---

Aguardando seu ok antes de tocar em `oderom-expr`.
