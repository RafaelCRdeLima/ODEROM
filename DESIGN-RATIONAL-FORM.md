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

## 5. Status

**D-RF.1, D-RF.2, D-RF.3 aprovados.** Quatro decisões adicionais, dadas
antes de codar e registradas aqui em vez de deixadas para emergir do
código:

**D-RF.4 — ordenação nunca depende de `AtomId`.** `AtomId` (o índice
interno de `AtomTable`) serve só para igualdade/hash O(1) durante o
agrupamento de monômios (`FxHashMap<Vec<(AtomId,u32)>, Scalar>`, não
`BTreeMap` -- agrupar não precisa de ordem total, só de `Eq`+`Hash`).
Qualquer ordenação com significado semântico (a forma canônica de
`Poly` para `Eq`/exibição, a conversão final de volta para `Expr::Add`
ordenado) usa uma chave derivada do *conteúdo* do átomo -- `Var(nome)`
por nome, `Sin(arg)`/`Cos(arg)` por `(discriminante, Ord de arg)`, sendo
`arg` um `Expr` já canônico que já tem `Ord` -- nunca a ordem de
inserção na tabela. Concretamente: `AtomTable` guarda `Vec<AtomKey>`
(conteúdo) + `FxHashMap<AtomKey, AtomId>` (interning); qualquer função
que precise ordenar recebe `&AtomTable` e compara via
`table.key(id)`, nunca via `id` cru.

**D-RF.5 — uma `AtomTable` por chamada de topo de `normalize()`, nunca
`static`/global.** Criada no início de cada conversão `Expr ->
RationalFunction`, passada por referência through a álgebra, descartada
no final junto com o resultado convertido de volta para `Expr`. Cada
thread (cada subcomando do CLI já roda na sua própria, ver commands.rs)
tem a sua, sem sincronização nenhuma porque não há nada compartilhado.

**D-RF.6 — `Poly`/`RationalFunction` só existem depois de toda
derivação.** Decisão explícita (não a outra opção, de átomos carregarem
regra de derivada): `diff()` continua operando exclusivamente sobre
`Expr` -- assinatura intocada, chain rule intocada, nunca vê `Poly`. A
sequência em `curvature::christoffel` (`diff(&g.get(...), coord)`
produzindo `Expr` cru, que só depois entra em `normalize()`) já garante
isso hoje; a garantia fica estrutural, não só convencional, porque
`Poly`/`RationalFunction`/`AtomTable` não são exportados publicamente
de `oderom-expr` -- só `normalize()` (via `rationalize`/`normalize`
internos) os constrói e consome, nunca aparecem numa assinatura que
`diff()` ou qualquer código fora do módulo possa alcançar.

**D-RF.7 — identidade trigonométrica, não opcional.** Correção à
proposta original: `sin`/`cos` como geradores livres torna `Poly`
canônico para o *anel livre* `Q[..,sin,cos,..]`, não para o corpo de
funções de verdade (`sin²+cos²=1` é uma relação real) -- isso degrada
exatamente o "N componentes identicamente nulos" que os testes de
aceitação já reportam, fazendo um componente que só é zero *por causa*
da identidade passar a ser relatado como não-zero. Correção: `sin` é
gerador primário (expoente inteiro livre); `cos` do mesmo argumento é
sempre mantido em grau <= 1 -- toda vez que a multiplicação de `Poly`
formaria `cos(arg)^(k>=2)`, aplica-se `cos^(2k) -> (1-sin(arg)²)^k` e
`cos^(2k+1) -> cos(arg)*(1-sin(arg)²)^k` antes de prosseguir (expandido
via `Poly::pow` sobre o gerador `Sin(arg)` da mesma tabela). Forma
canônica padrão do anel trigonométrico `Q[sin,cos]/(sin²+cos²-1)`. Teste
obrigatório (seção 4): um componente identicamente zero só via
`sin²+cos²=1` continua sendo detectado como zero.

**Nota sobre o guarda-corpo**: `denominator_degree` não precisa de
otimização -- uma vez que `Poly` existir, grau vira leitura de campo
(`den`'s grau total, já mantido pela própria estrutura). O custo atual é
temporário, não vale investir nele agora.

---

Implementação autorizada. Próximo passo: código.

## 6. Status final: switchover concluído

Reissner-Nordström completa e bate com a forma fechada exata
(`48M²/r⁶ - 96MQ²/r⁷ + 56Q⁴/r⁸`), via dois desenvolvimentos além do que
a seção 2.3 originalmente propôs (Euclides univariado simples não foi
suficiente):

- **PRS subresultante** (Collins/Brown) no lugar de pseudo-divisão crua
  na variável de polo, para manter o crescimento de coeficiente/termo
  polinomial em vez de exponencial ao longo da sequência de restos.
- **MDC multivariado recursivo** (`content()`/`primitive_part()` de
  verdade, não só monomial): a cada nível, a variável de maior
  prioridade (ordem total fixa D-RF.4) vira a variável de polo, os
  coeficientes vivem numa variável a menos, recursão desce até MDC de
  inteiro puro como caso base. Isso generaliza a restrição da seção 2.3
  ("MDC univariado por variável de polo, não multivariado geral") — o
  MDC agora *é* multivariado, mas continua construído em camadas
  univariadas recursivas, não busca de base de Gröbner.

`normalize()` foi trocado em produção para este motor
(`oderom-expr/src/normalize.rs`). `legacy_v1` permanece no código,
acessível via `ODEROM_ENGINE=legacy`, como escotilha de emergência e
oráculo diferencial permanente (`v1_and_v2_agree`, agora comparação por
valor via avaliação numérica, já que os dois motores provadamente têm
formas canônicas diferentes — ambas válidas — para a mesma função
racional).

### Sonda de 4 termos: limite conhecido, e a correção do que realmente o causa

Uma métrica sintética com `f(r) = 1 - 2M/r + Q²/r² - L²/r³` (3
parâmetros livres: `M`, `Q`, `L` — `r` é a coordenada, não um
parâmetro, contagem corrigida abaixo) foi usada como sonda de escala
durante o desenvolvimento e não foi perseguida até terminar. A nota
original, escrita a partir desse único caso, generalizou errado:

> ~~Métricas com 3 ou mais parâmetros livres podem não terminar.~~

**Falsificado por uso real** (achado incidental durante a Etapa 2 do
REPL, não hipotético): uma métrica com só 2 parâmetros — a mesma
contagem de Reissner-Nordström — travou por 60+s exatamente no estágio
em que RN termina em menos de 1s. A diferença não é contagem de
parâmetros; é `g_tt`/`g_rr` serem recíprocos (`g_tt·g_rr = -1`, a forma
`-f dt² + f⁻¹ dr²` de livro-texto) ou não. Quatro pontos medidos, não
estimados (fixture permanente: `oderom-session/tests/cancellation.rs`):

| `g_tt`/`g_rr` | parâmetros livres | kretschmann |
|---|---|---|
| recíprocos (RN: `f(r)=1-2M/r+Q²/r²`) | 2 (`M`,`Q`) | ~1s |
| recíprocos (`f(r)=1-2M/r+Q²/r²-L²/r³`) | 3 (`M`,`Q`,`L`) | ainda rodando após 30s |
| independentes, 1 parâmetro cada (`1-2M/r` vs `1-M/r`) | 1 (`M`) | ~1.2s |
| independentes, 2 parâmetros total (`1-2M/r+1/r²` vs `1-2M/r+Q²/r²`) | 2 (`M`,`Q`) | ainda rodando após 60s |

Leitura: a forma recíproca dá ao `poly_gcd` um fator grande e
garantido-compartilhado "de graça" em quase todo termo intermediário —
por isso os 2 parâmetros de RN são baratos mas os mesmos 2 parâmetros
sem reciprocidade não são. Reciprocidade compra aproximadamente o
espaço de um parâmetro a mais, não imunidade: passado esse espaço (em
qualquer uma das duas direções), o MDC multivariado recursivo continua
denso, e o próximo degrau algorítmico continua sendo MDC
modular/esparso (Zippel), onde bibliotecas externas (FLINT, Symbolica)
são a resposta conhecida.

O guarda-corpo continua sendo a defesa para qualquer caso que ultrapasse
o teto, em qualquer direção — nunca trava sem saída, nunca devolve
resultado errado: `--timeout`/`--max-nodes`/`--max-denominator-degree`
do CLI para comandos avulsos, e (DESIGN-UI-SESSION.md) `:timeout` do
REPL mais Ctrl+C, agora apoiados em checkpoints *dentro* de
`normalize()`/`poly_gcd`/o laço do PRS subresultante
(`oderom-expr/src/cancel.rs`), não só entre componentes — um único
componente que nunca retorna é exatamente o caso que expôs esta nota (o
checkpoint só-entre-componentes nunca era alcançado). Gatilho para
reabrir a decisão de biblioteca externa: um problema real nesse regime
bloqueando uso de verdade, não o teto por si só.

### D-RF.7 dentro do MDC quebra a exatidão do PRS subresultante — corrigido

Achado por fuzzing por propriedade (`v1_and_v2_agree`), não hipotético:
reescrever `cos²→1-sin²` (D-RF.7) *durante* a própria computação do MDC
recursivo (não só na conversão final `Poly`→`Expr`) muda o anel de
coeficientes no meio do algoritmo, quebrando a contabilidade de grau de
que o PRS subresultante depende para garantir divisão exata por β_i.
Verificado por fora do Rust (Python/sympy): a divisão É exata no anel
livre `Q[cos(t)][x]`; deixa de ser exata (resto não-zero, e não múltiplo
do ideal `(cos²+sin²-1)`) assim que a reescrita entra em cena no meio do
cálculo. Caso mínimo: `cos(0) + (r+x)^-3 + (r-1)` — nenhum cos² explícito
na entrada, o quadrado surge internamente ao elevar ao quadrado um
coeficiente líder durante o cálculo de β.

Corrigido, não documentado como limite: `TrigRewriteSuppressor`
(`oderom-expr/src/poly.rs`) suspende a reescrita para toda a descida
recursiva de `poly_gcd` (uma flag thread-local, guarda RAII que restaura
o valor anterior mesmo sob panic) e `Poly::normalize_trig` a reaplica
uma única vez, no resultado final, antes de devolver ao chamador. `a`/`b`
de entrada já chegam em forma normal (construídos por aritmética normal,
não suspensa, em `expr_to_rational`) — só o que o próprio MDC introduz
internamente fica temporariamente sem reduzir.

**Aviso registrado, não implementado**: `TrigRewriteSuppressor` sendo
thread-local está correto hoje porque cada `normalize()` roda do início
ao fim numa única thread (D-RF.5). Se o cálculo de componentes
(`christoffel`/`riemann_mixed` por componente, por exemplo) for
paralelizado no futuro, uma flag thread-local deixa de bastar sozinha —
o modo de falha é reescrita ativa dentro de um MDC rodando numa thread
que nunca teve a flag ligada (thread-local não herda por spawn, e um
executor que migra a mesma computação entre threads de trabalho pode
retomar uma região suspensa numa thread "limpa"), produzindo resultado
errado sem panic nenhum — silencioso, o pior tipo. Antes de paralelizar
qualquer coisa que chame `normalize()`/`poly_gcd`, essa suspensão precisa
virar estado passado explicitamente (parâmetro, ou carregado no
contexto/task que o executor paralelo já propaga), não uma thread-local.
Ver o comentário no próprio tipo (`poly.rs`) para o detalhe completo.

## 7. Dois limites conhecidos, cada um com teste de ouro adormecido (Rodada Metrica Nao-Diagonal)

**Ponto único de referência para quem for atacar o normalizador a
seguir** — não reconstrua isto a partir de `DESIGN-M2.md`, dos
comentários em `poly.rs`, ou desta conversa: o que segue é
autossuficiente. A extensão de núcleo que generalizou
`metric_inverse` para métricas não-diagonais (bloco 2x2 do Kerr
invertido em milissegundos, sem regressão nas diagonais — ver
`DESIGN-M2.md`, seção "D-M2.1 revisitada") expôs dois limites
pré-existentes deste motor, nenhum dos dois causado pela inversão em
si, ambos fora do escopo daquela rodada. Nenhum dos dois foi corrigido.
Kerr e Gödel continuam fora da galeria (`oderom-cli/src/gallery.rs`)
por causa deles.

### 7.1 MDC multivariado sem variável de polo única — bloqueia Kerr

**O que trava**: `christoffel`/`riemann_mixed` a partir da métrica de
Kerr (Boyer-Lindquist). Medido (release, `oderom-components/tests/diagnostic_kerr.rs`):
a inversão da métrica em si é rápida (`metric_block_structure` 1.7ms,
`metric_inverse` 29ms via o bloco 2x2 `{t,phi}`) — o custo está rio
abaixo: `christoffel` sozinho mediu **70.5s**, e `riemann_mixed` não
terminou dentro de um orçamento de 180s.

**Por quê**: o denominador `Sigma = r^2 + a^2*cos^2(theta)` de Kerr é
genuinamente bivariado (`r` e `theta` aparecem os dois) — não existe
"variável de polo" única, exatamente o caso que a seção 2.3 deste
documento já nomeava, por esse nome, antes desta rodada existir, como o
que a redução multivariada por MDC (seção 6, PRS subresultante +
`content()`/`primitive_part()` recursivo) não colapsa totalmente:
"correta e não-reduzida, só maior do que precisaria". Kerr é o primeiro
fixture real deste projeto a bater nesse caso.

**Forma fechada correta que deveria sair**: `R_ab = 0` identicamente,
em todo componente, com `M`, `a`, `r`, `theta` livres (Kerr é vácuo).

**Teste de ouro adormecido**: `oderom-components/tests/kerr.rs`,
`ricci_of_kerr_is_identically_zero` — `#[ignore]`d, correto, esperando
o motor. Tirar o `#[ignore]` e a suíte passar É a prova de que este
limite foi corrigido.

### 7.2 `exp(a)^n` nunca funde em `exp(n*a)` — bloqueia Gödel

**O que trava**: o escalar de Ricci de Gödel (coordenadas originais de
Gödel, `t,x,y,z`, bloco `{t,y}`) calcula rápido (suíte inteira,
`oderom-components/tests/godel.rs`, ~0.2s) mas não reduz à forma
fechada. O valor bruto é
`(-3*exp(x)^2 + 2*exp(2x)) / (2*a^2*exp(x)^2 - a^2*exp(2x))`.

**Por quê**: essa fração É algebricamente `-1/a^2` — numerador
`= -exp(2x)`, denominador `= a^2*exp(2x)`, cancelando por
`exp(x)^2 = exp(2x)` — mas `AtomTable::exp` (`oderom-expr/src/poly.rs`)
deliberadamente nunca reescreve `exp(a)^n` como `exp(n*a)` (ao
contrário de `sin`/`cos`, que ganharam exatamente esse tipo de redução
de potência para `cos^2 -> 1-sin^2`, D-RF.7 acima). A decisão de
deixar isso de fora foi tomada e documentada antes desta rodada, com a
alegação empírica de que nenhum fixture real precisaria — Gödel é o
primeiro que precisa (`g_ty` traz `exp(x)`, `g_yy` traz `exp(2x)`, na
mesma razão).

**Forma fechada correta que deveria sair**: `R = -1/a^2`, constante
(convenção de assinatura maioria-mais-plus deste projeto; derivação
independente — não assumida de memória — no próprio comentário de
módulo de `godel.rs`).

**Teste de ouro adormecido**: `oderom-components/tests/godel.rs`,
`ricci_scalar_of_godel_is_minus_one_over_a_squared` — `#[ignore]`d,
correto, esperando o motor. Mesmo critério: tirar o `#[ignore]` e a
suíte passar é a prova.

### O que NÃO fazer com esta seção

Nenhum dos dois limites foi atacado nesta rodada, de propósito — MDC
sem variável de polo única geral o bastante para `Sigma` bivariado, e
fusão `exp(a)^n -> exp(n*a)` em `AtomTable::exp`, são os dois próximos
passos reais, mas cada um é uma extensão do próprio motor racional
(mesma escala de decisão que este documento inteiro já foi), não um
ajuste pontual — decisão para uma rodada própria, não para ser
resolvida de passagem enquanto se mexe em outra coisa.

## 8. Denominadores estruturados (Rodada Kerr): aprovado, com o conjunto de geradores medido, não suposto

Ataque aprovado para 7.1 especificamente (7.2/Gödel continua fora de
escopo desta rodada — mecanismo de geradores exponenciais, não de
denominadores estruturados; não tentar cobrir os dois com o mesmo
código). Trocar MDC multivariado geral por denominadores estruturados:
representar uma função racional como `(numerador em Poly, multiconjunto
de (fator, expoente))` sobre o anel localizado nos denominadores
declarados da métrica. Fechado sob produto, soma e derivada — produto e
quociente nunca chamam MDC; soma só precisa de teste de divisibilidade
exata por fator já conhecido.

### 8.1 Diagnóstico medido antes de codar (não suposto)

`oderom-cli/tests/diagnostic_kerr_denominators.rs`, lendo
`examples/kerr.od` pelo parser real (não pela API Rust): `christoffel`
de Kerr completo, 20.7s em release, 20 componentes independentes
não-nulas, exatamente **7 denominadores distintos**. Conferidos um a um
por álgebra direta (substituição `s = sin²θ`, expansão binomial/trinomial
comparada termo a termo):

| Denominador | Igual a |
|---|---|
| (grande, 15 termos) | `Σ²·Δ` |
| (médio, 6 termos) | `Σ²` |
| (grande, 10 termos) | `Σ³` |
| (médio, 8 termos) | `Σ·Δ` |
| `±(a²-a²sin²θ+r²)` (2 componentes, sinal oposto) | `±Σ` |
| (médio, 6 termos) | `sin(θ)·Σ²` |

Todos os sete são potência de `{Σ, Δ, sin θ}` — nenhum denominador fora
desse conjunto multiplicativo apareceu. O único ajuste em relação à
hipótese original (`{Σ, Δ}`) é o fator extra `sin θ`, esperado: `sin θ`
já aparece diretamente em `g_φφ`, não vem de `Σ`/`Δ`.

### 8.2 Livre-de-quadrados/irredutibilidade sobre `{Σ, Δ, sin θ}`, verificado executavelmente

`oderom-expr/tests/kerr_generators.rs` — não só conferido à mão, rodado
contra o motor `normalize()`/`denominator_degree` já em produção (que já
faz MDC multivariado recursivo de verdade, seção 6): dividir `X` por `Y`
e comparar o grau do denominador resultante contra o grau de `Y`
sozinho — se bater, nada cancelou, `X` e `Y` são coprimos; se cair,
havia fator comum. Todos os pares passam: `Σ`/`Δ` coprimos, `Σ`/`sin θ`
coprimos, `Δ`/`sin θ` coprimos, e cada gerador é livre-de-quadrados
(`gcd(P, dP/dvar) = 1` para cada variável que `P` carrega). Teste de
sanidade incluído (`Σ` contra `Σ²` — deve **falhar** a checagem de
coprimalidade) confirma que o método realmente detecta um fator
compartilhado quando existe, não passa vacuamente.

### 8.3 Dependência de convenção trigonométrica — registrado, não uma suposição livre

**A validade do conjunto `{Σ, Δ, sin θ}` como geradores irredutíveis
depende da convenção trigonométrica já em vigor neste repositório
(D-RF.7), não é um fato absoluto sobre `Σ`.** Hoje `sin` é o gerador
primário de expoente livre e `cos` é sempre reduzido a grau `<=1` via
`cos² -> 1-sin²` (`oderom-expr/src/poly.rs`). Nessa base:

- `sin θ` é gerador de grau 1 do anel — irredutível **porque todo
  gerador de grau 1 é irredutível**, não por tratamento especial.
- `Σ = r² + a²cos²θ`, ao passar por essa redução, vira
  `r² + a² - a²sin²θ = (r²+a²) - (a sin θ)²` — uma diferença de quadrados
  que **não fatora sobre `Q(r,a)`** porque `r²+a²` não é um quadrado
  perfeito nesse corpo (forma quadrática irredutível padrão). `Σ`
  permanece irredutível *nesta base*.

**Se a convenção fosse invertida** (`cos` gerador primário de expoente
livre, `sin` reduzido a grau `<=1` via `sin² -> 1-cos²`), a mesma
`Σ` viraria `r² + a²cos²θ` com `cos²θ` livre — ainda irredutível, sem
mudança. Mas **`sin θ` deixaria de ser gerador**: `sin²θ = 1-cos²θ =
(1-cos θ)(1+cos θ)` — produto de dois fatores próprios, **redutível**.
O denominador `sin(θ)·Σ²` medido em 8.1 teria que ser re-expresso via
`(1-cos θ)` e `(1+cos θ)` como geradores de localização (dois fatores
novos, não um), ou a base trigonométrica precisaria ficar fixa e
documentada como parte do contrato do motor. **Nenhuma mudança de base
está sendo feita agora** — isto é só o registro de que o conjunto de
geradores de 8.1/8.2 é uma consequência de D-RF.7, e trocar D-RF.7 no
futuro exige reavaliar esta seção inteira, não só ajustar um valor.

### 8.4 Dois requisitos do motor

1. **Redução após toda soma**: depois de somar duas `RationalFunction`s
   (numerador combinado sobre denominador comum), testar divisibilidade
   exata do numerador por cada gerador do multiconjunto de denominador e
   baixar o expoente correspondente enquanto dividir. Sem isso a forma
   não é canônica — sobra fator no numerador que deveria ter cancelado,
   e o Ricci de Kerr não reduz a zero, só fica menor (exatamente o
   sintoma que já causou o teto anterior, seção 1, item 3).
2. **Conjunto de geradores derivado, nunca hardcoded para Kerr**: o
   multiconjunto de localização nasce dos denominadores que a própria
   métrica declara (`g_rr`, qualquer componente com fração) mais os
   determinantes de bloco que `metric_inverse` calcula
   (`DESIGN-M2.md`, "D-M2.1 revisitada") — nunca uma lista fixa
   `[Sigma, Delta]` escrita a mão em algum lugar do código. Se, durante
   a computação, aparecer um denominador fora desse conjunto: não
   falhar. Tentar admitir o fator novo como gerador se ele for
   irredutível (mesmo teste executável de 8.2, aplicado ao fator novo
   contra o conjunto já conhecido); caso contrário (redutível, ou
   irredutibilidade não decidida a tempo), cair no caminho de MDC geral
   já existente (seção 6) para aquela expressão específica, com log —
   correção acima de desempenho, mesma regra de 2.6 ("reduzido ou não,
   nunca errado").

Fora de escopo, registrado para não ser esquecido: Gödel (`exp(a)^n`
nunca fundindo em `exp(n*a)`, seção 7.2) é um mecanismo diferente
(geradores exponenciais, identidade de fusão de potência) e não deve
ser resolvido pelo mesmo código que ataca 7.1.

## 8.5 Status: implementado — Kerr fecha, os dois erros reais que apareceram no caminho

`oderom-expr/src/localized.rs` (`LocalizationContext`, `normalize_localized`,
`pub(crate) fn localization_generators` em `oderom-components/src/curvature.rs`)
implementados e ligados a `christoffel_localized`/`riemann_mixed_localized`/
`ricci_tensor_localized`/`lower_index_localized`/`raise_index_localized`/
`kretschmann_localized` — funções irmãs das existentes, que continuam
inalteradas (Schwarzschild/Reissner-Nordström continuam no motor geral,
zero mudança de comportamento).

**Resultado medido, ponta a ponta, `examples/kerr.od`, release**:

| Estágio | Tempo |
|---|---|
| `metric_inverse` | ~14ms |
| `christoffel_localized` | 28ms |
| `riemann_mixed_localized` | 672ms |
| `ricci_tensor_localized` | 7.5ms |
| **Ricci de Kerr** | **identicamente zero, nas 16 componentes** |
| `lower_first_index_localized` + `kretschmann_localized` | (total do teste completo: 3.8s) |
| **Kretschmann de Kerr** | **bate exatamente com a forma fechada**, `48M²(r²-a²cos²θ)[(r²+a²cos²θ)²-16r²a²cos²θ]/(r²+a²cos²θ)⁶` |

Contra o motor geral: `christoffel` 70.5s, `riemann_mixed` não termina em
180s (seção 7.1). `oderom-components/tests/kerr.rs`:
`ricci_of_kerr_is_identically_zero_via_the_localized_engine`,
`kretschmann_of_kerr_matches_the_closed_form_via_the_localized_engine` —
ambos testes ativos, não `#[ignore]`d (os dois testes originais que
citavam este limite continuam `#[ignore]`d de propósito: eles exercitam
especificamente o motor *geral*, e esse continua com o mesmo teto —
mudar isso seria uma decisão de trocar o motor padrão do projeto, fora
do escopo desta rodada).

**Dois erros reais, achados por medição, não por inspeção — cada um
teria, sozinho, deixado Kerr no mesmo lugar de antes (correto na teoria,
impraticável na prática):**

1. **`add()` chamava o motor geral mesmo sem overflow nenhum.** A
   primeira versão de `LocalizedRational::add` sempre roteava a
   combinação dos denominadores por `RationalFunction::from_raw` (que
   sempre roda `poly_gcd` de conteúdo, mesmo quando o denominador
   `overflow` de ambos os lados já era `1`) — pagando o custo do MDC
   geral sobre o numerador da soma, que pode ser grande, em toda soma,
   mesmo quando não havia nada de fato para reduzir. `christoffel_localized`
   já saía rápido (nada de overflow no Christoffel de Kerr), mas
   qualquer soma subsequente que introduzisse um overflow, por menor que
   fosse, reintroduzia o custo geral por trás de tudo que viesse depois.
   Corrigido com um caminho rápido explícito: quando os dois lados têm
   `overflow == 1`, a soma é só `Poly::add`, sem nenhuma chamada ao MDC.
2. **Produto de dois geradores já conhecidos não decompunha.**
   `ginv` de Kerr naturalmente produz `Sigma*Delta` como *um* polinômio
   já multiplicado (nunca escrito assim por um humano — emerge de
   combinar `g^rr = Delta/Sigma` com outros termos) — a primeira versão
   de `classify_or_admit` só testava igualdade contra um gerador inteiro
   de cada vez, então `Sigma*Delta` não batia com nenhum, falhava o
   teste de coprimalidade contra `Sigma` (compartilha o fator `Sigma`,
   corretamente detectado) e caía no motor geral — só que isso acontecia
   em praticamente todo componente de `riemann_mixed`, reproduzindo
   exatamente o custo que o motor inteiro existe para evitar:
   `christoffel_localized` terminava em 393ms mas `riemann_mixed_localized`
   nunca retornava. Corrigido dividindo repetidamente por cada gerador
   já conhecido (`Poly::exact_div`, nunca busca de MDC) antes de decidir
   se sobrou algo para admitir ou para o motor geral — `Sigma*Delta`
   agora decompõe em `{Sigma: 1, Delta: 1}` diretamente.

Achado incidental, não corrigido (correto, só não maximamente fatorado):
`sin(θ)²` aparece como denominador em alguns componentes antes de
`sin(θ)` em si ter sido admitido como gerador (ordem de processamento) —
`sin(θ)²` sozinho nunca seria admitido de qualquer forma (não é
livre-de-quadrados), então esses casos caem no motor geral para aquele
fator específico, correto e barato (grau 2), registrado em
`ctx.fallback_log()`. Não é um bug: é exatamente o "reduzido ou não,
nunca errado" de 2.6, aplicado ao novo motor.

**Atualização — zero fallbacks, ∇g=0 reativado.** `classify_or_admit`
ganhou uma terceira saída: quando o resto não é livre-de-quadrados,
tenta recuperar o fator repetido via `gcd(resto, d(resto)/d(gerador))`
(primeiro passo padrão de fatoração livre-de-quadrados, Yun) antes de
desistir. Achado real, não hipotético: `sin(θ)²` aparecia como
denominador em alguns componentes de Kerr *antes* de `sin(θ)` em si ter
sido admitido como gerador (artefato da ordem de processamento dos
componentes, não uma propriedade da métrica) — `sin(θ)²` sozinho
corretamente falha livre-de-quadrados, mas `gcd(sin(θ)², 2sin(θ)) =
sin(θ)` recupera exatamente o gerador certo. Com isso, os dois testes de
ouro (`ricci_of_kerr_is_identically_zero_via_the_localized_engine`,
`kretschmann_of_kerr_matches_the_closed_form_via_the_localized_engine`)
rodam com **zero fallbacks** — `ctx.fallback_log().is_empty()` é
asserção, não nota de rodapé. `kerr_christoffel_satisfies_metric_compatibility`
(`oderom-cli/tests/kerr.rs`) também foi refeito sobre o motor localizado
e saiu do `#[ignore]`: os 213s eram inteiramente o custo do motor geral
sobre os denominadores de Kerr, não do `christoffel` em si — o mesmo
teste agora completa em segundos junto com os outros dois neste arquivo.

**Regra travada em código, registrada aqui para não se perder**:
pertencimento ao conjunto de localização se decide por **divisão**
(`Poly::exact_div` repetido contra cada gerador conhecido,
`decompose_against_known_generators`), nunca por **casamento sintático**
contra a forma como o denominador foi escrito. `Σ·Δ` nunca aparece
escrito à mão em lugar nenhum — emerge de combinar `g^rr = Δ/Σ` com
outros termos durante `christoffel`/`riemann_mixed`, já multiplicado.
Testar "isso bate com um gerador conhecido, tal e qual" teria falhado
silenciosamente (viraria fallback) em quase todo componente. A mesma
lógica se estende agora ao próprio processo de admissão: um fator que
falha livre-de-quadrados pode ainda conter, *dentro de si*, um gerador
genuíno ainda não descoberto — também resolvido por divisão (o gcd com a
derivada), não por inspeção da forma como o fator chegou.

**Taxonomia de testes, revisada por um erro real desta rodada**: testes
de identidade (∇g=0, simetrias de Riemann, Bianchi) pegam bug de
implementação e são estruturalmente cegos a fixture errada — dada
qualquer métrica simétrica invertível, os Christoffels construídos a
partir dela satisfazem ∇g=0 automaticamente, então ∇g=0 nunca poderia
ter pego o termo de frame-dragging faltante em `g_φφ` (o bug real desta
rodada). Só pegam fixture errada os testes de fato específico *daquela*
métrica: Ricci=0, a forma fechada do Kretschmann, o determinante do
bloco reduzindo a `-Δsin²θ`, o limite `a→0` dando Schwarzschild, o tipo
de Petrov. Regra adotada: toda fixture de métrica nova entra com pelo
menos um fato verificável específico a ela, não só testes de identidade
— um teste de identidade sozinho não é suficiente para confiar numa
fixture nova.

**Conclusão estratégica sobre biblioteca externa (FLINT/Symbolica),
revisada**: o ganho de três ordens de grandeza (christoffel 70.5s → 28ms,
riemann_mixed nunca termina em 180s → 672ms) não veio de um MDC mais
rápido — veio de **não chamar MDC**. Uma biblioteca externa mais rápida
no motor geral (FLINT, Symbolica) provavelmente compraria uma ou duas
ordens de grandeza ali, e mesmo assim talvez não fechasse `riemann_mixed`
de Kerr dentro de um orçamento razoável — o problema nunca foi a
velocidade do MDC, foi precisar de MDC nenhum para um caso sem variável
de polo única. A decisão de biblioteca externa (seção 6, nota final)
não está morta, mas o gatilho que a reabriria não é mais Kerr — Kerr já
fechou, com estrutura, sem ela. O próximo gatilho real seria uma métrica
cujos denominadores emergentes não formem um conjunto pequeno,
livre-de-quadrados e coprimo dois a dois (o "caminho de MDC geral" desta
seção existindo precisamente para não travar nesse caso, só ficar mais
lento) — isso ainda não apareceu em nenhum fixture real deste projeto.

**Não integrado ao `oderom-cli` ainda** (o comando `oderom kretschmann
examples/kerr.od` continua no motor geral, então continua lento/sem
terminar para Kerr especificamente) — próximo passo: tentar localizar
sempre que o conjunto de geradores puder ser derivado e passar na
coprimalidade, caindo no motor geral quando não passar (agnóstico à
forma da métrica, nunca condicionado a "é diagonal?"/"é Kerr?") — gated
por um teste diferencial contra o corpus que o motor geral já resolve
(Schwarzschild, Reissner-Nordström, S², Gödel).

## 8.6 Fase 1: orçamento de execução no caminho localizado (invariante arquitetural)

Antes de o CLI poder rotear por padrão para o motor localizado, ele
precisa herdar a mesma garantia de `--timeout`/`--max-nodes` que o motor
geral já tem via `Checkpoint` (`oderom-components::curvature`). Sem
isso, um fallback que não termina ficaria silenciosamente sem freio.

**Invariante, não conserto pontual**: toda saída da representação
localizada para o motor geral passa por exatamente **uma** função —
`fallback_to_general_engine` (`oderom-expr/src/localized.rs`) — e só
essa função chama `RationalFunction::from_raw`/`.pow()`. Antes desta
fase havia dois pontos de saída (o ramo de `overflow` de `add()`, e o
caso de fração aninhada em `reciprocal_pow()`); ambos foram unificados
para chamar essa única função, que consulta o `Checkpoint` do chamador
antes de prosseguir. A garantia não depende de alguém lembrar de
checar em cada novo ponto de saída futuro — depende de nunca existir
mais de um ponto de saída. Qualquer extensão futura deste motor que
precise invocar o motor geral **deve** passar por
`fallback_to_general_engine`, nunca chamar `RationalFunction` diretamente.

`Checkpoint<'a> = &'a mut dyn FnMut() -> bool` é definido em
`oderom-expr/src/localized.rs`, espelhando (não importando — a
dependência vai na direção oposta) o tipo já usado em
`oderom-components::curvature`. Granularidade: grossa no laço externo
(um `checkpoint()` por componente independente, mesmo padrão de
`christoffel_checkpointed` e companhia) mais um `checkpoint()`
obrigatório dentro de `fallback_to_general_engine` — nada dentro da
aritmética de `Poly` (`add`/`mul`/`exact_div`), que já é barata o
suficiente (28ms/672ms de ponta a ponta, medido) para que checagem fina
custasse mais do que protege.

Estouro de orçamento erra com a computação inteira — nunca resultado
parcial —, com uma nova variante, `ComponentError::LocalizationFallbackBudgetExceeded`
(`oderom-components/src/error.rs`), carregando o nome do componente
sendo calculado (`Gamma^0_{{00}}`, `R^0_{{001}}`, etc. -- conceito que
`oderom-expr` propriamente não tem, por isso a conversão acontece uma
camada acima, em `curvature::budget_exceeded_error`), o denominador que
escapou do conjunto de geradores, e o conjunto de geradores em vigor
naquele momento — exatamente a entrada de que precisa a decisão
"admitir esse fator como gerador ou não". Verificado disparando de
verdade, não só existindo: `oderom-expr/src/localized.rs`'s
`the_execution_budget_actually_fires_at_the_fallback_boundary` (nível do
motor, controle preciso do ponto de disparo) e
`oderom-components/tests/kerr.rs`'s
`christoffel_localized_reports_the_escaped_denominator_when_the_budget_runs_out`
(nível do pipeline real, métrica sintética `1/(x-1)^3` desenhada para
forçar o fallback, já que o Kerr real não cai mais nele).

**Independência de ordem, verificada, não suposta**: o conserto do
`sin(θ)²`-antes-de-`sin(θ)` (recuperação de fator repetido via
`gcd(p, dp/dvar)`) estabeleceu um invariante testável —
`oderom-expr/src/localized.rs`'s módulo `order_independence`, exaustivo
(nunca amostrado) sobre `4! = 24` ordenações de `{Σ, Δ, sin θ, sin²θ}`,
mais o caso nomeado explicitamente que causou o bug original, mais uma
terceira checagem específica: um candidato composto (`Σ·Δ`) misturado
com seus próprios fatores, exaustivo sobre `3! = 6` ordenações. Achado
ao construir esse terceiro teste (registrado aqui porque uma primeira
versão dele comparou dois *conjuntos diferentes* — `{Σ·Δ, Σ}` contra
`{Σ, Δ}` — e leu a diferença como uma suposta falha de independência de
ordem): não há falha real. Uma vez que cada elemento do *mesmo* conjunto
tem chance de ser apresentado (o que uma permutação de verdade garante),
`Σ·Δ` apresentado primeiro ainda recupera `Σ` via `find_repeated_factor`,
e o `Δ` que aparece depois na mesma ordenação é admitido normalmente
contra ele — o conjunto final converge para `{Σ, Δ}` independentemente
da posição. Registrado como o processo real de verificação, não só o
resultado, porque a comparação inválida quase virou um relatório de
limitação inexistente.

**Overhead do checkpoint: nenhum. Alegação anterior retratada.**

Uma versão anterior desta seção afirmava um custo de +20-30% em
`riemann_mixed_localized` (672ms -> ~800-870ms) introduzido pelo
`Checkpoint`. **Essa alegação estava errada e está retratada aqui**, com
o processo que a derrubou, porque o erro é instrutivo: as duas medições
comparadas foram tiradas em momentos diferentes, sob carga de máquina
diferente, e a diferença era artefato de carga, não do código.

Medição controlada (mesma máquina, mesma janela de tempo, mesma carga,
quatro repetições de cada), com um `git worktree` no commit
pré-Fase-1 (`c3010d3`, que não tem `Checkpoint` nenhum) para servir de
base real em vez de um número anotado dias antes:

| Variante | `riemann_mixed_localized` |
|---|---|
| `c3010d3` — sem `Checkpoint` algum | 1.42-1.50s |
| Fase 1 — `Checkpoint` + `Result` propagado | 1.37-1.53s |
| Fase 1 + erro em `Box` | 1.38-1.61s |

As três são indistinguíveis. **Não há overhead do checkpoint a
explicar** — nem por despacho dinâmico (hipótese A, já falsificada por
contagem: 320 invocações no total, zero fallbacks), nem por `Result`/
inlining (hipótese B), nem por tamanho do erro (hipótese C). O número
absoluto varia muito com a carga da máquina (~672ms numa máquina ociosa,
~1.4s sob `load average` ~5), e foi exatamente essa variação que a
comparação original leu como regressão.

**Sobre a hipótese C especificamente** (tamanho do erro), medida antes
de ser descartada, porque o dado em si é real e vale registrar:
`size_of::<LocalizedRational>()` = 72 bytes,
`size_of::<LocalizationBudgetExceeded>()` = 96,
`size_of::<Result<LocalizedRational, LocalizationBudgetExceeded>>()` =
96 — ou seja, o `Result` era de fato 33% maior que o valor nu, e
`Box`ar o erro leva o `Result` de volta a exatos 72 bytes (otimização de
nicho: `Box` é não-nulo, então o discriminante cabe no nicho). A
mudança é real e mensurável *no tamanho*; **não é mensurável no tempo**,
então foi revertida — não se acrescenta `Box` à assinatura pública por
um ganho que não aparece em nenhuma medição.

**Lição de método, registrada para não se repetir**: comparar um número
medido agora contra um número anotado numa sessão anterior não é
medição, é anedota. Qualquer alegação futura de regressão de desempenho
neste projeto precisa de A/B na mesma janela (`git worktree` no commit
base, builds dos dois lados, execuções intercaladas) antes de virar
linha de documento — especialmente numa máquina de trabalho com
navegador aberto, onde a carga de fundo domina facilmente uma diferença
de 20%.

**Nota sobre granularidade** (correção de premissa, mantida da versão
anterior desta seção porque continua válida): pediu-se granularidade
*grossa* no caminho localizado, e simultaneamente que *todo* fallback
carregasse orçamento. Como os pontos de saída para o motor geral
(`add()`, `reciprocal_pow()`) ficam dentro da própria aritmética
recursiva, a segunda exigência força a primeira a ser fina — não há como
ter as duas. A escolha feita foi honrar a segunda (correção acima de
desempenho). Agora que se sabe que o custo é nulo, a tensão é teórica,
não prática.

### Proposta registrada, explicitamente NÃO implementada: canal de erro fora de banda

Registrada por completude, com a ressalva de que a medição acima
**removeu a motivação que a originou**: não há overhead para recuperar.
A ideia: como o caminho localizado puro nunca falha (zero fallbacks
medidos), propagar `Result` por cada quadro da recursão paga por um erro
que ocorre zero vezes; guardar o erro no próprio `LocalizationContext`
(já threaded como `&mut` por toda a recursão) deixaria
`expr_to_localized`/`add`/`reciprocal_pow` voltarem a retornar valor
puro.

**Não implementar** — e a razão principal não é mais desempenho: a troca
substitui uma garantia do compilador (`Result` que *não dá* para
ignorar) por um invariante mantido à mão (lembrar de consultar o campo
de erro no topo, e lembrar de curto-circuitar `fallback_to_general_engine`
depois que ele for setado, senão o componente corrente continua pagando
o custo que o orçamento existe para cortar). Isso é o oposto da direção
que esta seção inteira vem seguindo — a Fase 1 unificou a fronteira de
fallback justamente para que a garantia fosse estrutural em vez de
depender de alguém lembrar.

Condição de entrada, se algum dia entrar: um teste que **force o
estouro** e verifique a propagação ponta a ponta (não só que o erro é
setado, mas que ele chega ao chamador e que nada de trabalho caro roda
depois de setado). Sem esse teste, a troca não vale ser feita.

**Não integrado ao `oderom-cli` ainda** (o comando `oderom kretschmann
examples/kerr.od` continua no motor geral, então continua lento/sem
terminar para Kerr especificamente) — próximo passo: tentar localizar
sempre que o conjunto de geradores puder ser derivado e passar na
coprimalidade, caindo no motor geral quando não passar (agnóstico à
forma da métrica, nunca condicionado a "é diagonal?"/"é Kerr?") — gated
por um teste diferencial contra o corpus que o motor geral já resolve
(Schwarzschild, Reissner-Nordström, S², Gödel).

## 8.6 Fase 1: orçamento de execução no caminho localizado (invariante arquitetural)

Antes de o CLI poder rotear por padrão para o motor localizado, ele
precisa herdar a mesma garantia de `--timeout`/`--max-nodes` que o motor
geral já tem via `Checkpoint` (`oderom-components::curvature`). Sem
isso, um fallback que não termina ficaria silenciosamente sem freio.

**Invariante, não conserto pontual**: toda saída da representação
localizada para o motor geral passa por exatamente **uma** função —
`fallback_to_general_engine` (`oderom-expr/src/localized.rs`) — e só
essa função chama `RationalFunction::from_raw`/`.pow()`. Antes desta
fase havia dois pontos de saída (o ramo de `overflow` de `add()`, e o
caso de fração aninhada em `reciprocal_pow()`); ambos foram unificados
para chamar essa única função, que consulta o `Checkpoint` do chamador
antes de prosseguir. A garantia não depende de alguém lembrar de
checar em cada novo ponto de saída futuro — depende de nunca existir
mais de um ponto de saída. Qualquer extensão futura deste motor que
precise invocar o motor geral **deve** passar por
`fallback_to_general_engine`, nunca chamar `RationalFunction` diretamente.

`Checkpoint<'a> = &'a mut dyn FnMut() -> bool` é definido em
`oderom-expr/src/localized.rs`, espelhando (não importando — a
dependência vai na direção oposta) o tipo já usado em
`oderom-components::curvature`. Granularidade: grossa no laço externo
(um `checkpoint()` por componente independente, mesmo padrão de
`christoffel_checkpointed` e companhia) mais um `checkpoint()`
obrigatório dentro de `fallback_to_general_engine` — nada dentro da
aritmética de `Poly` (`add`/`mul`/`exact_div`), que já é barata o
suficiente (28ms/672ms de ponta a ponta, medido) para que checagem fina
custasse mais do que protege.

Estouro de orçamento erra com a computação inteira — nunca resultado
parcial —, com uma nova variante, `ComponentError::LocalizationFallbackBudgetExceeded`
(`oderom-components/src/error.rs`), carregando o nome do componente
sendo calculado (`Gamma^0_{{00}}`, `R^0_{{001}}`, etc. -- conceito que
`oderom-expr` propriamente não tem, por isso a conversão acontece uma
camada acima, em `curvature::budget_exceeded_error`), o denominador que
escapou do conjunto de geradores, e o conjunto de geradores em vigor
naquele momento — exatamente a entrada de que precisa a decisão
"admitir esse fator como gerador ou não". Verificado disparando de
verdade, não só existindo: `oderom-expr/src/localized.rs`'s
`the_execution_budget_actually_fires_at_the_fallback_boundary` (nível do
motor, controle preciso do ponto de disparo) e
`oderom-components/tests/kerr.rs`'s
`christoffel_localized_reports_the_escaped_denominator_when_the_budget_runs_out`
(nível do pipeline real, métrica sintética `1/(x-1)^3` desenhada para
forçar o fallback, já que o Kerr real não cai mais nele).

**Independência de ordem, verificada, não suposta**: o conserto do
`sin(θ)²`-antes-de-`sin(θ)` (recuperação de fator repetido via
`gcd(p, dp/dvar)`) estabeleceu um invariante testável —
`oderom-expr/src/localized.rs`'s módulo `order_independence`, exaustivo
(nunca amostrado) sobre `4! = 24` ordenações de `{Σ, Δ, sin θ, sin²θ}`,
mais o caso nomeado explicitamente que causou o bug original, mais uma
terceira checagem específica: um candidato composto (`Σ·Δ`) misturado
com seus próprios fatores, exaustivo sobre `3! = 6` ordenações. Achado
ao construir esse terceiro teste (registrado aqui porque uma primeira
versão dele comparou dois *conjuntos diferentes* — `{Σ·Δ, Σ}` contra
`{Σ, Δ}` — e leu a diferença como uma suposta falha de independência de
ordem): não há falha real. Uma vez que cada elemento do *mesmo* conjunto
tem chance de ser apresentado (o que uma permutação de verdade garante),
`Σ·Δ` apresentado primeiro ainda recupera `Σ` via `find_repeated_factor`,
e o `Δ` que aparece depois na mesma ordenação é admitido normalmente
contra ele — o conjunto final converge para `{Σ, Δ}` independentemente
da posição. Registrado como o processo real de verificação, não só o
resultado, porque a comparação inválida quase virou um relatório de
limitação inexistente.

**Overhead do checkpoint: nenhum. Alegação anterior retratada.**

Uma versão anterior desta seção afirmava um custo de +20-30% em
`riemann_mixed_localized` (672ms -> ~800-870ms) introduzido pelo
`Checkpoint`. **Essa alegação estava errada e está retratada aqui**, com
o processo que a derrubou, porque o erro é instrutivo: as duas medições
comparadas foram tiradas em momentos diferentes, sob carga de máquina
diferente, e a diferença era artefato de carga, não do código.

Medição controlada (mesma máquina, mesma janela de tempo, mesma carga,
quatro repetições de cada), com um `git worktree` no commit
pré-Fase-1 (`c3010d3`, que não tem `Checkpoint` nenhum) para servir de
base real em vez de um número anotado dias antes:

| Variante | `riemann_mixed_localized` |
|---|---|
| `c3010d3` — sem `Checkpoint` algum | 1.42-1.50s |
| Fase 1 — `Checkpoint` + `Result` propagado | 1.37-1.53s |
| Fase 1 + erro em `Box` | 1.38-1.61s |

As três são indistinguíveis. **Não há overhead do checkpoint a
explicar** — nem por despacho dinâmico (hipótese A, já falsificada por
contagem: 320 invocações no total, zero fallbacks), nem por `Result`/
inlining (hipótese B), nem por tamanho do erro (hipótese C). O número
absoluto varia muito com a carga da máquina (~672ms numa máquina ociosa,
~1.4s sob `load average` ~5), e foi exatamente essa variação que a
comparação original leu como regressão.

**Sobre a hipótese C especificamente** (tamanho do erro), medida antes
de ser descartada, porque o dado em si é real e vale registrar:
`size_of::<LocalizedRational>()` = 72 bytes,
`size_of::<LocalizationBudgetExceeded>()` = 96,
`size_of::<Result<LocalizedRational, LocalizationBudgetExceeded>>()` =
96 — ou seja, o `Result` era de fato 33% maior que o valor nu, e
`Box`ar o erro leva o `Result` de volta a exatos 72 bytes (otimização de
nicho: `Box` é não-nulo, então o discriminante cabe no nicho). A
mudança é real e mensurável *no tamanho*; **não é mensurável no tempo**,
então foi revertida — não se acrescenta `Box` à assinatura pública por
um ganho que não aparece em nenhuma medição.

**Lição de método, registrada para não se repetir**: comparar um número
medido agora contra um número anotado numa sessão anterior não é
medição, é anedota. Qualquer alegação futura de regressão de desempenho
neste projeto precisa de A/B na mesma janela (`git worktree` no commit
base, builds dos dois lados, execuções intercaladas) antes de virar
linha de documento — especialmente numa máquina de trabalho com
navegador aberto, onde a carga de fundo domina facilmente uma diferença
de 20%.

**Nota sobre granularidade** (correção de premissa, mantida da versão
anterior desta seção porque continua válida): pediu-se granularidade
*grossa* no caminho localizado, e simultaneamente que *todo* fallback
carregasse orçamento. Como os pontos de saída para o motor geral
(`add()`, `reciprocal_pow()`) ficam dentro da própria aritmética
recursiva, a segunda exigência força a primeira a ser fina — não há como
ter as duas. A escolha feita foi honrar a segunda (correção acima de
desempenho). Agora que se sabe que o custo é nulo, a tensão é teórica,
não prática.

### Proposta registrada, não implementada: canal de erro fora de banda

A medição acima aponta o conserto exato, se algum dia o overhead
importar: **o caminho localizado puro nunca falha** (zero fallbacks
medidos), então propagar `Result` por cada quadro da recursão paga, em
todo componente, por um erro que ocorre zero vezes. Alternativa:
guardar o erro no próprio `LocalizationContext` (que já é threaded como
`&mut` por toda a recursão — custo zero adicional), com
`fallback_to_general_engine` sendo o único a setá-lo, e só o topo
(`normalize_localized`) consultando/tomando. Assim
`expr_to_localized`/`add`/`reciprocal_pow` voltam a retornar valor puro,
sem `Result`, e o inlining perdido volta.

Detalhe necessário para não quebrar a garantia: uma vez setado o erro,
`fallback_to_general_engine` precisa curto-circuitar (devolver valor
trivial imediatamente em vez de chamar o motor geral), senão o
componente corrente continuaria pagando o custo que o orçamento existe
para cortar. Com isso, o pior caso após o estouro é terminar o trabalho
puramente localizado do componente corrente — barato e limitado — e o
laço externo (`curvature.rs`) já aborta prontamente no componente
seguinte. A API pública não muda: `normalize_localized` e os
`*_localized_checkpointed` continuam retornando `Result`; só a recursão
interna deixa de propagá-lo. **Não implementado** — proposta, aguardando
decisão, e independente da Fase 2 (não altera nenhuma assinatura que o
CLI use).

## Regra de medição

Todo tempo reportado declara o **perfil de compilação** e o **estado da máquina**,
ou não é um número. Quatro medições neste projeto foram lidas errado por omitir
um dos dois: uma regressão de 20-30% que não existia, um `falharam=1` causado por
carga que eu mesmo gerei, um "travamento" que era lentidão, e "26 segundos" que
eram 3,18s (debug contra release, fator 9x).
