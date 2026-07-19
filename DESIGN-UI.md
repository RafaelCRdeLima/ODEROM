# ODEROM — DESIGN-UI.md (UI: exploração simbólica de Geometria Diferencial)

**Status: Camada A (biblioteca) implementada, com três ajustes seus sobre
o que eu tinha proposto na seção 2 original — ver seção 5. A parte de CLI
(subcomandos novos, extensão de `prelude.od`) ainda não foi feita: era o
item mais aberto (D-UI.3) e ainda não foi confirmado, então não chutei um
formato de arquivo.** GUI/kernel Jupyter permanecem fora de escopo, por
instrução explícita — a existência do alvo `json` desde já é justamente
para não fechar essa porta.

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

Aguardando seu ok em D-UI.3 (ou outra prioridade) para a parte de CLI.
