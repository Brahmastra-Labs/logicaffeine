use logicaffeine_language::compile;
fn main() {
    for clue in [
        "Of Arnold and Rosie, one is tall and the other is short.",
        "Of the cat and the dog, one is tall and the other is short.",
        "Of the French class and the Art class, one is tall and the other is short.",
        "Of the French class and the Art class, one is taught by Mr. Farmer and the other is held during fourth period.",
    ] {
        match compile(clue) {
            Ok(out) => println!("OK:  {}\n     → {}\n", clue, out),
            Err(e) => println!("ERR: {}\n     → {:?}\n", clue, e),
        }
    }
}
