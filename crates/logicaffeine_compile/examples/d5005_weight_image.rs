//! Write the D5005 weight-image ledger — every Qwen3.8-27B checkpoint laid out over the card's four channels at both
//! beats — to `benchmarks/silicon/d5005/weight_image.json`, and print it; or write one checkpoint's image as the
//! card's loader takes it — the machine layout's tensors as the machine stores them, `channel<k>.bin` a channel's bytes,
//! `tiles.bin` the tile table — and read it back: every tile's check word, every tensor's first and last rows against
//! the checkpoint. Usage:
//!   cargo run -p logicaffeine-compile --example d5005_weight_image
//!   cargo run --release -p logicaffeine-compile --example d5005_weight_image -- --write <gguf> <dir> [data64|full72]

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::d5005::Beat;
use logicaffeine_compile::spatial::decode_engine::image::{crc32c, image_ledger, image_of, Image, ImageLayout, CHECK_BYTES};
use logicaffeine_compile::spatial::decode_engine::machine::Layout as MachineLayout;
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::program::lower;

fn write_image(gguf: &Path, dir: &Path, beat: Beat) {
    let mut g = Gguf::open(gguf).unwrap_or_else(|e| panic!("{}: {e}", gguf.display()));
    let spec = ModelSpec::from_gguf(&g).unwrap_or_else(|e| panic!("{}: {e:?}", gguf.display()));
    let numerics = EngineNumerics::fastest();
    let program = lower(&spec, numerics, 1).unwrap_or_else(|e| panic!("{}: {e}", gguf.display())).program;
    let machine = MachineLayout::of(&spec, numerics, &program, &g, 1, 1).unwrap_or_else(|e| panic!("{e:?}"));
    let image = image_of(&machine, &ImageLayout::d5005(beat)).unwrap_or_else(|e| panic!("{e:?}"));
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut files: Vec<BufWriter<File>> = (0..image.channel_lines.len())
        .map(|k| {
            let path = dir.join(format!("channel{k}.bin"));
            BufWriter::with_capacity(1 << 22, File::create(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        })
        .collect();
    let mut out: Vec<&mut dyn Write> = files.iter_mut().map(|f| f as &mut dyn Write).collect();
    image.write_channels(&mut g, &mut out).unwrap_or_else(|e| panic!("writing the channels: {e}"));
    for f in &mut files {
        f.flush().expect("a channel's file flushes");
    }
    std::fs::write(dir.join("tiles.bin"), image.tile_table()).expect("tiles.bin");
    println!(
        "wrote {}: {} tensors, {} tiles, {:?} lines of {} bytes a channel",
        dir.display(),
        image.tensors.len(),
        image.tiles.len(),
        image.channel_lines,
        image.layout.line_bytes
    );
    verify_image(&image, &mut g, dir);
}

/// Read the written image back: every channel file its lines long, every tile's check word its CRC-32C, and every
/// tensor's first and last stored rows where `locate` puts them, byte for byte the checkpoint's as the machine stores
/// them.
fn verify_image(image: &Image, g: &mut Gguf, dir: &Path) {
    let lb = image.layout.line_bytes as usize;
    let mut files: Vec<File> = (0..image.channel_lines.len()).map(|k| File::open(dir.join(format!("channel{k}.bin"))).expect("a channel's file")).collect();
    for (k, f) in files.iter().enumerate() {
        let len = f.metadata().expect("a channel's file").len();
        assert_eq!(len, image.channel_lines[k] * lb as u64, "channel{k}.bin");
    }
    if image.layout.check {
        let mut readers: Vec<BufReader<&File>> = files.iter().map(|f| BufReader::with_capacity(1 << 22, f)).collect();
        for (i, t) in image.tiles.iter().enumerate() {
            let mut span = vec![0u8; t.lines as usize * lb];
            readers[t.channel as usize].read_exact(&mut span).expect("a tile's lines");
            let body = span.len() - CHECK_BYTES as usize;
            assert_eq!(span[body..], crc32c(&span[..body]).to_le_bytes(), "tile {i}'s check word");
        }
    }
    let mut rows = 0;
    for (i, p) in image.tensors.iter().enumerate() {
        for row in [0, p.rows - 1] {
            let (ch, at) = image.locate(i, row);
            let want = image.stored_rows(i, g, row, 1).expect("a row of the checkpoint");
            let mut got = vec![0u8; want.len()];
            let f = &mut files[ch as usize];
            f.seek(SeekFrom::Start(at)).expect("a seek");
            f.read_exact(&mut got).expect("a row of the image");
            assert_eq!(got, want, "`{}` row {row}", p.name);
            rows += 1;
        }
    }
    println!("verified {}: every tile's check word, {rows} rows against the checkpoint", dir.display());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--write") {
        let beat = match args.get(3).map(String::as_str) {
            None | Some("full72") => Beat::Full72,
            Some("data64") => Beat::Data64,
            Some(other) => panic!("a beat is data64 or full72, not {other}"),
        };
        let (gguf, dir) = (args.get(1).expect("the checkpoint"), args.get(2).expect("the image directory"));
        write_image(Path::new(gguf), Path::new(dir), beat);
        return;
    }
    let silicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let l = image_ledger(&silicon.join("decode")).unwrap_or_else(|e| panic!("the weight-image ledger: {e}"));
    for r in &l.rows {
        println!(
            "{:<22} {:?}  {:>4} tensors {:>7} tiles  {:>6.2} GB data  {:>8} padding  {:>7} check  channels {:?} lines  spread {} of a {}-line tile",
            r.header,
            r.beat,
            r.tensors,
            r.tiles,
            r.data_bytes as f64 / 1e9,
            r.padding_bytes,
            r.check_bytes,
            r.channel_lines,
            r.spread_lines,
            r.largest_tile_lines
        );
    }
    let path = silicon.join("d5005/weight_image.json");
    std::fs::write(&path, serde_json::to_string_pretty(&l).expect("the ledger serializes") + "\n")
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}
