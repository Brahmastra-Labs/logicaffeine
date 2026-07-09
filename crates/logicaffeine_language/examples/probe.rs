//! Scratch probe: parse argv sentences and print FOL or the error.

fn main() {
    for arg in std::env::args().skip(1) {
        if let Some(rest) = arg.strip_prefix("tokens:") {
            let mut interner = logicaffeine_language::Interner::new();
            let mut lexer = logicaffeine_language::Lexer::new(rest, &mut interner);
            let toks = lexer.tokenize();
            for t in &toks {
                println!("  {:?}", t.kind);
            }
            continue;
        }
        if let Some(rest) = arg.strip_prefix("session:") {
            let mut session = logicaffeine_language::Session::new();
            for turn in rest.split('|') {
                match session.eval(turn) {
                    Ok(fol) => println!("SOK  {turn}\n  => {fol}"),
                    Err(e) => println!("SERR {turn}\n  => {e:?}"),
                }
            }
            continue;
        }
        match logicaffeine_language::compile::compile(&arg) {
            Ok(fol) => println!("OK   {arg}\n  => {fol}"),
            Err(e) => println!("ERR  {arg}\n  => {e:?}"),
        }
        let forest = logicaffeine_language::compile::compile_forest(&arg);
        println!("FOREST({}) {:?}", forest.len(), forest);
    }
}
