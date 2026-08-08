//! `index.html` e `keytest.html` carregam o MESMO `notebook.js`, então
//! todo id que ele procura tem de existir nos dois.
//!
//! `keytest.html` (o ponto de entrada do teste de UI real, ver
//! `frontend_entry_point` em `src/lib.rs`) é uma cópia à mão do markup
//! do `index.html`. Essa duplicação já saiu cara duas vezes: uma quando
//! os `<script>` divergiram, outra quando o botão "Exportar" foi
//! acrescentado só ao `index.html` -- e nesse segundo caso o
//! `notebook.js` chamaria `addEventListener` sobre `null`, derrubando
//! *todo* o resto da página, inclusive tudo o que `keymap.rs` testa. O
//! sintoma teria sido "o teste de teclado quebrou", que não aponta para
//! a causa.
//!
//! Este teste lê os ids que o `notebook.js` realmente procura, em vez
//! de manter uma terceira lista que também poderia envelhecer.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn dist() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist")
}

/// Todo `document.getElementById("X")` que aparece em `js`.
///
/// Um scanner de texto, não um parser de JavaScript: a forma é fixa
/// neste arquivo (sempre literal, sempre aspas duplas), e um id montado
/// dinamicamente -- que este scanner não veria -- também não seria um
/// id que os dois HTML pudessem declarar estaticamente. Se algum dia
/// passar a existir, ele fica fora daqui de propósito.
fn ids_procurados(js: &str) -> BTreeSet<&str> {
    js.match_indices("document.getElementById(\"")
        .filter_map(|(i, prefixo)| {
            let resto = &js[i + prefixo.len()..];
            resto.find('"').map(|fim| &resto[..fim])
        })
        .collect()
}

#[test]
fn index_e_keytest_declaram_todo_id_que_o_notebook_js_procura() {
    let js = std::fs::read_to_string(dist().join("notebook.js")).unwrap();
    let ids = ids_procurados(&js);
    assert!(ids.len() > 5, "o scanner nao achou ids suficientes -- a forma da chamada mudou? achou: {ids:?}");

    for pagina in ["index.html", "keytest.html"] {
        let html = std::fs::read_to_string(dist().join(pagina)).unwrap();
        let faltando: Vec<&str> = ids.iter().copied().filter(|id| !html.contains(&format!("id=\"{id}\""))).collect();
        assert!(
            faltando.is_empty(),
            "{pagina} nao declara {faltando:?}, que o notebook.js procura -- \
             os dois carregam o mesmo script e precisam do mesmo markup"
        );
    }
}

/// Os dois carregam exatamente os mesmos `<script src>`, na mesma
/// ordem, com a única exceção do driver do próprio teste.
///
/// A ordem importa: `backend.js` define o `invoke` que o `notebook.js`
/// lê no topo do arquivo, então trocar os dois de lugar quebra a página
/// inteira. Foi assim que a divergência apareceu da primeira vez.
#[test]
fn index_e_keytest_carregam_os_mesmos_scripts_na_mesma_ordem() {
    let scripts = |pagina: &str| -> Vec<String> {
        let html = std::fs::read_to_string(dist().join(pagina)).unwrap();
        html.match_indices("<script src=\"")
            .filter_map(|(i, prefixo)| {
                let resto = &html[i + prefixo.len()..];
                resto.find('"').map(|fim| resto[..fim].to_string())
            })
            .filter(|s| s != "keytest.js")
            .collect()
    };
    assert_eq!(
        scripts("index.html"),
        scripts("keytest.html"),
        "os <script> das duas paginas divergiram -- keytest.html precisa dirigir os MESMOS arquivos que o aluno carrega"
    );
}
