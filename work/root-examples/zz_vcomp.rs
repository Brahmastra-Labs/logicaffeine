use logicaffeine_language::compile;
fn main() {
    for clue in [
        "The design took more minutes than the item.",
        "The design took 10 minutes.",
        "Tara scored 3 points lower than Bessie.",
        "Tara scored 3 more points than Bessie.",
    ] {
        match compile(clue) {
            Ok(out) => println!("OK:  {}\n     -> {}\n", clue, out),
            Err(e) => println!("ERR: {}\n     -> {:?}\n", clue, e.kind),
        }
    }
}
