# ODEROM — DESIGN-M4.md (Marco 4: e-grafo e identidades multi-termo)

**Status: implementado.** Ver [README.md](README.md#marco-4-status). D4.1 e D4.2 confirmados como propostos (critério de aceitação da seção 4 tal qual escrito; regra de Bianchi específica para o head de Riemann, sem linguagem geral de identidades). Diferente dos marcos 2 e 3, esta implementação não precisou de nenhuma correção de rota no meio do caminho — os 14 testes unitários + o teste de aceitação passaram já na primeira tentativa completa.

Mesma regra dos marcos anteriores: proposta, não começo de implementação.

## 0. Uma diferença importante em relação aos marcos 2 e 3

Marco 2 tinha um critério de aceitação explícito (Kretschmann de Schwarzschild). Marco 3 também (S² com duas cartas). **O seu roteiro original não dá um critério de aceitação para o Marco 4** — só descreve o mecanismo: "e-grafo e saturação por igualdade; identidades multi-termo; extração por função de custo." Isso significa que preciso *propor* um critério concreto aqui, não só confirmar um que você já deu. É o principal motivo para eu parar antes de codar — não quero decidir sozinho o que "sucesso" significa neste marco.

## 1. Onde o e-grafo entra: nível abstrato, não escalar

O Marco 1 já tinha isso no radar — o `DESIGN.md` original dizia, sobre Bianchi: *"NÃO entram no Marco 1... não formam grupo e nenhum algoritmo de coset as resolve"*. Isso localiza o problema: identidades multi-termo (Bianchi: `R_{a[bcd]} = 0`, ou seja `R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c] = 0`) são uma relação entre *três monômios abstratos diferentes* (cada um já na forma canônica do Marco 1), não uma relação entre componentes escalares. Isso põe o e-grafo no nível de `oderom-core::Polynomial` / `oderom-canon`, não no nível de `oderom-expr::Expr` — é uma continuação natural do Marco 1, não do Marco 2/3.

## 2. Por que Bianchi não é uma regra de reescrita comum

Um e-grafo clássico (tipo `egg`) satura aplicando regras `padrão-esquerda ~ padrão-direita` (uma reescrita local, par a par). Bianchi não é isso — é uma relação *linear* entre três termos: nenhum dos três é "mais simples" que os outros dois juntos, e a identidade só faz sentido como uma restrição sobre a *soma*. A forma natural de representar isso num e-grafo é como uma regra que, ao encontrar um monômio de Riemann `R[a,b,c,d]` em qualquer lugar do grafo, também injeta o fato:

```
R[a,b,c,d]  ==  -( R[a,c,d,b] + R[a,d,b,c] )
```

ou seja, uma aresta de equivalência entre uma e-classe de um único monômio e uma e-classe de uma soma de dois outros monômios (com sinal). Isso é representável num e-grafo desde que o vocabulário de nós inclua "soma de monômios" como um tipo de nó (o e-grafo opera sobre `Polynomial`, não só sobre `Monomial`) — não é um golpe especial, é só usar o e-grafo no nível certo.

## 3. Estruturas propostas (`oderom-egraph`, crate nova)

Não vou usar a crate `egg` (dependência pesada nova, mudaria a política do projeto de novo, e a maior parte do valor de um e-grafo de propósito geral não se aplica aqui — só preciso de um caso bem mais estreito). Implemento um e-grafo pequeno, feito à mão, igual fiz com Schreier-Sims e o CAS.

```rust
pub struct EClassId(u32);

pub enum ENode {
    Term(Monomial),                    // um monômio já canonicalizado (Marco 1)
    Sum(SmallVec<[EClassId; 4]>),       // soma de e-classes (Polynomial)
    Neg(EClassId),
}

pub struct EGraph {
    union_find: ...,                    // union-find padrão, com "congruence closure": dois Sum/Neg com os mesmos filhos (após find) colapsam
    classes: FxHashMap<EClassId, Vec<ENode>>,
    hashcons: FxHashMap<ENode, EClassId>,
}
```

Operações centrais:
- `add(node) -> EClassId` (hash-consing: nó igual devolve a mesma e-classe).
- `union(a, b)` (mescla duas e-classes, reprocessa congruência).
- `apply_bianchi(riemann_head)`: para toda e-classe contendo um `Term(monomial)` cuja cabeça é Riemann, injeta a relação da seção 2 via `union`.
- Saturação: laço de ponto fixo aplicando as regras registradas até não haver mudança (ou um limite de iterações — Bianchi sozinha satura rápido porque só gera 3 termos por monômio de Riemann existente, mas dois quaisquer desses 3 podem por sua vez estar canonicamente relacionados a monômios já vistos, então preciso de um limite defensivo, tipo os `MAX_ITERS` que já uso em `oderom-expr::normalize`).
- `extract(id, cost_fn) -> Polynomial`: escolhe, recursivamente, o nó de menor custo em cada e-classe (programação dinâmica bottom-up padrão para extração em e-grafo). Função de custo default: número de monômios na soma (menor é melhor) — extrair "0" (soma vazia) é sempre o menor custo possível, então se uma e-classe está unida a zero, `extract` devolve zero.

## 4. Critério de aceitação proposto

Já que o roteiro não deu um, proponho:

> Declarar os três monômios `R[a,b,c,d]`, `R[a,c,d,b]`, `R[a,d,b,c]` (mesmos 4 índices livres, permutados), construir a soma `R[a,b,c,d] + R[a,c,d,b] + R[a,d,b,c]` como `Polynomial`, registrar a regra de Bianchi no e-grafo, saturar, e extrair — o resultado deve ser `Polynomial` vazio (zero). Sem a regra de Bianchi registrada, a mesma soma extrai para ela mesma (3 termos, já que nenhuma simetria pura de permutação do Marco 1 relaciona esses três — é exatamente por isso que Bianchi existe como identidade *adicional*).

Isso é literal, direto, e prova as três coisas que o roteiro pede ao mesmo tempo: e-grafo (union-find + congruência), saturação por igualdade (aplicar a regra até estabilizar), identidade multi-termo (Bianchi, que uma canonicalização por grupo nunca captura), e extração por função de custo (zero vence porque é o de menor custo).

## 5. Fora de escopo

Regras de reescrita além de Bianchi (segunda identidade de Bianchi, identidades de Ricci, etc.) — adiciono só Bianchi porque é o exemplo que o Marco 1 já citou nominalmente. Otimização de extração além de bottom-up ingênuo (extração ótima em e-grafo geral é NP-difícil em princípio; bottom-up guloso é o padrão até para `egg`, não vou tentar nada mais esperto). Integração com `oderom-cli` (fica para quando/se você pedir).

## 6. Questões abertas

**D4.1** — confirma o critério de aceitação da seção 4? É a peça que mais preciso de aval, já que eu mesmo propus (o roteiro não especificou).

**D4.2** — a regra de Bianchi (seção 2) precisa saber que `R` é *a* cabeça de Riemann especificamente (não é uma propriedade genérica de qualquer tensor 4-índices) — proponho registrar isso como um parâmetro explícito (`apply_bianchi(&mut egraph, riemann_head: HeadId)`), não como algo que o e-grafo descobre sozinho. Faz sentido, ou você imaginava algo mais genérico (tipo o usuário declarar identidades multi-termo arbitrárias no prelúdio)? Se for o segundo, é bem mais trabalho (uma linguagem para declarar identidades) — quero saber antes de escolher.

---

Aguardando seu ok, principalmente em D4.1 e D4.2.
