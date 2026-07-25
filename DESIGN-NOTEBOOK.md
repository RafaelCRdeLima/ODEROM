# ODEROM — DESIGN-NOTEBOOK.md (Etapa 3: caderno de blocos)

Substitui a proposta original de dois painéis (DESIGN-UI-SESSION.md,
que continua valendo para tudo abaixo do frontend: `Session`,
`Document`, `DefFingerprint`, obsolescência por nome, cache LRU, troca
atômica do `Model`, `CancelToken`, `:timeout`, `parse_query`/
`parse_model`, `Render`/`Target::Latex`). Nada abaixo desta linha muda
qualquer uma dessas peças — a janela é uma segunda casca sobre a mesma
máquina que o REPL usa, exatamente como a Etapa 2 já era.

Fundido: o que era Etapa 4 (obsolescência visível, progresso, cancelar)
entra no escopo de v1 do caderno. Entregue em dois incrementos revisáveis:

- **3a** — `oderom-notebook` completo e testado sem janela (blocos,
  classificação, concatenação, atribuição de erro, os três estados de
  declaração, salvar/abrir), mais a casca Tauri exibindo e executando
  blocos com KaTeX. Sem obsolescência visual, sem progresso, sem cancelar.
- **3b** — os três estados na tela, obsolescência visível, progresso,
  cancelar.

## 1. Interação

Documento é uma sequência vertical de blocos. Dentro de um bloco: Enter
insere linha, Shift+Enter executa o bloco. A saída aparece logo abaixo,
tipografada. Blocos não são reordenáveis em v1 — só criar, editar,
executar, apagar.

Não há painel de definições separado e nenhum modo escolhido pelo
usuário. `oderom_cli::parser::classify_block` (novo, mas reaproveitando
o mesmo lexer/`TokStream` de `parse_model`/`parse_query` — nunca uma
segunda gramática) olha a palavra-chave líder do bloco: as seis de
declaração (`manifold`/`bundle`/`head`/`chart`/`metric`/`connection`) e
as cinco de consulta (`christoffel`/`riemann`/`ricci`/`scalar`/
`kretschmann`) são conjuntos disjuntos, então o primeiro identificador
decide sem ambiguidade. Um bloco cujo primeiro token não é nenhuma das
dez é um terceiro caso, visível como tal (`BlockOutput::Unrecognized`),
nunca um erro silencioso.

Entrada é texto (LaTeX ou ASCII, o parser já aceita os dois — nenhum
trabalho novo aí), realçada por CodeMirror. Saída é tipografada por
KaTeX a partir de `Target::Latex`, que já existe. Sem edição dentro de
fórmula renderizada. Prévia ao vivo do bloco em edição: fora de v1,
estruturalmente compatível depois (mesmo `Target::Latex`, chamado sobre
o texto ainda não executado).

## 2. O Model é uma unidade reconstruída, não uma acumulação

A garantia que blocos quebram, e a resposta:

> O Model é derivado do texto atual de TODOS os blocos de declaração, em
> ordem de documento, reconstruído como uma unidade sempre que qualquer
> bloco de declaração é executado.

Shift+Enter num bloco de declaração concatena o texto atual de todo
bloco que `classify_block` reconhece como declaração (nessa ordem) e
chama `session.evaluate_definitions(concatenado)` — a mesma chamada que
`:reload` já faz. Nenhuma API nova em `oderom-session` para isto.

**Consequência aceita, escrita aqui para não virar surpresa**: um único
bloco de declaração quebrado bloqueia a reavaliação de todos os outros,
mesmo os que não têm nada a ver com ele. Não há "a maioria funcionando,
um rascunho quebrado do lado" até ele ser corrigido — é a mesma
atomicidade que `:reload` sempre teve (um arquivo inteiro falha ou
passa), só que blocos tornam mais fácil esbarrar nisso sem querer, já
que cada declaração agora é editável isoladamente.

Um bloco de consulta enxerga o Document atual inteiro, não só as
declarações acima dele no caderno — um `Document` só, sem escopo por
posição (a mesma regra do REPL).

Apagar um bloco de declaração NÃO dispara reconstrução na hora — a
definição só some do Model na próxima vez que qualquer bloco de
declaração for executado (a concatenação seguinte simplesmente não
inclui mais o texto apagado). "Nada recalcula sozinho" continua valendo
para apagar também.

## 3. Três estados para bloco de declaração

```rust
pub enum DeclarationStatus {
    Confirmed,   // o texto ATUAL deste bloco está no Model vigente
    Divergent,   // editado desde a última reconstrução bem-sucedida
    Error(String), // é a causa atribuível da última falha
}
```

Transições:

- Editar o texto de um bloco que classifica como declaração → `Divergent`
  imediatamente (não espera Shift+Enter) — mesma disciplina da
  obsolescência de consultas, aplicada às declarações.
- Reconstrução com sucesso → **todo** bloco de declaração vira
  `Confirmed`, não só o executado — o Model é um só, a apresentação tem
  que concordar com ele por inteiro.
- Reconstrução com falha → o bloco culpado vira `Error`; os demais
  mantêm o que tinham. Um bloco que nunca participou de uma
  reconstrução (recém-criado, ou que só agora passou a classificar como
  declaração) resolve para `Divergent`, nunca fica preso num estado
  "nunca executado" à parte — não há um quarto estado.

Bloco de consulta não usa esses três estados: seu status vem de
`EntryState` (`Pending`/`Running`/`Done`/`Stale`/`Failed`) via o
`EntryId` que ele guarda, pelo mecanismo de `DefFingerprint` que já
existe, e o bloco só mantém uma referência para esse `EntryId`.

## 4. Atribuição de erro na reconstrução

Um único `CliError` cobre a concatenação inteira. Dois casos:

**Com posição** (`CliError::Parse{position: Some(p), ..}`): a
concatenação é construída rastreando o intervalo de linhas de cada
bloco; `p.line` mapeia direto para o bloco dono. O(1), exato.

**Sem posição** (`CoreError::DuplicateName` e qualquer outro erro
pós-parse — não têm `Position` porque acontecem depois do parse, na
construção do `Model`): busca incremental por prefixo. Reparseia
`parse_model` sobre concatenações crescentes dos blocos de declaração,
na mesma ordem, e para no primeiro prefixo que falha — esse bloco é
exatamente o que fez a adição parar de funcionar. Para
`DuplicateName(nome)` isso dá, sem precisar extrair o nome do erro nem
tratá-lo como caso especial, exatamente "o segundo bloco que declara
aquele nome" (o primeiro já estava no prefixo que funcionava). O mesmo
mecanismo, sem mudança nenhuma, atribui qualquer outro erro pós-parse
com a mesma precisão — generaliza a regra pedida em vez de precisar de
um caso por tipo de erro. Custo: até N chamadas a `parse_model` (uma
por bloco de declaração), cada uma ~1ms — o mesmo orçamento "barato" já
aceito para a reconstrução em si.

Se a busca incremental não encontrar nenhum prefixo que falhe (não
deveria acontecer — o texto inteiro já falhou, então algum prefixo
tem que falhar também, o mais tardar o último) → fallback defensivo: o
bloco que o usuário acabou de executar. Nunca deveria disparar na
prática; existe para nunca deixar a atribuição sem resposta nenhuma.

## 5. Fluxo de execução

**Bloco de declaração**: síncrono. `evaluate_definitions` é parse +
construção de `Model`, sem `ExecutionContext`, sem progresso — nenhuma
curvatura envolvida.

**Bloco de consulta**: reexecutar com texto editado chama
`session.run_entry(...)` de novo — gera um `EntryId` novo a cada vez,
nunca reaproveita o antigo (o texto pode ter mudado). Antes de criar o
novo, remove o antigo (item novo abaixo).

> **Revisto na Etapa 3b** (seção 9): editar (sem executar) um bloco que
> já tinha resultado *não* remove mais esse resultado. Esta seção
> descrevia a política original — "mostrar uma resposta antiga ao lado
> de um texto que já não é aquele é o mesmo tipo de estado invisível
> que a obsolescência por dependência já existe para evitar" — mas ela
> tinha o problema oposto do que tentava resolver: apagar o resultado
> também é um jeito de mentir por omissão, escondendo algo que
> continuava verdadeiro um instante atrás. A Etapa 3b substitui "apagar
> ao editar" por "marcar como obsoleto ao editar", mantendo o resultado
> visível.

## 6. `Session::remove_entry` — nova API, pequena

Reexecutar (ou editar, ou apagar) um bloco de consulta no caderno é o
gesto principal, não uma exceção como era no REPL — cada execução sem
remoção deixaria uma entrada permanente em `session.entries()`, que a
varredura de obsolescência (seção 3 de DESIGN-UI-SESSION.md) percorre
inteira a cada reconstrução. Sem remoção, isso degrada com o tempo de
sessão.

```rust
// oderom-session/src/session.rs
pub fn remove_entry(&mut self, id: EntryId) {
    self.entries.retain(|e| e.id != id);
}
```

Testado em `oderom-session` diretamente (não só via o caderno):
reexecutar o mesmo bloco 100 vezes deixa `session.entries().len()`
constante.

## 7. Salvar/abrir

Formato texto simples, um delimitador por linha entre blocos: uma
linha cujo conteúdo (após remover `\r` final, para CRLF) é exatamente
`%%`. N blocos, N-1 delimitadores; o primeiro bloco não tem delimitador
antes. Seguro porque comentário de `.od` é `#`, nunca `%%`. Texto ganha
de JSON porque diffa bem no git.

O gravador recusa salvar (erro, não corrupção silenciosa) se o texto de
algum bloco contiver uma linha que já seja exatamente `%%` — checagem
antes de escrever, não depois.

Abrir nunca executa nada (`nada recalcula sozinho` de novo) e nunca
persiste saída — só os textos dos blocos, cada um começando como se
tivesse acabado de ser criado (`NeverRun`); o tipo (declaração/consulta)
é sempre redescoberto por `classify_block`, nunca guardado no arquivo,
para nunca poder ficar dessincronizado do conteúdo real.

## 8. Onde o código mora

`oderom-notebook`, nova crate, mesma relação com `oderom-session` que
`oderom-repl` já tem: toda a lógica (`Notebook`, `Block`,
`BlockOutput`, concatenação, atribuição de erro, salvar/abrir) testável
por script, sem abrir janela — mesmo critério de aceite de toda etapa
anterior. A casca Tauri (fininha) só chama essa crate via comandos IPC;
nenhuma lógica de geometria, álgebra ou renderização matemática mora
na casca.

Único ponto tocado em `oderom-cli`/`oderom-session` fora da seção 6:
`classify_block` novo em `oderom-cli/src/parser.rs` (reaproveitando o
lexer existente, nenhuma gramática nova).

## 9. Obsolescência visível (Etapa 3b)

Um bloco já executado continua mostrando seu resultado antigo para
sempre, mesmo depois de editado — a única coisa que muda é se esse
resultado é exibido como atual ou como obsoleto. Marcar como obsoleto
nunca recalcula, nunca limpa a saída, nunca muda o valor exibido:
puramente informativo, o mesmo invariante "nada recalcula sozinho" de
sempre.

**O que torna um bloco obsoleto** — duas regras independentes, cada uma
suficiente sozinha:

1. **Edição própria**: o texto atual do bloco não é mais igual ao texto
   que estava nele da última vez que *ele mesmo* foi o alvo direto de
   um Shift/Ctrl/Alt-Enter. Comparação ao vivo (`Block::is_obsolete`),
   nunca um sinalizador que só liga — desfazer a edição de volta ao
   texto executado limpa a marca sozinho, sem nenhum caso especial.
2. **Cascata conservadora**: quando um bloco é editado (com o texto
   realmente mudando), apagado, ou um bloco novo é inserido depois
   dele, todo bloco *abaixo* que já foi diretamente executado alguma
   vez é marcado obsoleto — independentemente de realmente depender ou
   não do que mudou.

A regra 2 é deliberadamente grosseira, e a razão foi verificada antes
de implementar, não assumida: `oderom-session` já rastreia dependência
fina por símbolo para blocos de consulta (`Entry::used`, comparado
contra `changed_or_removed_names` dentro de `Session::evaluate_definitions`),
mas essa informação só é recalculada quando uma reconstrução
*realmente roda* — ou seja, só na execução explícita de algum bloco de
declaração, nunca a partir de uma edição isolada. Como "nada recalcula
sozinho" proíbe editar/apagar/inserir de disparar reconstrução, não há
informação de dependência mais fresca disponível no momento em que a
cascata precisa decidir. `oderom-notebook` em si não expõe nenhum grafo
de dependência bloco-a-bloco (blocos são posições sobre um `Session`
compartilhado, não nós com arestas rastreadas). A regra conservadora
("marca tudo abaixo que já rodou") é portanto o único mecanismo
disponível sem inventar um analisador de dependências novo — erra para
o lado de avisar demais, que é o lado seguro.

A cascata é um sinalizador de verdade (`Block::obsolete_by_cascade`),
não uma comparação — mas só existe UMA forma de desligá-lo: executar o
bloco de novo, diretamente (`Notebook::execute_block` zera
`obsolete_by_cascade` só do bloco que foi o alvo direto, nunca de
blocos que uma reconstrução varre como efeito colateral). Um bloco
abaixo de uma edição continua marcado até ser reexecutado
*individualmente* — mesmo que a reconstrução de outro bloco o
reconfirme como parte do `Model` (`DeclarationStatus::Confirmed` e
"obsoleto" são sinais independentes agora: o primeiro descreve se o
texto do bloco está refletido no `Model` vivo; o segundo descreve se o
que está na tela ainda corresponde ao que se veria se tudo fosse
reexecutado agora).

**Aparência**: três estados na gutter — `[ ]` nunca executado, `[n]`
executado e atual, `[n]` com marcação âmbar executado e obsoleto.
Deliberadamente não vermelho (vermelho já significa erro de parse) e
deliberadamente não só cor (também uma borda/traço lateral, para
funcionar sem depender de percepção de cor) — decisão de exibição pura,
vive inteira em `oderom-app/dist/notebook.css`/`notebook.js`; o Rust só
expõe o booleano `obsolete` no DTO (`BlockDto`), nunca decide como ele
aparece na tela.

## 10. Cancelamento (Etapa 3b, segunda parte)

O problema: um bloco de consulta podia entrar em cálculo longo (ou
nunca terminar sozinho — a métrica não-recíproca já documentada em
`oderom-session/tests/cancellation.rs` é o caso real que expôs isto ao
vivo) e, até esta rodada, `execute_block` rodava a consulta
sincronamente segurando o `Mutex<Notebook>` do `oderom-app` pela
duração inteira — travando a janela inteira, não só aquele bloco,
porque todo outro comando (`list_blocks`, `edit_block`, ...) precisa do
mesmo `Mutex`.

### 10.1 A porta que o REPL já tinha aberto

O padrão de solução não foi inventado agora: `oderom-repl` já resolve
exatamente este problema desde a Etapa 2, com um `ExecutionContext`
cancelável (`Arc<ExecutionContext>`) rodando a computação numa thread
própria enquanto a thread principal fica livre para reagir a um
Ctrl+C. Este trabalho reaproveita esse padrão, não inventa um segundo:
`oderom-session::PendingQuery`/`Session::begin_query`/`finish_query`
são a versão genérica, sem REPL nem Tauri, do mesmo split — `run_query`
(a função que o REPL já chamava) não mudou.

### 10.2 O canal para progresso — desenhado agora, não implementado

A rodada anterior pediu explicitamente que o canal entre a thread de
computação e a interface já suportasse mensagens de estágio, mesmo sem
exibi-las ainda. Não foi preciso desenhar um canal novo:
`ExecutionContext` já tinha um campo interno `stage: Mutex<String>`,
atualizado internamente a cada estágio (`inverting the metric...`,
`computing Christoffel symbols...`, os mesmos textos que já aparecem no
stderr do REPL) e lido por um método que só precisou virar `pub`
(`ExecutionContext::current() -> String`). `Notebook::running:
HashMap<BlockId, Arc<ExecutionContext>>` — que já precisa existir para
o cancelamento poder achar o `ExecutionContext` certo — é exatamente o
mesmo handle que uma rodada futura leria para mostrar o estágio atual.
Nenhuma tubulação nova: só um campo de DTO e uma linha de frontend
ficam para depois.

### 10.3 `BlockOutput::Attempt` — em execução ou cancelado

Um bloco cuja tentativa mais recente está rodando ou foi cancelada
mostra `BlockOutput::Attempt { attempt: EntryId, previous:
Option<EntryId> }`, não `Query(EntryId)` — `attempt`'s próprio
`EntryState` (`Running` ou `Cancelled`, a nova variante) diz qual dos
dois é; `previous`, se `Some`, é a entrada de uma execução anterior bem
sucedida, mantida viva (nunca removida) especificamente para o
resultado antigo continuar na tela, obsoleto, em vez de sumir no
instante em que uma reexecução começa — a mesma lógica que a
obsolescência por edição já usa (seção 9), agora estendida ao
cancelamento: "não apague saída por causa de cancelamento" na letra.

Assim que a tentativa termina *sem* ser cancelada (`Done` ou `Failed`),
o bloco volta para `Query(attempt)` puro e `previous`, se havia, é
finalmente removido — exatamente como qualquer reexecução não-cancelada
sempre fez. Só uma tentativa genuinamente cancelada deixa o bloco preso
em `Attempt` para sempre, até ser reexecutado.

### 10.4 Por que a sessão nunca fica inconsistente — decisão registrada, não implícita

A pergunta foi feita explicitamente: se uma execução cancelada já tinha
começado a alterar o estado da sessão, o que acontece? Resposta: **nada
precisa ser revertido, porque nada parcial é escrito.**

- O único estado que `run_query` populariza *durante* a computação é o
  `ComputeCache` — e `LruCache::get_or_try_insert_with` só insere depois
  que `compute()` retorna `Ok`; um cancelamento é um `panic` capturado
  por `oderom_expr::run_cancellable`, então nunca alcança o `insert`.
  Isto já era verdade antes desta rodada e já era testado
  (`oderom-session`'s `mid_flight_cancellation_leaves_no_partial_cache_entry`,
  Etapa 2) — não uma garantia nova, uma que este trabalho depende de
  continuar valendo.
- `entries`/`document` só são alterados *depois* que o resultado (ou a
  notícia do cancelamento) já existe, nunca durante.

A única coisa genuinamente nova que este cancelamento introduz é o
`ComputeCache` viajar para fora do `Session` e voltar
(`Session::begin_query` faz `mem::take`; `finish_query` devolve) — o
que abre uma janela onde uma SEGUNDA consulta, se disparada enquanto a
primeira ainda está fora, encontra um cache temporariamente vazio.
Decisão registrada: isto é aceito como está. O resultado nunca fica
errado (um cache miss só custa recomputar aquele estágio do zero, o
mesmo custo de uma sessão nova), só potencialmente mais lento — e o
padrão de uso real do caderno (no máximo uma execução longa por vez,
tipicamente) raramente encontra essa janela na prática. Não foi
construído nenhum bloqueio adicional para fechá-la porque o custo
(complexidade, mais um lugar para travar) não se paga contra um caso
que já é seguro, só ocasionalmente subótimo.

**Obsoleta a partir da seção 10.8.** O parágrafo acima descreve um
risco que só existia porque uma segunda consulta *podia* ser disparada
enquanto a primeira ainda estava fora do `Session`. A exclusão mútua
global da seção 10.8 torna essa premissa falsa: `begin_execute` recusa
qualquer segunda execução enquanto uma primeira está em voo, então
nenhuma segunda consulta jamais encontra o `ComputeCache` fora —
não porque a janela tenha sido fechada por um lock novo, mas porque a
situação que a abria não pode mais acontecer. O raciocínio acima
(por que um cache miss nunca produz um resultado errado) continua
correto e vale a pena manter como registro, mas não descreve mais
nada que possa realmente ocorrer neste programa.

### 10.5 Uma tentativa por vez, por bloco (substituída — ver 10.8)

Esta seção descrevia a primeira versão do cancelamento, entregue nesta
mesma rodada antes da exclusão mútua: `Notebook::running` era um
`HashMap<BlockId, Arc<ExecutionContext>>`, e `begin_execute` só recusava
(retornando `Done` sem tocar `execution_count`) uma segunda execução do
*mesmo* bloco (`self.running.contains_key(&id)`) — dois blocos
*diferentes* podiam rodar em paralelo sem qualquer impedimento.
Verificado ao vivo logo em seguida (uma sonda descartável confirmou
duas `JoinHandle`s genuinamente simultâneas) e substituído pela decisão
da seção 10.8 antes de qualquer versão deste comportamento chegar ao
usuário como padrão assumido — nunca foi anunciado como definitivo.
Mantido aqui apenas como registro histórico do que existiu entre as
duas rodadas.

### 10.6 O canal de atualização no frontend — e uma lição de concorrência

`execute_block` agora retorna assim que a computação é despachada para
sua própria thread. `pollUntilSettled` (`oderom-app/dist/notebook.js`)
é quem observa esse bloco especificamente até ele assentar,
atualizando-o *no lugar* (`updateBlockChrome`) sem nunca chamar
`refresh()` (que reconstrói todos os editores da página e derrubaria o
cursor/foco de qualquer outro bloco em uso).

Dois problemas de concorrência genuínos apareceram testando isto pela
janela real, não hipotéticos:

1. Duas chamadas `invoke("list_blocks")` não têm garantia de ordem de
   resposta entre si. Uma resposta antiga (de uma execução anterior do
   mesmo bloco, ainda "presa" numa chamada em voo) pode chegar *depois*
   de uma resposta mais nova, sobrescrevendo um estado já correto
   (ex.: "cancelado") de volta para um estado antigo ("executando").
   `lastAppliedExecutionCount` (por `BlockId`) resolve isto comparando
   `execution_count` — um número real, monotônico, atribuído pelo
   backend (`Notebook::begin_execute`) — nunca a ordem de chegada das
   promises em si.
2. `pollUntilSettled` roda solto (fire-and-forget: nada mais espera por
   ele). Uma exceção não tratada dentro do laço o mata silenciosamente,
   no meio, sem terminar de assentar — deixando a tela presa no último
   estado renderizado, mesmo que o backend já tenha avançado. O laço
   inteiro agora vive dentro de um `try/catch` que trata qualquer
   exceção como "ainda não pronto, tentar de novo" — o mesmo padrão que
   `waitFor` (`keytest.js`) já usava.

### 10.7 Critério de aceitação — sem `sleep` artificial

A suíte real (`oderom-app/src-tauri/tests/keymap.rs`/`dist/keytest.js`)
reusa a métrica não-recíproca de
`oderom-session/tests/cancellation.rs` — a mesma que travou ao vivo —
em vez de um `sleep` artificial: cancelar algo que genuinamente nunca
terminaria sozinho é a única forma de provar que o cancelamento
funciona, não que um temporizador expirou.

### 10.8 Exclusão mútua global entre blocos — decisão deliberada

Depois que 10.1–10.7 entregaram cancelamento, uma pergunta separada
foi feita explicitamente e respondida com uma sonda descartável (não
hipótese, comportamento real medido): hoje, um segundo bloco *diferente*
pode começar a executar enquanto um primeiro ainda roda? Resposta:
sim — duas `JoinHandle`s genuinamente simultâneas, confirmadas ao
vivo. A pergunta seguinte, também explícita, foi o que fazer sobre
isso, com três respostas possíveis: deixar paralelo (e documentar como
tal), enfileirar, ou bloquear. **Decisão: bloquear — não é um
comportamento herdado de como o `PendingQuery` foi dividido, é uma
escolha deliberada, tomada depois de considerar as outras duas.**

**Por que paralelo foi rejeitado.** Não é o `ComputeCache` — perder
reaproveitamento de cache é o dano menor, e a seção 10.4 já mostrava
que um cache miss nunca produz um resultado errado. O problema real é
a `Session` compartilhada: se uma declaração e uma consulta que
depende dela rodam ao mesmo tempo, o resultado da consulta passa a
depender de qual thread o escalonador do SO decide rodar primeiro —
mesmo caderno, mesmas teclas, resultados diferentes entre uma execução
e outra. Não é uma data race de memória (nada aqui usa `unsafe`, e o
workspace inteiro roda sob `#![forbid(unsafe_code)]`) — é
não-determinismo semântico, e é pior de rastrear que uma data race
porque nada trava para revelar o problema. Além disso, execução
paralela quebra diretamente o que a seção 9 (obsolescência) construiu:
obsolescência assume "o estado da sessão no instante em que este bloco
executou" — com execuções sobrepostas esse instante vira um intervalo
que pode conter mudanças de outra thread, e a numeração `[n]` da
gutter deixa de indicar ordem de execução se as conclusões chegam fora
de ordem.

**Por que fila foi rejeitada.** Enfileirar evita o não-determinismo,
mas cria um estado novo — "bloco esperando para executar" — sem
resposta boa para perguntas que apareceriam de imediato: o que fazer
se o usuário edita um bloco enfileirado antes de sua vez chegar? E se
ele apaga esse bloco? Mais fundamental: uma fila é o mais perto de
"algo executa sem eu ter mandado *naquele momento*" que este projeto
deveria chegar — o princípio "nada recalcula sozinho" (seção 2 e
em toda parte deste documento) é sobre o usuário sempre saber
exatamente por que algo está rodando, e uma execução que só começa
minutos depois porque finalmente chegou sua vez na fila enfraquece
essa garantia.

**Por que bloquear só ficou aceitável agora.** Bloquear sem
cancelamento seria pior que os dois problemas acima: o usuário ficaria
genuinamente preso atrás de uma computação longa, sem saída. Só depois
que a seção 10 inteira (cancelamento) existiu é que bloquear virou uma
opção razoável — o usuário nunca fica preso, sempre pode cancelar o
que está rodando e liberar o resto imediatamente.

**O requisito não-negociável: bloqueado não pode ser silencioso.** Uma
tecla que não faz nada e não explica por quê é exatamente a classe de
bug que motivou este trabalho no primeiro lugar (a janela travada sem
aviso do início desta rodada).

A primeira versão desta seção tentou satisfazer isto só com a barra de
status (`renderStatusBar` em `dist/notebook.js`) sempre refletindo, ao
vivo, se algum bloco está rodando. **Corrigido depois de revisão:**
isso não basta. A mensagem da barra é *ambiente* — já estava na tela,
sem mudar, antes da tecla recusada ser apertada (essa é a própria razão
da recusa), então ela não tem nenhuma borda temporal perceptível no
instante exato da tecla. Do ponto de vista de quem está na frente da
tela, isso é indistinguível de a tecla simplesmente não ter feito nada
— o mesmo bug do primeiro dia, com outra roupa.

O reconhecimento real é um evento separado, com aresta temporal
própria, sobreposto à mensagem ambiente (nunca no lugar dela):
`flashRefusal` (`dist/notebook.js`) dá um halo breve (700ms,
`box-shadow`, nunca desloca layout) na barra de status *e* no bloco que
está de fato ocupando a execução (o mesmo `by: BlockId` que
`execute_block` já retorna em `Blocked{by}`) — o destaque no bloco tem
o efeito colateral bom de responder "quem está bloqueando" na hora,
sem precisar ler o texto. Um contador monotônico
(`refusalPulseSeq`, gravado em `dataset.refusalPulse` nos dois
elementos) é o sinal confiável e independente de tempo que qualquer
coisa — inclusive o teste automatizado — pode comparar antes/depois do
keydown; a duração exata da animação CSS é só cosmética, nada depende
dela. Retrigável mesmo se o halo anterior ainda não tiver terminado de
apagar (`restartFlash` força um reflow entre remover e readicionar a
classe), então recusas repetidas nunca ficam "absorvidas" em silêncio.
Deliberadamente nunca um diálogo modal, nunca rouba foco.

**Implementação.** `Notebook.running` deixou de ser
`HashMap<BlockId, Arc<ExecutionContext>>` (por bloco) e virou
`Option<(BlockId, Arc<ExecutionContext>)>` — global, no máximo uma
execução em voo em todo o caderno, nunca duas ao mesmo tempo em blocos
diferentes. `BeginExecution` ganhou a variante `Blocked { by: BlockId
}`, retornada por `begin_execute` sempre que `self.running` já é
`Some`, seja por causa de *outro* bloco ou do mesmo bloco de novo
(unificado — não há mais um caso especial para reentrada no mesmo
bloco como a versão anterior desta seção, 10.5, tinha). A checagem
acontece antes de `classify_block` despachar entre consulta e
declaração, então declarações são bloqueadas exatamente como
consultas — o cenário que motivou tudo isto (uma declaração
concorrendo com uma consulta que depende dela) é coberto sem caso
especial. Um bloco recusado fica inteiramente intocado: sem bump de
`execution_count`, sem `output` tocado, sem saída anterior perdida.
`edit_block` nunca consulta `running` — editar e focar continuam
funcionando normalmente em qualquer bloco, inclusive o que está
rodando, o tempo todo em que algo executa: bloquear execução não pode
bloquear a interface.

Critério de aceitação (`oderom-app/src-tauri/tests/keymap.rs`, mesma
disciplina da seção 10.7 — sem `sleep` artificial, a métrica
não-recíproca real): com um bloco longo rodando, Shift+Enter em outro
bloco não inicia execução e a recusa fica visível na barra de status;
o bloco recusado não muda de estado nem perde saída anterior;
cancelando o primeiro bloco, o segundo passa a executar normalmente;
terminando o primeiro naturalmente (sem cancelar), o segundo também
passa a executar; edição e foco continuam funcionando em todos os
blocos o tempo todo.
