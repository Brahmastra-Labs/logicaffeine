use super::*;

/// One file of an archive to write: its name, its bytes, and whether it is stored DEFLATE.
struct File<'a> {
    name: &'a str,
    bytes: &'a [u8],
    deflate: bool,
    method: Option<u16>,
}

fn file<'a>(name: &'a str, bytes: &'a [u8]) -> File<'a> {
    File { name, bytes, deflate: false, method: None }
}

/// A zip of `files`, each with its local header, then the central directory and its end record — in the ZIP64 form,
/// every count, size and offset in the ZIP64 record and extra fields, when `zip64`. `size_lie` adds to every stored
/// size the directory records.
fn zip(files: &[File], zip64: bool, size_lie: u64) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for f in files {
        let data = if f.deflate { miniz_oxide::deflate::compress_to_vec(f.bytes, 6) } else { f.bytes.to_vec() };
        let method = f.method.unwrap_or(if f.deflate { 8 } else { 0 });
        let header = out.len() as u64;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0]);
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(f.bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(f.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(f.name.as_bytes());
        out.extend_from_slice(&data);
        let size = f.bytes.len() as u64 + size_lie;
        let (c32, s32, h32) = if zip64 { (u32::MAX, u32::MAX, u32::MAX) } else { (data.len() as u32, size as u32, header as u32) };
        let extra: Vec<u8> = if zip64 {
            let mut e = 0x0001u16.to_le_bytes().to_vec();
            e.extend_from_slice(&24u16.to_le_bytes());
            e.extend_from_slice(&size.to_le_bytes());
            e.extend_from_slice(&(data.len() as u64).to_le_bytes());
            e.extend_from_slice(&header.to_le_bytes());
            e
        } else {
            Vec::new()
        };
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[45, 0, 45, 0, 0, 0]);
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&[0; 4]);
        central.extend_from_slice(&[0; 4]);
        central.extend_from_slice(&c32.to_le_bytes());
        central.extend_from_slice(&s32.to_le_bytes());
        central.extend_from_slice(&(f.name.len() as u16).to_le_bytes());
        central.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        // Comment length, disk number, internal and external attributes.
        central.extend_from_slice(&[0; 10]);
        central.extend_from_slice(&h32.to_le_bytes());
        central.extend_from_slice(f.name.as_bytes());
        central.extend_from_slice(&extra);
    }
    let cd_offset = out.len() as u64;
    out.extend_from_slice(&central);
    let (count, cd_size) = (files.len() as u64, central.len() as u64);
    if zip64 {
        let record = out.len() as u64;
        out.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
        out.extend_from_slice(&44u64.to_le_bytes());
        out.extend_from_slice(&[45, 0, 45, 0]);
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&record.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    let (n16, s32, o32) = if zip64 { (u16::MAX, u32::MAX, u32::MAX) } else { (count as u16, cd_size as u32, cd_offset as u32) };
    out.extend_from_slice(&n16.to_le_bytes());
    out.extend_from_slice(&n16.to_le_bytes());
    out.extend_from_slice(&s32.to_le_bytes());
    out.extend_from_slice(&o32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn written(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("net.pt2");
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

/// The bytes of a weight file: a counting pattern, as wide as it is asked.
fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 + 3) as u8).collect()
}

#[test]
fn stored_and_deflated_entries_read_whole_and_by_range() {
    let weight = pattern(4096);
    let graph = br#"{"graph_module": {"graph": {"nodes": []}}}"#;
    for zip64 in [false, true] {
        let bytes = zip(
            &[
                file("net/data/weights/weight_0", &weight),
                File { name: "net/models/model.json", bytes: graph, deflate: true, method: None },
                file("net/archive_format", b"pt2"),
            ],
            zip64,
            0,
        );
        let (_dir, path) = written(&bytes);
        let a = Pt2::open(&path).unwrap_or_else(|e| panic!("zip64 {zip64}: {e}"));
        assert_eq!(a.files(), vec!["archive_format", "data/weights/weight_0", "models/model.json"], "zip64 {zip64}");
        assert!(a.has("models/model.json") && !a.has("models/other.json"), "zip64 {zip64}");
        assert_eq!(a.read("data/weights/weight_0").unwrap(), weight, "zip64 {zip64}: a stored entry, whole");
        assert_eq!(a.read_range("data/weights/weight_0", 1000, 96).unwrap(), weight[1000..1096], "zip64 {zip64}: by range");
        assert_eq!(a.read("models/model.json").unwrap(), graph.to_vec(), "zip64 {zip64}: a deflated entry, inflated");
        assert_eq!(a.read_range("models/model.json", 2, 13).unwrap(), graph[2..15].to_vec(), "zip64 {zip64}: deflated, by range");
        assert_eq!(a.json("models/model.json").unwrap()["graph_module"]["graph"]["nodes"], serde_json::json!([]), "zip64 {zip64}");
        assert!(a.read_range("data/weights/weight_0", 4000, 97).is_err(), "zip64 {zip64}: a range past the entry is refused");
        assert!(a.read("data/weights/weight_9").is_err(), "zip64 {zip64}: an absent entry is refused");
    }
}

#[test]
fn what_is_not_a_pt2_archive_is_refused_by_name() {
    let (_d, path) = written(b"not a zip at all, a text file long enough to search the tail of");
    let e = Pt2::open(&path).unwrap_err().to_string();
    assert!(e.contains("not a zip archive"), "{e}");

    let bytes = zip(&[File { name: "net/data/weights/weight_0", bytes: b"abc", deflate: false, method: Some(12) }], false, 0);
    let (_d, path) = written(&bytes);
    let e = Pt2::open(&path).unwrap_err().to_string();
    assert!(e.contains("method 12") && e.contains("weight_0"), "{e}");

    let bytes = zip(&[file("net/models/model.json", b"{}"), file("other/models/model.json", b"{}")], false, 0);
    let (_d, path) = written(&bytes);
    let e = Pt2::open(&path).unwrap_err().to_string();
    assert!(e.contains("one root folder"), "{e}");

    let bytes = zip(&[file("net/data/weights/weight_0", b"abcd")], false, 3);
    let (_d, path) = written(&bytes);
    let a = Pt2::open(&path).unwrap();
    let e = a.read("data/weights/weight_0").unwrap_err().to_string();
    assert!(e.contains("holds 4 bytes where the directory says 7"), "{e}");
}
