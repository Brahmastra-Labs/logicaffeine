//! A memory as the card's channels hold it for the card port's tests: every channel line a bus line — the line's 64
//! data bytes, or on SECDED lines its 72 bus bytes as `ofs::secded_line` codes them — laid out by a physical map, a
//! script of engine reads and writes over it with the reads it expects, and the run of that script through
//! `logos_card_port` under iverilog on the shell's own channel path; beside it, a script of a hybrid engine's fast side
//! through `logos_fast_port` on the hybrid shell's fast channel ports.

#![allow(dead_code)]

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::spatial::decode_engine::machine::LINE;
use logicaffeine_compile::spatial::decode_engine::ofs::{
    afu_sources, hybrid_shell_verilog, secded_line, secded_read, AfuConfig, LocalMemoryLine,
};
use logicaffeine_compile::spatial::decode_engine::physical::{card_port_verilog, channel_mux_verilog, fast_port_verilog, PhysicalMap};

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/d5005_card_port").join(name)
}

pub fn text(o: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

pub fn next(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// A line's bus bytes as a Verilog word, its last byte most significant.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().rev().map(|b| format!("{b:02x}")).collect()
}

/// A channel line as the bus holds it, and what a read of it gives: the data and whether a word was uncorrectable.
pub fn code(secded: bool, data: &[u8; 64]) -> Vec<u8> {
    if secded {
        secded_line(data).to_vec()
    } else {
        data.to_vec()
    }
}

pub fn decode(secded: bool, bus: &[u8]) -> ([u8; 64], bool) {
    if secded {
        let r = secded_read(bus.try_into().unwrap());
        (r.data, r.uncorrectable != 0)
    } else {
        (bus.try_into().unwrap(), false)
    }
}

/// One run of the port: the map, the bus line every channel line holds — as the ops leave it, and as the first op
/// found it — the ops and the reads they expect, and whether the ops fault.
pub struct Run {
    pub map: PhysicalMap,
    pub secded: bool,
    pub channels: Vec<Vec<Vec<u8>>>,
    pub before: Option<Vec<Vec<Vec<u8>>>>,
    pub ops: String,
    pub expect: String,
    pub reads: usize,
    pub count: usize,
    pub fault: bool,
}

impl Run {
    /// The channels holding `memory` as `map` lays it out — the weight image's lines from `weights` — each bus line
    /// coded as the line asks.
    pub fn new(map: PhysicalMap, secded: bool, memory: &[u8], weights: &[Vec<u8>]) -> Run {
        let lb = map.image.layout.line_bytes as usize;
        let flat = map.channel_bytes(memory, weights);
        let channels = flat
            .iter()
            .map(|c| {
                c.chunks(lb)
                    .map(|l| {
                        let mut d = [0u8; 64];
                        d.copy_from_slice(&l[..64]);
                        code(secded, &d)
                    })
                    .collect()
            })
            .collect();
        Run { map, secded, channels, before: None, ops: String::new(), expect: String::new(), reads: 0, count: 0, fault: false }
    }

    pub fn space(&self) -> u32 {
        (self.map.tensors_end / LINE) as u32
    }

    /// Logical line `x`'s channel and channel line, or `None` below the space, where the port faults.
    pub fn place(&self, x: u32) -> Option<(usize, usize)> {
        let at = x as u64 * LINE;
        if at < self.map.tensors_end {
            return None;
        }
        assert!(x < self.space() + self.map.lines as u32, "line {x} past the map's space");
        let (c, byte) = self.map.locate(at);
        Some((c as usize, (byte / self.map.image.layout.line_bytes) as usize))
    }

    /// The channels as the first op finds them: what the memories start the run holding.
    fn starting(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.channels.clone());
        }
    }

    pub fn read(&mut self, x: u32) {
        self.starting();
        let line = match self.place(x) {
            Some((c, k)) => {
                let (d, bad) = decode(self.secded, &self.channels[c][k]);
                self.fault |= bad;
                d
            }
            None => {
                self.fault = true;
                [0u8; 64]
            }
        };
        self.ops += &format!("00{x:08x}{:016x}{}\n", 0u64, hex(&[0u8; 64]));
        self.expect += &format!("{}\n", hex(&line));
        self.reads += 1;
        self.count += 1;
    }

    pub fn write(&mut self, x: u32, mask: u64, data: [u8; 64]) {
        self.starting();
        match self.place(x) {
            Some((c, k)) if mask == u64::MAX => self.channels[c][k] = code(self.secded, &data),
            Some((c, k)) => {
                let (old, bad) = decode(self.secded, &self.channels[c][k]);
                if bad {
                    self.fault = true;
                } else {
                    let merged: [u8; 64] = std::array::from_fn(|i| if mask >> i & 1 == 1 { data[i] } else { old[i] });
                    self.channels[c][k] = code(self.secded, &merged);
                }
            }
            None => self.fault = true,
        }
        self.ops += &format!("01{x:08x}{mask:016x}{}\n", hex(&data));
        self.count += 1;
    }

    /// `n` ops over the space's lines from `seed`: reads anywhere and of lines just written, writes under every byte,
    /// under a random scatter of bytes and under a few bytes, back to back and apart.
    pub fn script(&mut self, n: usize, seed: u64) {
        self.script_where(n, seed, &|_| true);
    }

    /// The granule of the space logical line `x` lies in.
    pub fn granule(&self, x: u32) -> u64 {
        (x as u64 * LINE - self.map.tensors_end) / LINE / self.map.granule
    }

    /// [`Run::script`] over the lines of the granules `keep` holds.
    pub fn script_where(&mut self, n: usize, seed: u64, keep: &dyn Fn(u64) -> bool) {
        let (space, lines) = (self.space(), self.map.lines as u32);
        let mut s = seed;
        let mut recent: Vec<u32> = Vec::new();
        for _ in 0..n {
            let r = next(&mut s) % 100;
            let mut x = space + (next(&mut s) % lines as u64) as u32;
            while !keep(self.granule(x)) {
                x = space + (next(&mut s) % lines as u64) as u32;
            }
            match r {
                0..=39 => self.read(x),
                40..=54 if !recent.is_empty() => {
                    let y = recent[(next(&mut s) % recent.len() as u64) as usize];
                    self.read(y);
                }
                _ => {
                    let mut data = [0u8; 64];
                    for chunk in data.chunks_mut(8) {
                        chunk.copy_from_slice(&next(&mut s).to_le_bytes());
                    }
                    let mask = match next(&mut s) % 10 {
                        0..=3 => u64::MAX,
                        4..=6 => next(&mut s),
                        _ => (0..1 + next(&mut s) % 4).fold(0u64, |m, _| m | 1 << (next(&mut s) % 64)),
                    };
                    self.write(x, mask, data);
                    if next(&mut s) % 3 == 0 {
                        self.write(x, !mask | 1, data.map(|b| b ^ 0x5a));
                    }
                    recent.push(x);
                    if recent.len() > 16 {
                        recent.remove(0);
                    }
                }
            }
        }
    }

    /// Run the ops through the port under iverilog, up to `gap` cycles between them: every read as expected, the
    /// fault as the ops make it, and every channel line, read back from the memories, what the model holds.
    pub fn check(&self, gap: u64, seed: u64, what: &str) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ops.hex"), &self.ops).unwrap();
        std::fs::write(dir.path().join("expect.hex"), if self.expect.is_empty() { format!("{}\n", hex(&[0u8; 64])) } else { self.expect.clone() }).unwrap();
        let dump = self.channels.iter().map(Vec::len).max().unwrap().max(1);
        for (c, lines) in self.before.as_ref().unwrap_or(&self.channels).iter().enumerate() {
            let body: String = (0..dump).map(|k| format!("{}\n", hex(lines.get(k).map_or(&[0u8; 72][..self.bus_bytes()], |l| &l[..])))).collect();
            std::fs::write(dir.path().join(format!("channel{c}.hex")), body).unwrap();
        }
        let line = if self.secded { LocalMemoryLine::Data512Secded } else { LocalMemoryLine::Data512 };
        let afu = tempfile::tempdir().unwrap();
        let mut files = vec![fixture("tb_card_port.v")];
        for f in afu_sources(&AfuConfig { line, ..AfuConfig::d5005() }) {
            if ["hw/rtl/logos_dcfifo.v", "hw/rtl/logos_secded.v", "hw/rtl/logos_stream.v", "hw/rtl/logos_d5005_shell.v", "sim/logos_avmm_memory_model.v"].contains(&f.path) {
                let p = afu.path().join(f.path.rsplit('/').next().unwrap());
                std::fs::write(&p, f.text).unwrap();
                files.push(p);
            }
        }
        let port = afu.path().join("logos_card_port.v");
        std::fs::write(&port, card_port_verilog()).unwrap();
        files.push(port);
        let granule = self.map.granule;
        assert!(granule.is_power_of_two(), "the port's granule is a power of two");
        let lines_log2 = (64 - (dump as u64 - 1).leading_zeros()).max(1);
        let sim = dir.path().join("sim");
        let params = [
            format!("-Ptb.DWM={}", line.bus_bits()),
            format!("-Ptb.ECC={}", self.secded as u32),
            format!("-Ptb.LINES_LOG2={lines_log2}"),
            format!("-Ptb.DUMP={dump}"),
            format!("-Ptb.NOPS={}", self.count),
            format!("-Ptb.NREADS={}", self.reads.max(1)),
            format!("-Ptb.SPACE={}", self.space()),
            format!("-Ptb.BASE={}", self.map.base),
            format!("-Ptb.GRANULE_LOG2={}", granule.trailing_zeros()),
            format!("-Ptb.SEED={seed}"),
            format!("-Ptb.GAP={gap}"),
        ];
        let build = Command::new("iverilog").args(["-g2012", "-s", "tb"]).args(&params).arg("-o").arg(&sim).args(&files).output().expect("iverilog");
        assert!(build.status.success(), "{what}: iverilog:\n{}", text(&build));
        let run = Command::new("vvp").arg(&sim).current_dir(dir.path()).output().expect("vvp");
        let out = text(&run);
        let want = format!("RESULT ops={} reads={} errors=0 fault={}", self.count, self.reads, self.fault as u32);
        assert!(out.contains(&want), "{what}: want {want}:\n{out}");
        for (c, lines) in self.channels.iter().enumerate() {
            let got: Vec<String> = std::fs::read_to_string(dir.path().join(format!("out{c}.hex")))
                .unwrap()
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('@'))
                .map(str::to_lowercase)
                .collect();
            assert_eq!(got.len(), dump, "{what}: channel {c}'s lines");
            for (k, l) in lines.iter().enumerate() {
                assert_eq!(got[k], hex(l), "{what}: channel {c} line {k}");
            }
        }
    }

    pub fn bus_bytes(&self) -> usize {
        if self.secded {
            72
        } else {
            64
        }
    }

    /// The fast port's ops beside the card port's, `n` of them from `seed` over the lines of the granules `keep` holds:
    /// whole lines read and written in runs of up to 70 lines across granules and alone, now and then a line below the
    /// space, and fences; a read of a line written since the last fence is preceded by one.
    pub fn fast_script(&mut self, n: usize, seed: u64, keep: &dyn Fn(u64) -> bool) -> FastOps {
        assert!(!self.secded, "a hybrid engine's lines carry no SECDED code");
        assert!((0..self.map.lines.div_ceil(self.map.granule)).any(keep), "a granule the fast port's ops may reach");
        self.starting();
        let (space, lines) = (self.space(), self.map.lines as u32);
        let mut s = seed;
        let mut f = FastOps::default();
        let mut dirty: HashSet<u32> = HashSet::new();
        while f.count < n {
            let r = next(&mut s) % 100;
            if r < 3 {
                f.fence();
                dirty.clear();
                continue;
            }
            if r < 5 && space > 0 {
                let x = (next(&mut s) % space as u64) as u32;
                if next(&mut s) % 2 == 0 {
                    f.read(x, [0u8; 64]);
                } else {
                    f.write(x, random_line(&mut s));
                }
                f.fault = true;
                continue;
            }
            let write = next(&mut s) % 100 < 45;
            let len = [1, 1, 2, 3, 7, 16, 31, 32, 33, 40, 70][(next(&mut s) % 11) as usize];
            let from = space + (next(&mut s) % lines as u64) as u32;
            for x in from..(from + len).min(space + lines) {
                if !keep(self.granule(x)) {
                    continue;
                }
                let (c, k) = self.place(x).unwrap();
                if write {
                    let data = random_line(&mut s);
                    self.channels[c][k] = code(false, &data);
                    dirty.insert(x);
                    f.write(x, data);
                } else {
                    if dirty.contains(&x) {
                        f.fence();
                        dirty.clear();
                    }
                    f.read(x, decode(false, &self.channels[c][k]).0);
                }
            }
        }
        f
    }

    /// Run the card port's ops and the fast port's `fast` side by side on a hybrid engine's channels under iverilog, the
    /// card port's up to `gap` cycles between ops and the fast port's up to `fgap`, its descriptors of up to `maxl`
    /// lines: every read of each as expected and in order, the faults as the ops make them, and every channel line
    /// what the model holds.
    pub fn check_beside(&self, fast: &FastOps, gap: u64, fgap: u64, maxl: u32, seed: u64, what: &str) {
        let (out, dir, dump) = self.simulate(&[fast], false, gap, fgap, maxl, seed, what);
        let want = format!(
            "RESULT ops={} reads={} errors=0 fault={} fast={} freads={} ferrors=0 ffault={}",
            self.count,
            self.reads,
            self.fault as u32,
            fast.count,
            fast.reads,
            fast.fault as u32
        );
        assert!(out.contains(&want), "{what}: want {want}:\n{out}");
        self.check_channels(dir.path(), dump, what);
    }

    /// [`Run::check_beside`] with every port of `fasts` at once, all of them through `logos_channel_mux` onto the
    /// channels' one fast side: each port's reads as expected and in its order, each port's faults as its ops make them.
    pub fn check_muxed(&self, fasts: &[FastOps], gap: u64, fgap: u64, maxl: u32, seed: u64, what: &str) {
        assert!(fasts.len() >= 2, "{what}: ports to share the channels");
        let ports: Vec<&FastOps> = fasts.iter().collect();
        let (out, dir, dump) = self.simulate(&ports, true, gap, fgap, maxl, seed, what);
        for (p, f) in fasts.iter().enumerate() {
            let want = format!("PORT {p} fast={} freads={} ferrors=0 ffault={}", f.count, f.reads, f.fault as u32);
            assert!(out.contains(&want), "{what}: want {want}:\n{out}");
        }
        let want = format!(
            "RESULT ops={} reads={} errors=0 fault={} fast={} freads={} ferrors=0",
            self.count,
            self.reads,
            self.fault as u32,
            fasts.iter().map(|f| f.count).sum::<usize>(),
            fasts.iter().map(|f| f.reads).sum::<usize>()
        );
        assert!(out.contains(&want), "{what}: want {want}:\n{out}");
        self.check_channels(dir.path(), dump, what);
    }

    /// The card port's ops and the fast ports' `fasts` run on a hybrid engine's channels under iverilog — one fast port
    /// on the channels' fast side (tb_fast_port.v), or with `muxed` every one through the channel mux
    /// (tb_channel_mux.v) — from the channels as they were before the ops; what the simulation printed, its directory
    /// (the channels after in `out<b>.hex`) and the lines a channel's dump holds.
    #[allow(clippy::too_many_arguments)]
    fn simulate(&self, fasts: &[&FastOps], muxed: bool, gap: u64, fgap: u64, maxl: u32, seed: u64, what: &str) -> (String, tempfile::TempDir, usize) {
        assert!(!self.secded, "a hybrid engine's lines carry no SECDED code");
        assert!(self.count > 0 && fasts.iter().all(|f| f.count > 0), "{what}: ops on every port");
        assert!(muxed || fasts.len() == 1, "{what}: one fast port on the channels' fast side unless muxed");
        let dir = tempfile::tempdir().unwrap();
        let none = format!("{}\n", hex(&[0u8; 64]));
        let or_none = |e: &String| if e.is_empty() { none.clone() } else { e.clone() };
        std::fs::write(dir.path().join("ops.hex"), &self.ops).unwrap();
        std::fs::write(dir.path().join("expect.hex"), or_none(&self.expect)).unwrap();
        if muxed {
            for (p, f) in fasts.iter().enumerate() {
                std::fs::write(dir.path().join(format!("fast{p}.hex")), &f.ops).unwrap();
                std::fs::write(dir.path().join(format!("fast_expect{p}.hex")), or_none(&f.expect)).unwrap();
            }
            let counts: String = fasts.iter().map(|f| format!("{:08x}{:08x}\n", f.count, f.reads)).collect();
            std::fs::write(dir.path().join("counts.hex"), counts).unwrap();
        } else {
            std::fs::write(dir.path().join("fast.hex"), &fasts[0].ops).unwrap();
            std::fs::write(dir.path().join("fast_expect.hex"), or_none(&fasts[0].expect)).unwrap();
        }
        let dump = self.channels.iter().map(Vec::len).max().unwrap().max(1);
        for (c, lines) in self.before.as_ref().unwrap_or(&self.channels).iter().enumerate() {
            let body: String = (0..dump).map(|k| format!("{}\n", hex(lines.get(k).map_or(&[0u8; 64][..], |l| &l[..])))).collect();
            std::fs::write(dir.path().join(format!("channel{c}.hex")), body).unwrap();
        }
        let line = LocalMemoryLine::Data512;
        let afu = tempfile::tempdir().unwrap();
        let mut files = vec![fixture(if muxed { "tb_channel_mux.v" } else { "tb_fast_port.v" })];
        for f in afu_sources(&AfuConfig { line, ..AfuConfig::d5005() }) {
            if ["hw/rtl/logos_dcfifo.v", "hw/rtl/logos_stream.v", "sim/logos_avmm_memory_model.v"].contains(&f.path) {
                let p = afu.path().join(f.path.rsplit('/').next().unwrap());
                std::fs::write(&p, f.text).unwrap();
                files.push(p);
            }
        }
        let mut sources = vec![
            ("logos_d5005_shell.v", hybrid_shell_verilog(true)),
            ("logos_card_port.v", card_port_verilog()),
            ("logos_fast_port.v", fast_port_verilog()),
        ];
        if muxed {
            sources.push(("logos_channel_mux.v", channel_mux_verilog()));
        }
        for (name, text) in sources {
            let p = afu.path().join(name);
            std::fs::write(&p, text).unwrap();
            files.push(p);
        }
        let granule = self.map.granule;
        assert!(granule.is_power_of_two(), "the ports' granule is a power of two");
        let lines_log2 = (64 - (dump as u64 - 1).leading_zeros()).max(1);
        let sim = dir.path().join("sim");
        let mut params = vec![
            format!("-Ptb.LINES_LOG2={lines_log2}"),
            format!("-Ptb.DUMP={dump}"),
            format!("-Ptb.NOPS={}", self.count),
            format!("-Ptb.NREADS={}", self.reads.max(1)),
            format!("-Ptb.NFAST={}", fasts.iter().map(|f| f.count).max().unwrap()),
            format!("-Ptb.NFREADS={}", fasts.iter().map(|f| f.reads).max().unwrap().max(1)),
            format!("-Ptb.SPACE={}", self.space()),
            format!("-Ptb.BASE={}", self.map.base),
            format!("-Ptb.GRANULE_LOG2={}", granule.trailing_zeros()),
            format!("-Ptb.SEED={seed}"),
            format!("-Ptb.GAP={gap}"),
            format!("-Ptb.FGAP={fgap}"),
            format!("-Ptb.MAXL={maxl}"),
        ];
        if muxed {
            params.push(format!("-Ptb.P={}", fasts.len()));
        }
        let build = Command::new("iverilog").args(["-g2012", "-s", "tb"]).args(&params).arg("-o").arg(&sim).args(&files).output().expect("iverilog");
        assert!(build.status.success(), "{what}: iverilog:\n{}", text(&build));
        let run = Command::new("vvp").arg(&sim).current_dir(dir.path()).output().expect("vvp");
        (text(&run), dir, dump)
    }

    /// Every channel line the simulation in `dir` left (its `out<b>.hex`, `dump` lines a channel) what the model holds.
    fn check_channels(&self, dir: &std::path::Path, dump: usize, what: &str) {
        for (c, lines) in self.channels.iter().enumerate() {
            let got: Vec<String> = std::fs::read_to_string(dir.join(format!("out{c}.hex")))
                .unwrap()
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('@'))
                .map(str::to_lowercase)
                .collect();
            assert_eq!(got.len(), dump, "{what}: channel {c}'s lines");
            for (k, l) in lines.iter().enumerate() {
                assert_eq!(got[k], hex(l), "{what}: channel {c} line {k}");
            }
        }
    }
}

/// 64 random bytes from `s`.
fn random_line(s: &mut u64) -> [u8; 64] {
    let mut data = [0u8; 64];
    for chunk in data.chunks_mut(8) {
        chunk.copy_from_slice(&next(s).to_le_bytes());
    }
    data
}

/// The fast port's ops and the reads it expects, in the order they return.
#[derive(Default)]
pub struct FastOps {
    pub ops: String,
    pub expect: String,
    pub reads: usize,
    pub count: usize,
    pub fault: bool,
}

impl FastOps {
    fn read(&mut self, x: u32, line: [u8; 64]) {
        self.ops += &format!("00{x:08x}{}\n", hex(&[0u8; 64]));
        self.expect += &format!("{}\n", hex(&line));
        self.reads += 1;
        self.count += 1;
    }

    fn write(&mut self, x: u32, data: [u8; 64]) {
        self.ops += &format!("01{x:08x}{}\n", hex(&data));
        self.count += 1;
    }

    fn fence(&mut self) {
        self.ops += &format!("02{:08x}{}\n", 0, hex(&[0u8; 64]));
        self.count += 1;
    }
}

