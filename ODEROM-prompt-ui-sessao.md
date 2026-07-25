# ODEROM — interface gráfica com sessão

> Prompt autocontido. Não pressupõe nenhuma conversa anterior sobre UI.
> Identificadores, comentários e commits em inglês; conversa comigo em português.

---

## Contexto

O ODEROM hoje é um núcleo em Rust mais uma CLI. O que já existe e funciona:

- linguagem `.od` com `manifold`, `bundle`, `chart`, `metric`, `connection`, nas grafias ASCII e LaTeX, com um único parser de `SCALAR_EXPR`
- subcomandos `christoffel`, `riemann`, `ricci`, `scalar`, `kretschmann`
- trait `Render` com alvos `Unicode`, `Latex`, `Json`
- elisão por simetria na exibição de componentes (órbitas, supressão de zeros com contagem, truncamento explícito)
- execução em thread com orçamento de parede, progresso por estágio e guarda de grau de denominador
- motor aritmético com forma normal racional: Reissner–Nordström completa em ~21,5 s, Schwarzschild em ~1,5 s

O que falta é o lugar onde eu de fato trabalho. Hoje só existe linha de comando, e ninguém faz pesquisa em geometria diferencial digitando subcomando e lendo texto cru.

## Objetivo

Um aplicativo de desktop, binário único, com instalação de um clique — sem stack Python, sem servidor, sem kernel externo. Referência de experiência: Cadabra e Mathematica. Eu defino uma métrica uma vez e vou trabalhando em cima dela, fazendo perguntas, montando equações, vendo tudo tipografado.

**Não é um notebook Jupyter e não deve imitar um.** As diferenças estão especificadas abaixo e são deliberadas.

## Stack

- **Tauri** como casca desktop. Backend em Rust; as crates do ODEROM entram **linkadas como biblioteca**, com chamada de função direta. Sem subprocesso, sem protocolo, sem IPC com a CLI.
- **Front-end** em HTML, CSS e JavaScript simples. Sem framework, a menos que você me apresente um motivo concreto.
- **KaTeX** para tipografia. O alvo `Target::Latex` já existe — o front-end só passa a string. Não escreva tipografia nenhuma.
- A CLI continua existindo e continua sendo a interface testada em CI.

---

## O modelo de sessão

Esta é a parte que exige desenho, e é onde quero sua atenção. O resto é encanamento.

### Duas áreas com regras diferentes

**Definições** — o documento `.od`. Variedade, fibrado, carta, métrica, conexão. É o que eu salvo em disco, versiono e mando para um colega. Editado como documento inteiro, avaliado como documento inteiro.

**Planilha de trabalho** — a sequência de perguntas que eu faço em cima das definições. `ricci`, uma contração, uma substituição, uma equação sendo montada. Interativo, descartável, cada entrada independente das outras.

A separação existe porque as regras são diferentes: **definição muda raro e invalida coisas; pergunta muda toda hora e não invalida nada**. Misturar as duas num fluxo único de células é exatamente o que torna o Jupyter confuso, e é o que não quero.

### O problema central: resultado obsoleto

Esta é a razão de a sessão exigir desenho em vez de ser só uma variável global.

Eu defino a métrica, calculo o Ricci, olho a saída. Depois volto e edito a métrica. O Ricci na tela agora está errado — mas continua ali, com a mesma aparência de um resultado válido. Meia hora depois eu não lembro mais de qual versão ele veio.

É a doença clássica do Mathematica e do Jupyter, e é a maior fonte de resultado errado em ciência feita com notebook. O ODEROM inteiro foi construído em torno de verificação — portão de teste diferencial, verificação independente de divisão exata, guarda de expoente. Importar essa doença pela porta da interface seria contradizer a tese do projeto.

### Regra inegociável

> **Nunca pode existir na tela um resultado que aparenta ser atual e não é.**

Implementação:

- cada entrada da planilha registra **de quais definições ela dependeu** ao ser calculada
- quando as definições mudam, toda entrada que dependia de algo alterado é marcada **obsoleta**
- obsoleta significa: visualmente distinta e inequívoca, com um jeito óbvio de recalcular
- obsoleta **não** significa: sumir, recalcular sozinho, ou ficar igual com um aviso discreto

Nada de recálculo automático em cascata. Recomputar RN custa 21,5 s; eu decido quando pagar isso.

### Granularidade

O alvo é **por nome**: a entrada guarda o conjunto de nomes de definição que referenciou (`{g, schw}`), e só é invalidada se algum deles mudou. Editar um comentário no documento não deve invalidar nada; trocar `g` deve invalidar tudo que usou `g`.

Se na primeira versão for mais simples invalidar tudo a cada reavaliação das definições, tudo bem — **mas o tipo já nasce carregando o conjunto de nomes**, para o refinamento não exigir tocar em tudo depois. Deixe explícito no código qual das duas está implementada.

### Atomicidade

Avaliar as definições é atômico: parse do documento inteiro, construção de um `Model` novo, e **troca só em caso de sucesso**. Uma avaliação que falha nunca deixa a sessão com um `Model` pela metade. O estado anterior continua válido e utilizável, e o erro aparece apontando linha e coluna.

Entradas da planilha são independentes entre si: uma que falha vira entrada em estado de erro e não afeta as outras.

### Cache

Reaproveite o cache indexado pelo hash da forma canônica que já existe no desenho do núcleo. Mudar uma pergunta na planilha não pode recalcular Christoffel do zero. Mudar a métrica, sim.

### Persistência

- as definições são salvas como arquivo `.od` normal, e são a fonte de verdade
- a planilha pode ser salva num arquivo companheiro, contendo **apenas as entradas, nunca as saídas**
- ao reabrir, as definições são reavaliadas e as entradas aparecem não-calculadas

Isso é deliberado: gravar saída em arquivo é como a doença do resultado obsoleto sobrevive ao fechamento do programa. Se quiser evitar recomputar tudo ao abrir, persista o **cache** separadamente, indexado por hash — nunca a saída apresentada como atual.

---

## Superfície do backend

Comandos Tauri, com estes contratos. Ajuste os nomes se preferir, mas mantenha a forma.

```
evaluate_definitions(source: String)
    -> Ok { names: Vec<Name>, elapsed_ms }
     | Err { message, line, column }

run_entry(input: String)
    -> Ok { latex: String, unicode: String, used: Vec<Name>, elapsed_ms }
     | Err { message, line, column }

cancel_running()
    -> ()

session_snapshot()
    -> { has_model: bool, names: Vec<Name>, entries: Vec<EntryState> }
```

Progresso é emitido como evento durante a execução, reaproveitando o progresso por estágio que já existe.

**Regra dura:** nenhuma lógica de geometria, de álgebra ou de renderização matemática pode viver no front-end. O front-end recebe strings prontas e as entrega ao KaTeX. Se você se pegar escrevendo lógica de índice ou de simetria em JavaScript, o desenho está errado.

---

## Layout

- **barra superior** — nome do arquivo, botão de avaliar definições, botão de cancelar, tempo da última execução
- **painel esquerdo** — editor das definições `.od`, com realce de sintaxe e marcação de erro na linha
- **painel direito** — planilha: lista de entradas, cada uma com o que foi digitado e a saída tipografada abaixo; campo de nova entrada ao final
- **barra inferior** — estado atual e mensagem de erro

Entradas obsoletas ficam visualmente distintas no painel direito, com botão de recalcular.

---

## Escopo da v1

Dentro:

- abrir, editar e salvar `.od`
- realce de sintaxe
- avaliar definições, com erro apontando linha e coluna
- planilha com entradas, saída tipografada, elisão por simetria já existente
- marcação de obsolescência
- progresso e cancelamento
- exemplo carregado ao abrir sem arquivo

Fora — não implemente, nem como esboço:

- gráficos e visualização
- autocompletar
- múltiplos documentos ou abas
- exportar PDF
- histórico, desfazer além do editor, qualquer coisa colaborativa

Gráficos entram depois no mesmo painel como SVG, sem arquitetura nova. Não pré-construa nada para eles.

---

## Pré-requisitos

Antes do aplicativo, duas coisas que hoje não existem e vão morder:

1. **`examples/` com arquivos `.od` comentados** — Schwarzschild, Reissner–Nordström, S², nas duas grafias, com comentário explicando cada declaração. Hoje a única fonte de verdade da sintaxe está em `tests/fixtures/`, e isso já me custou tempo. Um desses exemplos vira o conteúdo inicial quando o aplicativo abre sem arquivo.

2. **História de instalação num comando**, documentada no README, para a CLI e para o aplicativo. Eu já travei uma vez no primeiro comando porque não havia binário no PATH.

---

## Critério de aceitação

Definido do meu lado, em termos do que eu faço, não de código:

1. abro o ODEROM e o exemplo de Reissner–Nordström já está carregado
2. clico em avaliar; as definições são aceitas
3. digito `ricci` na planilha e vejo o Ricci tipografado
4. digito `kretschmann`, vejo progresso, e consigo cancelar no meio
5. edito a métrica no painel esquerdo e reavalio; **as saídas anteriores ficam visivelmente obsoletas**
6. recalculo uma entrada e ela volta ao estado atual

O item 5 é o que separa este programa de um notebook comum. Se ele não funcionar, o resto não importa.

---

## Como trabalhar

1. **Antes de qualquer código**, escreva `DESIGN-UI-SESSION.md` respondendo:
   - as structs de sessão, entrada e estado de obsolescência, com seus campos
   - como o conjunto de nomes referenciados é coletado durante a avaliação
   - o que exatamente invalida o quê, e qual granularidade você vai implementar primeiro
   - como a obsolescência aparece na tela
   - como a atomicidade da troca de `Model` é garantida
   - como cancelamento e progresso se ligam à infraestrutura de thread que já existe

   **Pare aí e espere minha resposta.** Não comece a implementar sem meu ok.

2. Depois do ok: commits pequenos, um conceito por commit.
3. A suíte existente continua verde a cada passo, incluindo `v1_and_v2_agree`. A CLI continua funcionando.
4. Testes do modelo de sessão em Rust, sem depender da interface: definir, consultar, redefinir, verificar que a entrada ficou obsoleta, recalcular, verificar que voltou. Isso é lógica, não pintura, e tem que ser testável sem abrir janela.
5. Se em algum ponto o desenho exigir lógica matemática no front-end, **pare e me diga** em vez de contornar.

Comece pelo `DESIGN-UI-SESSION.md`.
