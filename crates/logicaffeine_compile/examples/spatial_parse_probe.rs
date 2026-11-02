//! Scratch: which line of a source the spatial front end fails to parse (removed once the regression is fixed).
use logicaffeine_compile::spatial::pipeline::template_from_source;
use logicaffeine_compile::spatial::target::SpatialTarget;

fn main() {
    let src =
        std::fs::read_to_string(std::env::args().nth(1).expect("a source file")).expect("read");
    let lines: Vec<&str> = src.lines().collect();
    let wh = SpatialTarget::wormhole();
    for n in 1..=lines.len() {
        let head = lines[..n].join("\n");
        let e = template_from_source(&head, &wh).err();
        let parse = matches!(&e, Some(logicaffeine_compile::spatial::passes::SpatialReject::Parse(m)) if m.contains("needs the word"));
        if parse {
            println!("first PARSE failure after {n} lines: {}", lines[n - 1]);
            if let Some(logicaffeine_compile::spatial::passes::SpatialReject::Parse(m)) = e {
                println!("{}", &m[..m.len().min(300)]);
            }
            return;
        }
    }
    println!("no parse failure in {} lines", lines.len());
}
