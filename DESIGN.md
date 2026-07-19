# ODEROM — DESIGN.md (Marco 1: núcleo abstrato)

Este documento descreve a arquitetura proposta para o Marco 1. Nenhum código foi escrito ainda. Contém, ao final, uma lista de decisões que prefiro confirmar com você antes de começar a implementar, especialmente as que tocam sinal e ordem — por serem irreversíveis de errar em silêncio.

## 1. Layout do workspace

```
ODEROM/
├── Cargo.toml                # workspace root, resolver = "2"
├── DESIGN.md
├── README.md                 # números de desempenho vão aqui (criterion)
├── oderom-core/               # 1.1 — representação de termos
│   └── src/
│       ├── lib.rs
│       ├── scalar.rs          # Scalar (racional)
│       ├── perm.rs            # Perm, SignedPerm — permutações + sinal
│       ├── registry.rs        # interner de nomes -> IDs; declarações
│       ├── symmetry.rs        # SymmetryGenerator, Bsgs, SymmetryGroup (Schreier–Sims)
│       ├── head.rs            # TensorHead, SlotSig, Variance
│       └── monomial.rs        # SlotId, Factor, Matching, Monomial, Polynomial
├── oderom-types/              # 1.2 — verificação de tipos
│   └── src/
│       ├── lib.rs
│       ├── domain.rs           # Domain (Everywhere por ora)
│       ├── judgment.rs        # ExprType, typecheck de Monomial/Polynomial
│       └── error.rs           # TypeError (thiserror)
├── oderom-canon/              # 1.3 — canonicalização (Butler–Portugal)
│   └── src/
│       ├── lib.rs
│       ├── word.rs             # linearização de Monomial em palavra de permutação
│       ├── coset.rs            # busca com retrocesso sobre S \ g / (D×P)
│       └── error.rs
├── oderom-cli/                # 1.4 — binário `oderom`
│   └── src/
│       ├── main.rs
│       └── parser.rs           # parser de prelude.od e de expressões
└── tests/
    ├── acceptance.rs           # tabela de aceitação da seção "Critérios"
    └── prop_canon.rs           # proptest: canon(g·x) == canon(x) a menos de sinal
```

Justificativa da separação em 4 crates: `oderom-core` não deve saber que tipos existem (ele é puramente combinatório — grafos de contração e grupos de permutação); `oderom-types` não deve saber como canonicalizar; `oderom-canon` depende de `oderom-core` mas não de `oderom-types` (canonicalização é uma operação sobre a estrutura de contração, independente de bem-tipagem — você pode querer canonicalizar antes de tipar, para cache por forma canônica, como o Marco 2 vai exigir). `oderom-cli` depende dos três.

## 2. Structs centrais

### 2.1 `oderom-core`

**Scalar** — racional próprio, sem dependência externa (nesta fase não preciso de bignum; monômios têm coeficientes pequenos):

```rust
/// Sempre reduzido: gcd(num, den) == 1, den > 0, e (num == 0 => den == 1).
pub struct Scalar { num: i64, den: i64 }
```

**Permutações e sinal** — tipo compartilhado por `SymmetryGroup` e pelo canonicalizador:

```rust
pub struct Perm(SmallVec<[u16; 12]>);       // notação de uma linha
pub struct SignedPerm { pub perm: Perm, pub sign: i8 }  // sign ∈ {+1, -1}
```

**Registry** — interner central. Toda `ManifoldId`, `BundleId`, `HeadId` é um índice num `Registry`; não existem estas entidades soltas, só através dele (evita comparação de string em qualquer caminho quente):

```rust
pub struct ManifoldId(u32);
pub struct BundleId(u32);
pub struct HeadId(u32);

pub struct Registry {
    manifolds: Vec<ManifoldDecl>,
    bundles:   Vec<BundleDecl>,
    heads:     Vec<TensorHead>,
    names:     FxHashMap<String, Interned>,  // rustc-hash
}
```

**Variância e assinatura de slot**:

```rust
pub enum Variance { Contra, Co }             // índice em cima / embaixo

pub struct SlotSig {
    pub bundle: BundleId,
    pub variance: Variance,
    pub dim: u32,                            // dimensão literal (Marco 1: sempre conhecida)
}
```

**Grupo de simetria** (geradores + BSGS memoizado na declaração):

```rust
pub struct SymmetryGenerator { pub perm: Perm, pub sign: i8 }

pub struct Bsgs {
    pub base: SmallVec<[u16; 8]>,
    pub strong_generators: Vec<SignedPerm>,
    pub transversals: Vec<FxHashMap<u16, SignedPerm>>,   // um por nível da cadeia
}

pub struct SymmetryGroup {
    pub arity: u8,
    pub generators: Vec<SymmetryGenerator>,
    pub bsgs: Bsgs,                          // calculado uma vez, via Schreier–Sims
}
```

**TensorHead**:

```rust
pub struct TensorHead {
    pub id: HeadId,
    pub name: String,
    pub slots: SmallVec<[SlotSig; 4]>,
    pub symmetry: SymmetryGroup,
}
```

Riemann: `slots = [TM*, TM*, TM*, TM*]` (quatro índices covariantes na convenção totalmente abaixado, ou a mistura que você preferir — ver decisão D1 abaixo), `symmetry = ⟨(0 1)⁻, (2 3)⁻, (0 2)(1 3)⁺⟩`. Métrica: `⟨(0 1)⁺⟩`. Levi-Civita: geradores = transposições adjacentes `(i, i+1)` cada uma com sinal `-1`, para `i` em `0..n-1`.

**O grafo de contração** — o coração do marco:

```rust
pub struct SlotId { pub factor: u16, pub slot: u8 }   // posição dentro do Monomial, não do TensorHead

pub struct Factor { pub head: HeadId }

/// Emparelhamento perfeito entre slots contraídos. Cada par é um edge
/// não-ordenado: (SlotId, SlotId) mas comparado/hasheado como conjunto.
/// NÃO existe nome de índice mudo em lugar nenhum desta struct.
pub struct Matching { pairs: Vec<(SlotId, SlotId)> }

pub struct AbstractIndex(u32);   // interna só para índices LIVRES (rótulo do usuário)

pub struct Monomial {
    pub coeff: Scalar,
    pub factors: SmallVec<[Factor; 4]>,
    pub contractions: Matching,
    pub free: Vec<(SlotId, AbstractIndex)>,
}

/// Soma de monômios. Invariante mantido pelo construtor: todos os termos
/// têm o mesmo multiset de AbstractIndex livres (verificado em oderom-types,
/// mas o invariante é documentado aqui porque a struct vive aqui).
pub struct Polynomial { pub terms: Vec<Monomial> }
```

Note que `free` guarda `AbstractIndex` — isso não contradiz "índice mudo é aresta": índices **livres** precisam de nome porque atravessam a expressão inteira (aparecem nos dois lados de uma equação, em todos os termos de uma soma). Só os mudos perdem o nome, porque a única coisa que um mudo faz é conectar dois slots — e isso já é exatamente o que `Matching` representa.

Construtores de `Monomial`/`Matching` serão privados-com-validação (`Monomial::try_new(...) -> Result<Self, CoreError>`), não campos públicos livres — ver decisão D4.

### 2.2 `oderom-types`

```rust
pub enum Domain { Everywhere }   // Marco 3 acrescenta variantes com obrigação SMT

pub struct ExprType {
    pub manifold: ManifoldId,
    pub free_signature: Vec<(AbstractIndex, SlotSig)>,
    pub domain: Domain,
}

#[derive(thiserror::Error, Debug)]
pub enum TypeError {
    #[error("slot {left_slot} de `{left_head}` é seção de {left_bundle}, não pode contrair com slot {right_slot} de `{right_head}`, seção de {right_bundle}")]
    IncompatibleContraction { /* nomeia os dois slots, os dois heads, os dois bundles */ },

    #[error("soma inválida: termo {index} tem índices livres {found:?}, esperado {expected:?}")]
    FreeIndexMismatch { /* ... */ },
}
```

Mensagens em linguagem geométrica, como pedido — nunca "type mismatch at position 2".

### 2.3 `oderom-canon`

```rust
pub struct Canonical {
    pub monomial: Monomial,   // forma canônica (fatores/slots reordenados)
    pub perm: Perm,           // permutação aplicada, para depuração/explicação na CLI
    pub sign: i8,
}

pub enum CanonResult { Zero, Value(Canonical) }

pub fn canonicalize(m: &Monomial, reg: &Registry) -> Result<CanonResult, CanonError>;
```

`Zero` cobre exatamente o caso "sinal -1 numa configuração que iguala o monômio ao seu próprio negativo" (ex.: `ε[a,b,c] T[a,b]` com `T` simétrico).

O algoritmo (Butler–Portugal — Butler 1991, LNCS 559; Portugal, J. Phys. A 32 (1999) 7779) busca o representante mínimo de `S \ g / P'` sob uma ordem total nas permutações, onde `g` é a linearização do monômio (uma palavra que lista, em ordem canônica de fatores, o destino de cada slot) e `P'` é o grupo que fixa essa linearização — ver decisão D2 sobre o que exatamente entra em `P'` dado que os mudos já são arestas.

## 3. Decisões de representação

**D1 — mudo é aresta, não nome (dado por você; documentando a consequência).**
`Matching` é um conjunto de pares não-ordenados de `SlotId`. Não existe, em lugar nenhum do código, uma função que renomeia um índice mudo — se eu me pegar escrevendo isso, é sinal de bug de desenho, conforme instruído. Consequência prática: o grupo `D` da literatura clássica de Butler-Portugal (que existe para quocientar a liberdade de *nomear* mudos) não atua sobre `Monomial` — ele só reaparece, empobrecido, dentro de `oderom-canon::word`, como a liberdade de orientar cada aresta (qual dos dois slots é listado primeiro) ao linearizar o grafo numa palavra para a busca em BSGS. Isso é um artefato do algoritmo de busca, não da estrutura de dados pública. Ver questão aberta Q2.

**D2 — `Matching` armazenado como `Vec<(SlotId, SlotId)>` mas com igualdade/hash por conjunto de pares não-ordenados.**
Alternativa seria um `Vec<SlotId>` indexado por posição com uma involução (like um "pareamento por array", comum em implementações de perfect matching). Prefiro pares explícitos porque o número de contrações por monômio é pequeno (tipicamente < 10) e a legibilidade/depuração importa mais que a constante de tempo aqui.

**D3 — `AbstractIndex` existe só para índices livres.**
Índices mudos não recebem `AbstractIndex` em nenhum estágio, nem mesmo como um id interno transitório durante o parsing — o parser da CLI (1.4) já deve emitir `Matching` diretamente ao ler um par de ocorrências repetidas num monômio, e descartar o nome digitado pelo usuário assim que a aresta é formada.

**D4 — construtores validados, não campos públicos.**
O rascunho no seu prompt mostra `Monomial` com campos `pub`. Vou implementar como campos privados com construtor validado (`Monomial::try_new`) e getters, porque a struct tem invariantes não triviais (matching é um emparelhamento perfeito só sobre os slots de fato contraídos; toda `SlotId` referenciada existe; um slot aparece no máximo uma vez entre `contractions` e `free`) que preciso impedir de violar via mutação direta de campo. Se você prefere campos públicos com invariantes garantidos só "por convenção", me diga — mas dado o aviso sobre erro de sinal enterrado, prefiro impedir o estado inválido de existir.

**D5 (menor) — CLI sem dependências novas.**
`clap` usaria macro derive ou ao menos uma API builder relativamente pesada; para uma única subcomanda (`oderom canon "<expr>"`) mais leitura de `prelude.od`, vou escrever um parser recursivo-descendente à mão em `oderom-cli::parser`, sem pedir dependência nova. Se no meio do caminho isso parecer errado, aviso antes de puxar algo como `clap` ou `pest`.

## 4. Questões que quero confirmar antes de implementar 1.3

Essas são precisamente do tipo que você pediu para eu perguntar em vez de chutar.

**Q1 — ordem total sobre permutações.**
Pretendo usar a convenção padrão de Portugal: fixar a base da cadeia estabilizadora (do BSGS combinado de `S` e da ordem de fatores) e comparar candidatos da dupla classe lateral por ordem lexicográfica da imagem da base. Isso é o que a literatura usa, mas quero seu ok explícito antes de gravar isso como "a" definição de forma canônica — trocar essa convenção depois invalida qualquer forma canônica já cacheada (relevante para o Marco 2).

**Q2 — papel exato de D dado D1.**
Como descrito na decisão D1: acho que `D` sobrevive apenas como a liberdade de orientação de cada aresta (`2^k` para `k` pares contraídos) dentro da linearização usada pela busca, e não como o grupo cheio de renomeação `Z₂ wr S_k` da formulação clássica — porque já não há k "nomes" para permutar entre si, só um conjunto de arestas. Meu plano é implementar exatamente essa versão reduzida. Quero confirmação de que esse raciocínio está certo antes de codificar, porque é fácil errar por excesso (buscar sobre um grupo maior que o necessário — correto mas lento) ou por falta (buscar sobre um grupo menor que o necessário — errado, produz falso-positivo de forma canônica diferente para tensores iguais).

**Q3 — convenção de variância nos slots do Riemann/métrica declarados no prelude default.**
Vou declarar `R` com os 4 slots covariantes (`T*M` em todos, i.e., `R_{abcd}`) e a métrica com 2 slots covariantes, já que Marco 1 não tem levantamento/abaixamento de índice (isso é componente, Marco 2). Isso é consistente com os exemplos de teste do enunciado (`R[a,b,c,d] R[c,d,a,b]`, `R[a,b,c,d] g[a,c] g[b,d]`)? Presumo que sim mas confirmo antes de fixar no `prelude.od` default.

## 5. O que não muda em relação ao que você já especificou

Marcado aqui só para deixar claro que não estou reabrindo: assinatura de `Monomial`/`Factor`/`Matching`/`free` (D1–D3 só refinam, não contradizem); Bianchi fora do Marco 1 com testes `#[ignore]`; `Scalar` só racional; sem SMT, sem cartas, sem componentes; sem dependências além de `thiserror`, `smallvec`, `rustc-hash`, `proptest`, `criterion`.

---

Aguardando seu ok (e respostas a Q1–Q3) antes de começar a implementação.
