//! board_host_portability_lock — every FPGA board host finds and flashes its board the same way on Linux and
//! macOS, through `scripts/lib/ftdi_ports.py`.
//!
//! A board's FT2232 channels have no stable device name. Linux numbers `/dev/ttyUSBn` in enumeration order —
//! this box's two boards have held 0/1 and 2/3 both ways round, so a host defaulting to `/dev/ttyUSB3` talks
//! to whichever board enumerated second — and macOS names the nodes after the board's serial. The JTAG cable
//! differs per board (`digilent` on the Nexys A7, `digilent_b` on the Nexys Video), and so does which channel
//! carries the bytes: JTAG holds channel A on the Nexys A7 and channel B on the Nexys Video, the byte channel is
//! the other one. The loader needs `sudo` on Linux but must not ask for a password on macOS. `ftdi_ports.py` is
//! the one place that knows each of these; every other script asks it. This lock holds every script under
//! `scripts/` to that: no Linux device node, no udev lookup, no FTDI channel opened by an interface number of its
//! own, no loader invocation of its own, and every host that opens a serial port finds its board through the
//! module.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// The one file that may name device nodes, udev and the loader: it is where each of them is decided.
const THE_RESOLVER: &str = "scripts/lib/ftdi_ports.py";

/// Every `.py` and `.sh` under `scripts/`, as repo-relative paths, sorted.
fn scripts() -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).expect("read scripts/") {
            let p = e.expect("dir entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("py") | Some("sh")
            ) {
                out.push(p);
            }
        }
    }
    let mut found = Vec::new();
    walk(&root().join("scripts"), &mut found);
    let mut rel: Vec<String> = found
        .iter()
        .map(|p| {
            p.strip_prefix(root())
                .expect("under the repo")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    rel.sort();
    rel
}

/// What a line does that only the resolver may do, if anything.
fn bypass(line: &str) -> Option<&'static str> {
    let node = line.match_indices("/dev/ttyUSB").any(|(i, m)| {
        line[i + m.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    });
    if node {
        return Some("names a Linux device node by number");
    }
    if line.contains("/dev/serial/by-id") || line.contains("udevadm") {
        return Some("finds a port through udev, which macOS does not have");
    }
    if line.contains("ftdi://") {
        return Some("opens an FTDI channel by an interface number of its own, not the board's byte channel");
    }
    let loader = line.contains("openFPGALoader -c")
        || line.contains("openFPGALoader\", \"-c\"")
        || line.contains("openFPGALoader', '-c'")
        || line.contains("sudo openFPGALoader")
        || line.contains("sudo -n openFPGALoader");
    if loader {
        return Some("runs the loader itself instead of `ftdi_ports.py flash` (cable and sudo per board and OS)");
    }
    None
}

#[test]
fn no_script_names_a_device_node_or_runs_the_loader_itself() {
    let mut offenders = Vec::new();
    for rel in scripts() {
        if rel == THE_RESOLVER {
            continue;
        }
        let text = std::fs::read_to_string(root().join(&rel)).expect(&rel);
        for (n, line) in text.lines().enumerate() {
            if let Some(why) = bypass(line) {
                offenders.push(format!("{rel}:{}: {why}\n        {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "\n{} line(s) under scripts/ bypass scripts/lib/ftdi_ports.py:\n  {}\n\n\
         Find a board's port with `ftdi_ports.byte_port(serial)` / `ftdi_ports.port_arg(...)` in Python or\n\
         `python3 scripts/lib/ftdi_ports.py port A|B` in shell, and flash with\n\
         `python3 scripts/lib/ftdi_ports.py flash A|B <bit>`.\n",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// A host that opens a serial port reaches its board through the resolver, so it runs unchanged on a machine
/// whose device nodes are named differently. The one exception is a module another host loads to decode a
/// reply, which never opens a port of its own choosing.
#[test]
fn every_host_that_opens_a_serial_port_finds_its_board_through_the_resolver() {
    let mut missing = Vec::new();
    for rel in scripts() {
        if rel == THE_RESOLVER || !rel.ends_with(".py") {
            continue;
        }
        let text = std::fs::read_to_string(root().join(&rel)).expect(&rel);
        if text.contains("serial.Serial(") && !text.contains("import ftdi_ports") {
            missing.push(rel);
        }
    }
    assert!(
        missing.is_empty(),
        "\nhost(s) that open a serial port without scripts/lib/ftdi_ports.py:\n  {}\n",
        missing.join("\n  ")
    );
}

/// The census is not vacuous: it sees the hosts it governs and the resolver it exempts, and its line rule
/// recognises each bypass it names.
#[test]
fn the_census_reads_the_hosts_it_governs() {
    let all = scripts();
    for must in [
        THE_RESOLVER,
        "scripts/logos-chip.py",
        "scripts/run-on-nexys-full.sh",
        "scripts/ddr2-dbquery-read.py",
    ] {
        assert!(
            all.iter().any(|s| s == must),
            "the census does not see {must}"
        );
    }
    assert!(bypass("    ap.add_argument(\"--port\", default=\"/dev/ttyUSB3\")").is_some());
    assert!(bypass("SERIAL=210292BFA310; PORT=/dev/ttyUSB1").is_some());
    assert!(bypass("BYID = \"/dev/serial/by-id/usb-Digilent\"").is_some());
    assert!(bypass("    ftdi.open_from_url(f\"ftdi://ftdi:2232:{serial}/2\")").is_some());
    assert!(bypass("sudo openFPGALoader -c digilent --ftdi-serial \"$SERIAL\" \"$BIT\"").is_some());
    assert!(bypass("        [\"sudo\", \"-n\", \"openFPGALoader\", \"-c\", cable]").is_some());
    assert!(
        bypass("the board's /dev/ttyUSB UART").is_none(),
        "prose naming the node family is not a node"
    );
    assert!(bypass("python3 scripts/lib/ftdi_ports.py flash B \"$SIL/$BIT\"").is_none());
}
