# DESIGN-TENSOR-ALGEBRA.md

Plano para ampliar a manipulação de equações tensoriais em índices abstratos.
Escrito a partir de "ODEROM — capacidades e lacunas".

---

## 0. A tese

O relatório trata 3.1 (Leibniz) e 3.2 (casamento de somas) como dois itens
separados. Eles não são. São a mesma lacuna vista de dois lados: **a e-graph hoje
representa produtos e somas de monômios canônicos, e nada mais.** Tudo que falta
— Leibniz, comutador de derivadas, substituição, contração declarada — precisa ou
de um construtor acima do monômio, ou de casamento que não exija que a soma tenha
exatamente a aridade do padrão.

Duas decisões destravam quase tudo:

1. **A derivada entra como nó, mas é transiente.** `∇_c(AB)` só precisa existir
   entre o parser e a normalização. Leibniz é orientável e termina; empurrar para
   baixo devolve o espaço representável por `Monomial`, onde Butler–Portugal
   continua valendo. Isso é uma fração do custo de uma e-graph tipada geral.

2. **A soma vira multiconjunto com coeficientes racionais, e o casamento de
   identidades vira seleção de sub-multiconjunto.** Isso é a correção certa que o
   relatório já identificou em 3.2, e é pré-requisito de tudo em 3.1 — Leibniz
   produz somas que precisam colher termos.

Acrescento uma terceira, que não está no relatório e acho que vale mais que as
duas: **um procedimento de decisão completo por álgebra linear sobre a base
canônica**, rodando ao lado do caminho rápido da e-graph. Identidades multi-termo
são relações lineares entre elementos de base canônica. Escaloná-las decide zero,
e dá forma normal, não só teste. Serve de oráculo para testar diferencialmente o
caminho rápido — que é heurístico e vai continuar sendo.

Meta de aceitação do plano inteiro, uma frase: **`∇^a G_{ab} = 0` derivado, não
declarado.**

---

## 1. Mapa das lacunas em camadas

Reorganizando o que está em 2.2 e 3.x por camada, porque a ordem de ataque sai daqui.

| Camada | Existe hoje | Falta |
|---|---|---|
| Representação | `Monomial` (produto de fatores com slots), `ENode = Term \| Sum` | nó de derivada; produto como nó; coeficientes na soma |
| Canonicalização | Butler–Portugal/BSGS sobre grupo de slots declarado; zero por simetria | forma normal módulo identidades multi-termo |
| Casamento | identidade casa na raiz `Sum` com exatamente k e-classes | sub-multiconjunto; múltiplas cópias; coeficiente parcial |
| Axiomas | `--metric`, `--bianchi`, `--bianchi2`, `--metric-compatible` | Leibniz, linearidade de ∇, identidade de Ricci (comutador), contração declarada |
| Operações do usuário | canon, simplify | substituição com variáveis de padrão, definição de heads derivados, derivada de Lie / Killing |
| Verificação | controle negativo por par de flags | ponte para componentes; oráculo completo; métrica genérica |
| Integração | subcomandos CLI sobre strings | blocos `.od`, notebook |

Note que `head`, `symmetry` e `on` já estão na linguagem `.od` (§2.1). A metade de
índices abstratos já tem meia ponte para a linguagem; falta o verbo, não o
substantivo.

---

## 2. Decisão A — derivada como nó transiente

### O problema como o relatório o coloca

`ENode` tem só `Term(Monomial)` e `Sum(..)`. Uma derivada vive dentro do monômio
como head de um fator, então `(∇_c A)B` e `A(∇_c B)` existem, mas `∇_c(AB)` não.
Sem lado esquerdo não há o que distribuir. E `ENode::Deriv(índice, EClassId)` leva
índice livre para dentro de um nó — território que `monomial.rs:326` declara não
policiar.

### A saída

O território que `monomial.rs:326` não policia só precisa ser atravessado, não
habitado. Concretamente:

```rust
enum ENode {
    Term(Monomial),
    Sum(SumNode),            // ver Decisão B
    Deriv(SlotIndex, EClassId),   // NOVO — transiente
}
```

com **uma invariante forte e checada**: ao final da normalização de entrada,
nenhuma e-class alcançável a partir da raiz contém um `Deriv`. `Deriv` existe
entre o parser e o passe de descida, e some.

O passe de descida é uma reescrita orientada, aplicada até saturar:

```
Deriv(c, Sum[t1..tn])          → Sum[Deriv(c,t1) .. Deriv(c,tn)]     (linearidade)
Deriv(c, Term(f1·f2·…·fk))     → Σ_i Term(f1·…·(∇_c f_i)·…·fk)        (Leibniz)
Deriv(c, Term(escalar const))  → 0
```

Termina: cada aplicação estritamente reduz a profundidade de `Deriv` acima de um
`Term`, e Leibniz numa raiz de k fatores gera k termos com `∇` já dentro do fator,
que é a forma que `Monomial` representa. Não há reescrita que reintroduza `Deriv`.

Consequências que valem escrever antes de implementar:

- **Butler–Portugal fica intocado.** Ele continua vendo só monômios planos. Esse
  é o ponto todo da escolha.
- **Extração fica intocada.** Se `Deriv` nunca sobrevive à normalização, `extract`
  não precisa saber dele. O relatório listava `extract` como custo; ele sai da conta.
- **O parser muda pouco.** Ele passa a poder produzir `Deriv`; não precisa mais
  rejeitar `∇_c(A B)`.
- **Índice livre dentro de um nó existe, mas por uma janela.** O checador da
  invariante (§5) roda no fim da normalização em builds de debug e nos testes.

### O que isso *não* dá

Não dá a direção de subida — reconhecer `(∇_c A)B + A(∇_c B)` e reescrever como
`∇_c(AB)`. Isso é fatoração, é ambígua (qual agrupamento?), e é apresentação, não
correção. Fica fora, e fica registrado como fora. Se um dia for pedida, o lugar
dela é na extração com uma função de custo, não na normalização.

### Custo

Um novo variante de enum, um passe de reescrita (~150–250 linhas), mudanças no
parser para aceitar `∇_c( ... )` com parênteses, e o checador de invariante.
Não toca `oderom-canon`.

---

## 3. Decisão B — soma como multiconjunto com coeficientes

### O problema como o relatório o coloca

> As identidades casam estruturalmente com o nó `Sum`. Reduzem quando a raiz é a
> instância de três e-classes. Uma soma de seis que contém duas cópias dela não é
> vista. Tentei coletar termos semelhantes antes da e-graph; não funciona, porque
> quem canonicaliza é `add_monomial`, não `Monomial::try_new`.

O diagnóstico está certo e a conclusão também: a correção é casar somas a menos de
coleção de termos. Detalhando o que isso quer dizer em estrutura:

```rust
struct SumNode {
    terms: BTreeMap<EClassId, Rational>,   // multiconjunto com coeficientes, ordenado
}
```

Três propriedades que isso compra e que a lista de e-classes não comprava:

1. **Coleção é a construção.** Inserir `t` com coeficiente `c` num `SumNode` que já
   tem `t` soma os coeficientes. Termo com coeficiente zero sai. Isso mata a
   necessidade de um passe de coleção separado — que foi exatamente o que falhou
   antes por rodar na camada errada.
2. **Normal form AC de graça.** `BTreeMap` sobre `EClassId` dá associatividade,
   comutatividade e ordem determinística sem passe extra. Somas aninhadas achatam
   na inserção.
3. **Aplicação parcial de identidade fica expressável.** Bianchi diz que
   `t1+t2+t3 = 0`. Numa soma com coeficientes, aplicar é: achar `t1,t2,t3` com
   coeficiente comum `c = min(c1,c2,c3)` (com sinal), subtrair `c` de cada. O que
   sobra continua sendo uma soma legítima. Sem coeficientes isso não se escreve.

### Casamento por sub-multiconjunto

Com a soma nessa forma, o casamento de uma identidade de k termos vira:

> achar um subconjunto de k termos da soma e uma substituição consistente de
> índices tal que o subconjunto seja uma instância do padrão.

É casamento AC com seleção de subconjunto — NP-difícil em geral, irrelevante aqui,
porque k ≤ 5 e as somas têm dezenas de termos, não milhares. Mas o ingênuo `C(n,k)`
precisa de poda. A poda certa:

**Chave de forma.** Para cada monômio, calcular uma chave que ignora nomes de
índices e retém tudo o mais:

```
shape_key(m) = multiconjunto ordenado de (head_id, aridade, ordem_de_derivada)
             ⊕ padrão de contração canônico com nomes apagados
```

Termos com chaves de forma diferentes nunca casam com o mesmo slot do padrão. Então:
indexar os termos da soma por chave, gerar só os k-subconjuntos cujo vetor de chaves
bate com o do padrão, e só então tentar a unificação de índices. Na prática isso
reduz o espaço de busca a quase nada, porque um padrão de Bianchi só casa com termos
que têm um Riemann.

**Ordem de tentativa.** Determinística — ordem de `EClassId`, que já é estável. Nada
de heurística de "melhor primeiro" nesta rodada; o caminho completo (Decisão C) é
que resolve o caso em que a ordem gulosa erra.

### Consequência imediata

`R[a,b,[c,d;e]]` — o caso que o relatório cita como falhando embora a mesma soma
escrita à mão zere — passa a fechar. Esse é o teste de aceitação da rodada.

---

## 4. Decisão C — procedimento de decisão completo por espaço linear

Esta é a adição que não está no relatório e a que eu recomendo com mais convicção.

### O argumento

Depois de Butler–Portugal, cada monômio é um elemento de base canônico. Uma
identidade multi-termo é **uma relação linear entre elementos de base**. A pergunta
"esta expressão é zero módulo Bianchi?" é literalmente:

> o vetor de coeficientes da expressão está no espaço-linha gerado pelas instâncias
> das identidades?

E isso se decide por escalonamento sobre ℚ. Não é heurística; é decisão. E dá mais
que teste de zero: reduzir o vetor módulo a forma escalonada reduzida dá **forma
normal** — que é precisamente o que `simplify` deveria devolver e hoje não garante.

### Estratificação

Identidades preservam: multiconjunto de heads, grau, conjunto de índices livres,
ordem total de derivada. Então a matriz é bloco-diagonal por estrato, e cada bloco
é pequeno. Nunca se monta uma matriz global.

```
stratum_signature(m) = (multiconjunto de heads com ordem de derivada,
                        índices livres com variância,
                        número de contrações)
```

### Geração do conjunto gerador

Para uma identidade `I` com n índices e um estrato S:

1. Enumerar a base canônica de S. (Finita: heads fixos, alfabeto de índices
   limitado pelos livres de S mais os dummies necessários.)
2. Para cada monômio de base `m ∈ S` e cada fator de `m` cujo head casa com o head
   do padrão de `I`, gerar todas as substituições dos índices de `I` para o alfabeto
   de índices de `m` — **incluindo as não injetivas**, que são exatamente as
   contrações da identidade, desde que respeitem a regra de variância (um em cima,
   um embaixo).
3. Para cada substituição, montar `I_σ ⊗ resto`, canonicalizar cada monômio
   resultante, e emitir o vetor.

Ordem de grandeza: primeira Bianchi tem 4 índices; alfabeto de 6 dá 1296
substituições, cada uma uma canonicalização. Segunda Bianchi, 5 índices, 7776.
Isso é nada. Para produtos de dois Riemanns o multiplicador é o número de fatores
"resto", que também é pequeno.

### Cache

A matriz escalonada de um estrato depende só da assinatura do estrato e do conjunto
de identidades ativas. **Tabela de relações cacheável**, chaveada por
`(stratum_signature, conjunto de flags de axioma)`. É o mesmo movimento que o
`DefFingerprint` da sessão já faz para outra coisa. É também, essencialmente, o
que o Invar do xAct pré-computa — com a diferença de que aqui é derivado em tempo
de execução, não tabelado à mão, o que é consistente com "geradores derivados,
nunca hardcoded".

### Honestidade sobre o que "completo" quer dizer

Completo **dentro do estrato**: decide toda consequência linear das identidades
declaradas naquele grau, incluindo as obtidas por contração. Não fecha sobre
consequências que passariam por grau maior e voltariam — isso exigiria fechamento
por produto e derivada, que é problema de base de Gröbner e fica fora. Isso precisa
estar na docstring do subcomando, não só aqui. O relatório mostra que você prefere
limite conhecido e escrito a garantia vaga.

### Interface

```
oderom simplify --engine=egraph      # rápido, heurístico, padrão
oderom simplify --engine=linear      # completo no estrato, forma normal
oderom simplify --engine=both        # roda os dois, falha se discordarem
```

`both` é o modo de teste diferencial. Não é modo de produção; é a suíte.

---

## 5. A invariante que segura a correção

Esta seção é o risco principal do plano e merece ser lida antes das rodadas.

Uma e-graph funde duas e-classes quando prova que são iguais. Mas `A[a]` e `A[b]`
**não são iguais** — índices livres tornam a e-class dependente de índice. Hoje
isso não morde porque os índices vivem dentro de `Monomial` e a igualdade de
`Monomial` os inclui. Assim que `Deriv(c, EClassId)` existe, a e-class referenciada
carrega índices livres, e uma fusão indevida produz nonsense silencioso.

Duas invariantes, ambas checadas:

**I1 — assinatura uniforme.** Todo e-node de uma mesma e-class tem o mesmo conjunto
de índices livres, com a mesma variância. Violação é bug, não caso a tratar.
Checador em `debug_assert` mais um teste dedicado que constrói a violação de
propósito e exige o pânico.

**I2 — dummies canônicos antes de hash.** `T[a,c]S[c,b]` e `T[a,d]S[d,b]` são o
mesmo objeto. Se entrarem na e-graph com rótulos diferentes, ela fragmenta e o
casamento passa a depender de como o usuário digitou. Butler–Portugal já produz
rótulos canônicos de dummy; a regra é que **nenhum monômio entra na e-graph sem
passar por canonicalização**. Isso é presumivelmente o que `add_monomial` já
garante — mas passa a ser invariante declarada, com teste, não propriedade
emergente do fluxo atual.

O princípio do relatório sobre fallback silencioso se aplica igual aqui: **uma
violação de I1 ou I2 deve quebrar teste, não degradar resultado.** Um contador de
violações assertado em zero, como o do engine localizado, é o formato que já
funcionou.

---

## 6. Plano de rodadas

Uma capacidade isolada por rodada, verificação real em cada uma, como de praxe.

### R0 — diagnóstico, sem implementar nada

Instrumentar `simplify` sobre o corpus de testes atual e registrar:

- distribuição do número de termos na `Sum` raiz;
- quantas tentativas de casamento falham **só** por contagem de termos, versus
  por incompatibilidade de índices;
- distribuição de grau, número de índices distintos, e tamanho da base canônica
  por estrato;
- quantos casos do corpus caem no modo "soma com cópias" de 3.2.

**Por que primeiro:** o mesmo movimento que instrumentar `riemann_mixed` para
contar denominadores distintos antes de comprometer com o engine localizado. Se o
tamanho de estrato típico for pequeno, a Decisão C pode virar o caminho *padrão* e
a e-graph vira otimização — o que reordena R2 e R3. Não decida isso sem o número.

**Entrega:** um relatório de números. Zero mudança de comportamento.

### R1 — `SumNode` com coeficientes

Trocar a representação da soma por multiconjunto com coeficientes racionais.
Coleção na inserção. Achatamento de somas aninhadas na inserção.

- **Aceitação:** toda a suíte atual passa sem mudança de resultado esperado; somas
  com termos repetidos colhem; termo de coeficiente zero desaparece.
- **Controle negativo:** `T[a,b] - T[a,b]` zera; `T[a,b] - T[b,a]` não zera sem
  simetria declarada.
- **Risco:** ordem de termos na saída muda. Vários testes comparam string. Espere
  churn de expectativas e trate como churn, não como regressão.

### R2 — casamento por sub-multiconjunto, com poda por chave de forma

- **Aceitação:** `R[a,b,[c,d;e]]` zera pela CLI. É o caso nominal de 3.2.
- **Aceitação 2:** soma de seis termos contendo duas instâncias disjuntas de Bianchi
  zera; contendo uma instância mais lixo, reduz ao lixo.
- **Controle negativo:** soma de seis com dois termos de uma instância e um de outra
  (que não fecha) **não** zera.
- **Risco:** aplicação gulosa pode escolher a sobreposição errada quando duas
  instâncias compartilham termo. Documentar como limite, e é R3 que o cobre.

### R3 — engine linear e teste diferencial

Implementar §4. Expor `--engine=egraph|linear|both`. Colocar `both` na suíte.

- **Aceitação:** em todo caso do corpus onde a e-graph reduz, o linear concorda.
  Em pelo menos um caso construído onde a e-graph erra por sobreposição gulosa,
  o linear acerta e `both` falha — provando que o teste diferencial morde.
- **Aceitação 2:** forma normal é estável — rodar `simplify --engine=linear` duas
  vezes é idempotente.
- **Risco:** explosão do estrato em graus altos. Mitigar com orçamento explícito
  no mesmo formato do `Checkpoint` do engine localizado: ponto único de saída,
  falha em vez de queda silenciosa.

### R4 — `ENode::Deriv` e Leibniz

Implementar §2. Nó transiente, passe de descida, invariantes I1/I2 com checador.

- **Aceitação:** `∇_c(A[a] B[b])` normaliza para a soma de dois termos, e é igual
  à soma escrita à mão.
- **Aceitação 2:** `∇_c(A[a] B[b] C[d])` dá três termos; produto com escalar
  constante mata o termo certo.
- **Controle negativo:** `∇_c(A[a]) B[b]` **não** ganha termo em `∇_c B`.
- **Controle de invariante:** teste que constrói fusão de e-classes com assinaturas
  diferentes e exige falha.
- **Risco:** é a rodada que mais mexe em parser. Considere separar em R4a (parser
  aceita e produz `Deriv`, normalização rejeita) e R4b (normalização desce).

### R5 — identidade de Ricci (comutador)

`[∇_a, ∇_b] T = ` termos de Riemann, um por slot de `T`, com sinal por variância.

**A convenção de sinal e de ordem de índices do Riemann precisa ser declarada, não
embutida** — ela já é uma escolha na metade de componentes, e as duas metades têm
que concordar ou a ponte de R7 vira gerador de falso-negativo.

- **Aceitação:** `∇^a ∇^b F[a,b] = 0` para `F` antissimétrico declarado. Exercita
  comutador, depois morte por simetria, depois coleção. Teste pequeno e bonito.
- **Aceitação 2:** `[∇_a,∇_b] f = 0` para escalar.
- **Controle negativo:** sem `F` antissimétrico declarado, não zera.

### R6 — motor de substituição

`--subst "T[a,b] -> S[a,b] + g[a,b] f"`, com variáveis de padrão sobre slots,
casamento respeitando simetria declarada, e renomeação de captura de dummies.

Isto é a feature mais pedida de qualquer CAS tensorial e hoje está inteiramente
ausente. É também o que transforma ODEROM de "verificador de identidades" em
"ferramenta de derivação".

- **Aceitação:** substituir a definição de Einstein em `G[a,b]` e recuperar
  `R[a,b] - g[a,b] R/2`.
- **Controle negativo:** substituição cujo lado esquerdo tem índice livre que não
  aparece no direito é rejeitada em tempo de parse, não em tempo de execução.
- **Risco:** captura de dummy. Renomear os dummies do lado direito para fora do
  alfabeto do alvo antes de instanciar. Teste dedicado com colisão deliberada.

### R7 — ponte índices abstratos → componentes

Avaliar uma expressão em índices abstratos sobre uma métrica/chart declarada,
produzindo componentes via a maquinaria de `oderom-components`.

Isto liga as duas metades do sistema, que hoje não se falam, e vale mais como
**estratégia de verificação** do que como feature (§8).

- **Aceitação:** `G[a,b]` avaliado em Schwarzschild dá zero componente a componente;
  em Kerr também; `R[a,b]` de Gödel dá o que a metade de componentes já dá.
- **Risco:** o custo de avaliação em Kerr. Usar o engine localizado e o mesmo
  orçamento.

### R8 — derivada de Lie e equação de Killing

`£_ξ` como head derivado, `∇_(a ξ_b) = 0`, e a consequência
`∇_a ∇_b ξ_c = R_{cbad} ξ^d`.

- **Aceitação:** derivar a última a partir da equação de Killing mais R5. É um
  resultado real de GR e usa toda a pilha.

### Meta de aceitação transversal

**`∇^a G_{ab} = 0`, derivado.** Precisa de: segunda Bianchi (existe), contração com
métrica através de derivada (metric-compatible existe), traço duplo, coleção de
termos em soma de mais de três (R1+R2), e forma normal confiável (R3). É o teste
que só passa quando o plano inteiro até R3 fechou, e é o que eu poria como
critério de "a capacidade aumentou de fato".

---

## 7. Duas correções de higiene que o relatório levantou

Não são desta linha de trabalho, mas atrapalham a verificação dela.

**Os 38 `#[ignore]`.** Você mesmo separou: sondas de medição deliberadas
(`measure_*`, `diagnostic_*`, `probe_generator_yield`) versus verificações reais
dormentes (`ricci_of_kerr_is_identically_zero`,
`kretschmann_of_kerr_through_the_real_binary`,
`ricci_scalar_of_godel_is_minus_one_over_a_squared`). A separação já está feita na
sua cabeça; falta estar no código. Sugestão: mover as sondas para um binário de
bench e as verificações reais para uma feature `slow` que roda em CI mas não no
laço de desenvolvimento. `#[ignore]` misturando as duas coisas significa que
ninguém sabe o que está desligado.

Isso importa aqui porque **R7 vai precisar exatamente dessas três verificações
como base de comparação.**

**Kerr pela CLI: 2m20s contra ~4s pela API.** O relatório diz que o laço de
`kretschmann_cmd` refaz trabalho e deixa a decisão com você. Registro que se R7 vai
usar a CLI como caminho de verificação, esse fator 35 vira custo de suíte.

---

## 8. Estratégia de verificação

Três camadas, e a terceira é a que eu acho que falta hoje.

**1. Controle negativo por axioma.** Já é sua prática — cada flag é um axioma
separado, com controle negativo em ambas as direções para cada par. Manter, e
estender a cada axioma novo (Leibniz, Ricci, substituição).

**2. Teste diferencial e-graph versus linear.** §4. Cobre o buraco de que o caminho
rápido é heurístico e vai continuar sendo.

**3. Oráculo por componentes, com métrica genérica.** R7. E aqui um cuidado que vale
escrever: **verificar uma identidade só em Schwarzschild e Kerr produz
falso-positivo.** Ambas são Ricci-plana; qualquer identidade errada que dependa de
`R[a,b] = 0` passa. O corpus de verificação precisa de:

- uma métrica com Ricci não-nulo (FRW ou de Sitter já estão na galeria);
- uma métrica **genérica** — componentes que são funções racionais arbitrárias das
  coordenadas, sem simetria nenhuma. Cara de avaliar, mas é a única que não
  esconde erro por simetria acidental.

Uma métrica genérica 4D é provavelmente pesada demais para o laço normal. Sugestão:
genérica em 2D e 3D para o laço, genérica 4D na suíte lenta. O ponto do relatório
sobre compatibilidade métrica — que ∇g=0 é identidade algébrica da construção e não
distingue métrica certa de errada — é exatamente o tipo de erro que a métrica
genérica pega e a simétrica não.

---

## 9. O que fica de fora, declaradamente

- **Anti-Leibniz / fatoração de derivadas.** Apresentação, ambígua, e não é
  correção. Se voltar, volta na extração com função de custo.
- **Projetores de Young / forma normal de monômios de Riemann à la Invar.** O
  caminho linear de §4 decide as mesmas perguntas em grau limitado, mais devagar.
  Trocar por tabelas de Young é otimização, e só faz sentido depois de R3 dar o
  número de quanto o linear custa de verdade.
- **Fechamento por Gröbner sobre produtos e derivadas.** Fora. O limite fica escrito
  na docstring.
- **FLINT e Symbolica.** Continuam fora por sua instrução, e o relatório já observa
  que Kerr deixou de ser o gatilho para reconsiderar.

---

## 10. Onde eu posso estar errado

Quatro pontos, em ordem de quanto me preocupam.

1. **A ordem R2 antes de R3 pode estar invertida.** Se R0 mostrar que os estratos
   típicos são pequenos, o engine linear deveria ser o padrão e a e-graph vira
   caminho rápido opcional — e aí R2 é trabalho para otimizar algo que raramente
   é o caminho. R0 existe para decidir isso, e eu não decidiria antes dele.

2. **"`Deriv` é transiente" pode não sobreviver a R6.** Substituição pode
   reintroduzir `Deriv` no meio do processo, se o lado direito tiver derivada de
   produto. A resposta é renormalizar depois de cada substituição, o que deve
   funcionar — mas é uma afirmação que estou fazendo sem ter olhado o código de
   `add_monomial`, e é o primeiro lugar onde o plano pode rachar.

3. **A chave de forma pode não ser discriminante o bastante** para podar de fato,
   se muitos termos do corpus têm o mesmo multiconjunto de heads. R0 mede isso de
   graça — inclua a distribuição de chaves de forma por estrato no relatório.

4. **A convenção do Riemann entre as duas metades.** Estou assumindo que
   `oderom-components` e `oderom-core` concordam na ordem de índices e no sinal.
   Se não concordarem, R5 e R7 produzem discordância que vai parecer bug de
   álgebra e é bug de convenção. **Verificar isso é barato e deve ser feito em R0,
   não em R5.**

---

## 11. Próximo passo concreto

R0. Um relatório de números, nenhuma mudança de comportamento, mais a checagem de
convenção do item 10.4. Posso escrever o prompt formal em inglês para o Claude Code
quando você quiser — mas o conteúdo de R0 é curto o bastante que talvez você prefira
pedir direto.

---

## Questão aberta para R3 — transbordo na eliminação

Registrada em R1, deliberadamente **não decidida** aqui, e sem influência sobre
o desenho de R1.

Os coeficientes da soma são `oderom_core::Scalar { num: i64, den: i64 }`. Isso
basta para R1, onde as operações são somas de coeficientes vindos de
canonicalização. Não basta para R3: eliminação gaussiana sobre ℚ faz numerador e
denominador crescerem a cada pivô, e R0 mediu estratos de até 63 colunas — o
suficiente para estourar `i64` com folga.

As saídas usuais são Bareiss livre de frações em `i128`, aritmética de precisão
arbitrária, ou modular com reconstrução racional. `oderom-expr` já tem
`BigScalar`, mas está fora de escopo até R7, e importá-lo antes disso acoplaria
as duas metades justamente na rodada em que elas ainda não se falam.

Decidir isto é pré-requisito de R3, não de R2.

---

# Achados do catálogo (48 entradas, `catalogue/tensor-identities.md`)

Quatro achados que não existiam em lugar nenhum deste plano. Registrados aqui
porque uma rodada que passa sem escrevê-los é uma rodada em que eles se perdem
entre sessões — e é assim que um plano deixa de descrever o sistema que governa.

## Achado 1 — heads de rank 0 são rejeitados, e isso bloqueia a meta do próprio plano

`head Rs :` falha com `expected an identifier, found Eof`. O escalar de Ricci não
pode ser declarado.

**Consequência local**, registrada no catálogo: `∇_a R = 2∇^b R_ab` e o traço do
tensor de Einstein não puderam ser escritos como entradas.

**Consequência maior:** a meta transversal deste plano é `∇^a G_ab = 0`. A
derivação contrai a segunda identidade de Bianchi duas vezes e **produz `∇_a R`**.
Sem escalar declarável, o critério de aceitação de R1–R3 não é enunciável hoje.

### Deliberado ou omissão: **omissão no parser, mais uma questão de design real embaixo**

Verificado, não suposto:

- `parse_head_decl` (`oderom-cli/src/parser.rs:992`) abre um `loop` que chama
  `toks.ident()?` **antes** de testar o terminador. A aridade mínima 1 é artefato
  do formato do laço — não há guarda, não há comentário justificando, não há
  mensagem de erro que mencione rank 0.
- `Registry::declare_head` (`oderom-core/src/registry.rs:89`) **não** rejeita
  `slots` vazio. Valida apenas que a aridade dos geradores bate com `slots.len()`.
  O núcleo aceitaria um head de rank 0.

Até aqui é omissão. Mas há um ponto onde rank 0 não é só destravar o parser:

- `Registry::derivative_head` (`registry.rs:145`) faz
  `base_head.slots.first().ok_or(CoreError::UnknownHead(...))` — ele deriva a
  assinatura do slot de derivada **do primeiro slot da base**, para manter
  bundle e dimensão consistentes sem inventá-los. Um head de rank 0 não tem
  primeiro slot, e `∇_a R` falharia com `UnknownHead`, que é uma mensagem
  enganosa para essa causa.

Ou seja: suportar rank 0 exige decidir **de onde vem o bundle/dimensão do índice
de derivada** quando não há slot de onde copiar. Isso é decisão de design, não
conserto de parser, e é exatamente o caso de que a meta de aceitação precisa —
`∇_a R` é derivada de um escalar.

**Merece rodada própria, e vem antes de R3.** O escopo é maior do que "o parser
tem um buraco".

## Achado 2 — axiomas não podem ser declarados

W2-07 (`∇_[a F_bc] = 0`), W2-09/W2-10 (Killing) e W2-14 (traços de Weyl) falham
por **uma** razão compartilhada que não é R4, R5 nem R6: `--bianchi`,
`--bianchi2` e `--metric-compatible` estão cada uma cabeada a uma identidade
específica de um tensor específico. Não há como dizer "este head antissimétrico
satisfaz esta identidade".

As quatro **expandiram corretamente** — antissimetrização, simetrização e
contração com a métrica fizeram seu trabalho e produziram a expressão certa não
reduzida. Falta apenas a declaração.

**Questão aberta:** isto é provavelmente **a mesma capacidade de R6
(substituição) vista do outro lado**. R6 generaliza *reescrita* para regras
declaradas pelo usuário; isto generaliza *axiomas* para identidades declaradas
pelo usuário. A especificação de R6 não deveria ser escrita antes disto ser
resolvido, ou as duas serão desenhadas duas vezes.

**Inferido, não instrumentado:** que as quatro entradas compartilhem causa única.
A leitura vem da forma das flags existentes, não de instrumentação do casador.

## Achado 3 — a metade abstrata não tem interface

Nenhuma das 48 entradas do catálogo é alcançável pelo notebook. A gramática de
consulta da sessão (`oderom-cli/src/parser.rs:434`, `parse_query`) não tem
`canon` nem `simplify`; o app cobre só a metade de componentes. **O catálogo mede
a CLI.**

A decisão que isso implica tem **duas partes**, e elas têm valores muito
diferentes:

1. **Verbo de consulta** — a gramática ganha uma produção, a sessão um tipo de
   entrada. Pequeno, e sozinho só move o terminal para dentro de uma caixa de
   texto.
2. **Declaração no documento** — heads, simetrias e o cenário abstrato precisam
   viver num bloco `.od` para o verbo ter sobre o que operar. Isto não é mais uma
   produção: é decidir **como um documento ODEROM declara um tensor abstrato**, e
   é a mesma decisão de que R6 e o Achado 2 precisam.

A segunda é a que produz informação.

Conecta com o item 8 do catálogo: blocos `.od` **não foram necessários** para as
48 entradas, e isso foi contorno, não sucesso — definições não puderam ser
expressas de forma alguma, então o formato foi **contornado, não testado**. O
catálogo ainda não é consumidor real da linguagem `.od` do lado abstrato. A
janela é o que forçaria a questão.

**Para quem ler depois:** a metade de componentes é apresentável (Kerr fecha,
Kretschmann bate com a forma fechada, exportação funciona) e a metade abstrata
está na rodada 1 de 8. Dar às duas o mesmo destaque numa janela sugeriria uma
paridade de maturidade que não existe.

## Achado 4 — a mensagem de erro de W2-01, contra R4

`(V[a] V[b]);c` emite `parse error: expected a tensor factor, found Sym('(')`. A
mensagem reclama de um parêntese; não diz que derivada de um produto não é
representável. Quando R4 chegar, o erro que ele substitui é hoje indistinguível
de um erro de digitação.

Trivial de corrigir, não corrigido. Anexado a R4.

## Também registrado

### W1-15 começa em R1b, não em R2/R3

`R[[a,b,c,d]]` com `--bianchi R` devolve `1/3 R[a,b,c,d] − 1/3 R[a,c,b,d] +
1/3 R[a,d,b,c]`. O sistema expandiu 24 termos, colapsou para três, carregou os
coeficientes racionais corretos e parou **exatamente** no casamento da
identidade. Tudo até o último passo funcionou.

O último passo não pode funcionar porque esses três termos **carregam
coeficiente**, e o coeficiente ainda mora dentro de `ENode::Term` — logo
`1/3 R[a,b,c,d]` e `R[a,b,c,d]` são e-classes diferentes e não há chave comum
sobre a qual uma identidade de k termos pudesse reconhecê-los. **R1b é
pré-requisito, não só R2/R3**, e o catálogo confirmou isso por um caminho
independente do raciocínio que originou R1.

### Vacuidade — princípio, não nota de rodapé

**Um teste que não pode falhar não é um teste.** Duas espécies, e só uma precisa
de substituição:

- **Vacuidade permanente.** A fixture de convenção em de Sitter: uma métrica
  maximamente simétrica tem `R_abcd = (1/L²)(g_ac g_bd − g_ad g_bc)`, contra a
  qual as três simetrias de slot valem por construção para *qualquer* `g`
  simétrico, e o ciclo de Bianchi cancela independentemente da convenção de ordem
  de índices. Nenhum trabalho posterior conserta isso — **precisa de outra
  fixture** (FRW, ou Kerr, cujo Weyl é rico; 2D e 3D não servem, pois lá o
  Riemann é determinado pelo escalar e pelo Ricci respectivamente).
- **Vacuidade temporária.** W1-16: passa só porque o caso positivo W1-15 também
  falha. Resolve-se sozinha quando W1-15 virar A. **Precisa de marcação, não de
  substituição** — e o catálogo carrega `VACUOUS (pending W1-15)` como campo, não
  como prosa, justamente para que ninguém leia "16 verdes" e não saiba que um é
  oco.

---

# Ordem reposicionada

1. **R1b** — representação da soma. Destrava W1-15. O catálogo confirmou por
   caminho independente que é pré-requisito, não otimização.
2. **Heads de rank 0** (Achado 1) — antes de R3, porque a meta de aceitação de R3
   precisa de `∇_a R`. Escopo maior que o parser: inclui decidir de onde vem a
   assinatura do índice de derivada de um escalar.
3. **`simplify` no notebook** (Achado 3) — depois de R1b, antes de R3. R1b muda a
   representação da soma e a saída renderizada pode mudar junto; expor antes
   significa refazer.
4. **R3** — o motor linear de decisão.
5. **Onda três do catálogo** — **depois** dos Achados 1 e 2 estarem resolvidos ou
   explicitamente adiados, senão ela produz mais entradas contra a mesma parede.

Achados 2 e 4 se anexam a rodadas existentes (R6 e R4 respectivamente) em vez de
virarem rodadas próprias.
