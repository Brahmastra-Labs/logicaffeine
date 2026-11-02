#!/usr/bin/env python3
"""Build one third-party candidate of the comparative-synthesis bench and record it in the manifest.

    build-qor-candidate.py <kernel> bambu <fsm|pipe> <interface> [--device D] [--period NS] [--deadline S]
    build-qor-candidate.py <kernel> calyx dahlia [--deadline S]
    build-qor-candidate.py <kernel> gemmini <variant> [--deadline S]
    build-qor-candidate.py <kernel> vortex <variant> [--deadline S]
    build-qor-candidate.py <kernel> spatial <fpga> [--deadline S]

The tool runs at its pin (scripts/qor/qor-tools.toml) on the tracked source in
benchmarks/silicon/qor/kernels/<kernel>/; its RTL lands in benchmarks/silicon/qor/rtl/<kernel>/<candidate>/
(local: generated files carry the tools' own licences and are regenerated, never committed), with the
tool's date stamp removed so a rebuild is byte-identical. The candidate's ports are read back from the RTL
by yosys, its binding comes from the kernel's kernel.json, and its entry in
benchmarks/silicon/qor/candidates.json records the files' SHA3-256, the source, and the exact command.

    bambu    Bambu 2024.10 in the logos-qor-bambu container (scripts/qor/bambu-image.Dockerfile): -O3, synchronous
             active-high reset, `fsm` = scheduled FSM, `pipe` = the top function pipelined at II 1.
             --device defaults to the Nexys A7 part xc7a100t-1csg324-VVD, --period to 10 ns.
    calyx    Dahlia -> Calyx -> `fud2 --to synth-verilog` in the pinned Calyx image.
    gemmini  Gemmini's MeshWithDelays elaborated by chisel3 in the logos-qor-chisel image from the Gemmini and
             berkeley-hardfloat sources at their pins (kernels/<kernel>/gemmini/: build.sbt, Elaborate.scala), and
             driven on the beat schedule of the variant in kernel.json's `gemmini` table.
    vortex   the Vortex GPGPU at its pin, configured out of tree (`configure --xlen`) with the variant's defines
             from kernel.json's `vortex` table, converted to Verilog by sv2v; the kernel (kernels/<kernel>/vortex/
             kernel.S) assembled by clang and linked by ld.lld at the startup address into the image lines the
             harness serves; launched through its KMU as Vortex's runtime launches a kernel, and flushed.

    spatial  Spatial's own Chisel backend at its pin (the commit the Plasticine toolchain runs): the app in kernels/<kernel>/
             spatial/ (kernel.json's `spatial` table) compiled with `--synth --fpga <fpga>` — the target whose latency
             model Spatial retimes for (its generic ASIC target, which this snapshot's command line does not offer,
             retimes by the Zynq one) — and its accelerator elaborated `fringeless`; the build's transcript is kept in
             logs/qor/build/<candidate>.log. The candidate is the generated
             `AccelTop`, driven as Fringe drives it: enable held until its sticky done, each ArgOut taken as it is
             written, the accelerator's reset between calls.

A tool that fails, crashes, or has not finished by the deadline (--deadline, default 7200 s; its container is
killed by name) is recorded in the manifest's `refusals` with the reason, never left as a silent gap.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qorlib  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
QOR = os.path.join(ROOT, "benchmarks", "silicon", "qor")
MANIFEST = os.path.join(QOR, "candidates.json")
PINS = tomllib.load(open(os.path.join(HERE, "qor-tools.toml"), "rb"))
BAMBU_IMAGE = PINS["bambu"]["image"]


def sha3(path):
    return hashlib.sha3_256(open(path, "rb").read()).hexdigest()


def rel(path):
    return os.path.relpath(path, ROOT)


def contained(args, deadline):
    """`docker run --rm <args>` under a name of its own; past `deadline` seconds that container is killed and
    None returned."""
    name = "qor-build-%d-%d" % (os.getpid(), contained.n)
    contained.n += 1
    try:
        return subprocess.run(["docker", "run", "--rm", "--name", name] + args, capture_output=True, text=True,
                              timeout=deadline)
    except subprocess.TimeoutExpired:
        subprocess.run(["docker", "kill", name], capture_output=True)
        return None


contained.n = 0


def run(cmd, cwd=None):
    r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("command failed (%d): %s\n%s\n%s" % (r.returncode, " ".join(cmd), r.stdout[-3000:], r.stderr[-3000:]))
    return r


# A terminal's control sequences as tools colour their transcripts (CSI: ESC [ parameters, final byte), and every other
# control character but a newline or a tab.
CONTROL = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|[\x00-\x08\x0b-\x1f\x7f]")


class Refusal(Exception):
    """The tool refused to build the candidate: recorded in the manifest's `refusals`, never a silent gap. The reason
    is plain text, whatever the tool's transcript carried."""

    def __init__(self, kernel, name, tool, generate, reason):
        reason = CONTROL.sub("", reason)
        super().__init__(reason)
        self.kernel, self.name, self.tool, self.generate, self.reason = kernel, name, tool, generate, reason


def yosys_ports(files, top, sv, cwd=None, closed=False):
    """The top's ports as yosys reads them; with `closed`, every module the hierarchy instantiates must be defined."""
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "ports.json")
        reads = " ".join("read_verilog -defer %s%s;" % ("-sv " if sv else "", f) for f in files)
        check = " -check" if closed else ""
        run(["yosys", "-q", "-p", "%s hierarchy%s -top %s; proc; write_json %s" % (reads, check, top, out)], cwd=cwd)
        mod = json.load(open(out))["modules"][top]["ports"]
    return [{"name": n, "dir": "Out" if p["direction"] == "output" else "In", "bits": len(p["bits"])}
            for n, p in sorted(mod.items())]


def bind(contract, dut, lsb=0, width=0, dut_lsb=0):
    return {"contract": contract, "lsb": lsb, "width": width, "dut": dut, "dut_lsb": dut_lsb}


def mem(m):
    out = {"prefix": m["prefix"], "kind": m["kind"], "contract": m["contract"], "words_per_port": m["words_per_port"]}
    if m.get("dims"):
        out["dims"] = m["dims"]
    return out


def lanes(meta, contract):
    """The DUT ports one wide contract port is split across, with the width of each."""
    return meta["lanes"][contract], meta.get("lane_bits", {}).get(contract, 64)


def bambu(kernel, schedule, iface, device, period, deadline):
    meta = json.load(open(os.path.join(QOR, "kernels", kernel, "kernel.json")))["bambu"][iface]
    top = meta["top"]
    tag = "a7" if device.startswith("xc7a100t") else re.sub(r"[^a-z0-9]", "", device.lower())[:8]
    name = "%s_bambu_%s_%s_%sp%s" % (kernel.lower(), schedule, iface, tag, str(period).replace(".", "_"))
    bambu_dir = os.path.join(ROOT, "tools", "qor", "bambu-%s" % PINS["bambu"]["version"])
    if not os.path.isdir(bambu_dir):
        sys.exit("Bambu is not installed: scripts/qor/fetch-qor-tools.py bambu")
    src_dir = os.path.join(QOR, "kernels", kernel)
    flags = ["--top-fname=%s" % top, "--device-name=%s" % device, "--clock-period=%s" % period,
             "--compiler=I386_CLANG16", "-O3", "--reset-type=sync", "--reset-level=high"]
    if meta["pragmas"]:
        flags.append("--generate-interface=INFER")
    if schedule == "pipe":
        flags.append("-p=%s" % top)
    with tempfile.TemporaryDirectory(dir=os.path.join(ROOT, "logs")) as work:
        for f in os.listdir(src_dir):
            if f.endswith(".c"):
                shutil.copy(os.path.join(src_dir, f), work)
        r = contained(["-u", "%d:%d" % (os.getuid(), os.getgid()), "-v", "%s:/opt/bambu:ro" % bambu_dir,
                       "-v", "%s:/work" % work, BAMBU_IMAGE, meta["file"]] + flags, deadline)
        produced = os.path.join(work, "%s.v" % top)
        if r is None:
            raise Refusal(kernel, name, "bambu", "bambu %s %s" % (meta["file"], " ".join(flags)),
                          "Bambu %s did not finish within %d s" % (PINS["bambu"]["version"], deadline))
        if r.returncode != 0 or not os.path.exists(produced):
            crash = re.search(r"(Segmentation fault|Aborted|error[^\n]*)", r.stdout + r.stderr)
            raise Refusal(kernel, name, "bambu", "bambu %s %s" % (meta["file"], " ".join(flags)),
                          "Bambu %s exited %d: %s" % (PINS["bambu"]["version"], r.returncode, crash.group(1) if crash else "no RTL"))
        text = re.sub(r" - Date \S+", "", open(produced).read())
        roms = {f: open(os.path.join(work, f)).read() for f in sorted(os.listdir(work)) if f.endswith(".mem")}
        report = "\n".join(l for l in r.stdout.splitlines() if re.search(r"cycles|frequency|area|DSPs|flip-flops", l))
    out_dir = os.path.join(QOR, "rtl", kernel, name)
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    path = os.path.join(out_dir, "%s.v" % top)
    open(path, "w").write(text)
    rom_paths = []
    for f, body in roms.items():
        rp = os.path.join(out_dir, f)
        open(rp, "w").write(body)
        rom_paths.append(rp)
    open(os.path.join(out_dir, "bambu-report.txt"), "w").write(report + "\n")
    ports = yosys_ports([path], top, sv=False, cwd=out_dir)
    inputs, outputs = [], []
    for c, d in meta["inputs"]:
        if d is None:
            ports_of, w = lanes(meta, c)
            for j, lane in enumerate(ports_of):
                inputs.append(bind(c, lane, w * j, w))
        else:
            inputs.append(bind(c, d))
    for entry in meta["outputs"]:
        if entry[1] is None:
            ports_of, w = lanes(meta, entry[0])
            for j, lane in enumerate(ports_of):
                outputs.append(bind(entry[0], lane, w * j, w))
        else:
            c, d, dlsb, w = entry
            outputs.append(bind(c, d, 0, w, dlsb))
    if meta.get("strobe"):
        for o in outputs:
            o["valid"] = "%s_vld" % o["dut"]
    protocol = {"StartDone": {"start": "start_port", "done": "done_port"}} if schedule == "fsm" else \
        {"Pipelined": {"start": "start_port", "done": "done_port", "ii": 1}}
    candidate = {
        "kernel": kernel, "name": name, "top": top, "clock": "clock",
        "reset": {"port": "reset", "active_high": True}, "protocol": protocol,
        "inputs": inputs, "outputs": outputs, "mems": [mem(m) for m in meta["mems"]],
        "batch": meta.get("batch", 1), "ports": ports,
    }
    generate = "bambu %s %s" % (meta["file"], " ".join(flags))
    note = {"c": "scalar arguments and the return value as ports",
            "ports": "scalar arguments in, `ovalid` output ports out (each value with its own strobe)",
            "array": "array arguments as dual-port block-RAM interfaces"}[iface]
    return candidate, [path] + rom_paths, [rel(os.path.join(src_dir, meta["file"]))] + \
        ([rel(os.path.join(src_dir, "kernel.c"))] if meta["file"] != "kernel.c" else []), generate, note


def calyx(kernel, deadline):
    meta = json.load(open(os.path.join(QOR, "kernels", kernel, "kernel.json")))["dahlia"]
    name = "%s_calyx_dahlia" % kernel.lower()
    image = PINS["calyx"]["image"]
    src = os.path.join(QOR, "kernels", kernel, meta["file"])
    with tempfile.TemporaryDirectory(dir=os.path.join(ROOT, "logs")) as work:
        shutil.copy(src, os.path.join(work, "kernel.fuse"))
        inner = "cd /w && fud2 kernel.fuse --to synth-verilog -o kernel.sv > fud.log 2>&1; rc=$?; chown -R %d:%d /w; exit $rc" % (
            os.getuid(), os.getgid())
        r = contained(["-v", "%s:/w" % work, image, "bash", "-lc", inner], deadline)
        generate = "fud2 %s --to synth-verilog" % meta["file"]
        if r is None:
            raise Refusal(kernel, name, "calyx", generate, "fud2 did not finish within %d s" % deadline)
        if r.returncode != 0:
            log = open(os.path.join(work, "fud.log")).read() if os.path.exists(os.path.join(work, "fud.log")) else ""
            last = [l for l in log.splitlines() if l.strip() and "WARN" not in l][-1:] or ["no log"]
            raise Refusal(kernel, name, "calyx", generate, "fud2 exited %d: %s" % (r.returncode, last[0]))
        text = open(os.path.join(work, "kernel.sv")).read()
    out_dir = os.path.join(QOR, "rtl", kernel, name)
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    path = os.path.join(out_dir, "%s.sv" % name)
    open(path, "w").write(text)
    ports = yosys_ports([path], "main", sv=True)
    candidate = {
        "kernel": kernel, "name": name, "top": "main", "clock": "clk",
        "reset": {"port": "reset", "active_high": True}, "protocol": {"GoDone": {"go": "go", "done": "done"}},
        "inputs": [], "outputs": [], "mems": [mem(m) for m in meta["mems"]], "batch": meta.get("batch", 1),
        "ports": ports,
    }
    shapes = ", ".join("%s %s" % (m["prefix"], "x".join(str(d) for d in m["dims"]) if m.get("dims") else
                                  "%d-word" % (len(m["contract"]) * m["words_per_port"])) for m in meta["mems"])
    return candidate, [path], [rel(src)], generate, \
        "operands and results through Calyx seq_mem interfaces, Dahlia's calling convention (%s)" % shapes


def gemmini_beats(v):
    """The 8x8 mesh at one row per cycle, a call every `period` beats with its request at beat 0 and bias `d`/`b`
    zero where the dataflow does not use it.

    ws: each call's weight rows preload through `d` (last row first) into the PE buffer the next call computes with,
        while the previous call's activation rows stream through `a` against the weights it preloaded.
    os: each call's activation rows enter through `a` and Gemmini's own transposer (a_transpose 0, its default
        output-stationary path), which hands them to the mesh a call later, when that call's weight rows stream
        through `b`; the accumulated rows leave as the next call shifts in.

    Row t of a call is on io_resp `result_offset + row_step * t` cycles after the call's beat 0."""
    drives = []
    ws = v["dataflow"] == "ws"

    def drive(beat, dut, value):
        drives.append({"beat": beat, "dut": dut, "value": value})

    for t in range(8):
        for port in ("io_a_valid", "io_b_valid", "io_d_valid"):
            drive(t, port, {"Const": 1})
        for k in range(8):
            if ws:
                drive(t, "io_a_bits_%d_0" % k, {"Previous": {"contract": "a@%d" % t, "lsb": 8 * k, "width": 8}})
                drive(t, "io_d_bits_%d_0" % k, {"Slice": {"contract": "w@0", "lsb": 64 * (7 - t) + 8 * k, "width": 8}})
            else:
                drive(t, "io_a_bits_%d_0" % k, {"Slice": {"contract": "a@%d" % t, "lsb": 8 * k, "width": 8}})
                drive(t, "io_b_bits_%d_0" % k, {"Previous": {"contract": "w@0", "lsb": 64 * t + 8 * k, "width": 8}})
    for k in range(8):
        drive(0, "io_%s_bits_%d_0" % ("b" if ws else "d", k), {"Const": 0})
    for port, value in (("io_req_valid", 1), ("io_req_bits_pe_control_dataflow", 1 if ws else 0),
                        ("io_req_bits_pe_control_propagate", 1), ("io_req_bits_pe_control_shift", 0),
                        ("io_req_bits_a_transpose", 0), ("io_req_bits_bd_transpose", 0), ("io_req_bits_total_rows", 8),
                        ("io_req_bits_flush", 0), ("io_req_bits_tag_v", 0)):
        drive(0, port, {"Const": value})
    captures = [v["result_offset"] + v["row_step"] * t for t in range(8)]
    return {"Beats": {"period": v["period"], "drives": drives, "captures": captures}}


def gemmini(kernel, variant, deadline):
    table = json.load(open(os.path.join(QOR, "kernels", kernel, "kernel.json")))["gemmini"]
    v = table[variant]
    name = "%s_gemmini_%s" % (kernel.lower(), variant)
    elab = os.path.join(QOR, "kernels", kernel, "gemmini")
    src = os.path.join(ROOT, "tools", "qor", "gemmini", "src", "main", "scala", "gemmini")
    hf = os.path.join(ROOT, "tools", "qor", "gemmini-hardfloat", "hardfloat", "src", "main", "scala")
    if not (os.path.isdir(src) and os.path.isdir(hf)):
        sys.exit("Gemmini is not installed: scripts/qor/fetch-qor-tools.py gemmini chisel")
    cache = os.path.join(ROOT, "tools", "qor", "chisel-cache")
    os.makedirs(cache, exist_ok=True)
    args = "runMain qor.Elaborate %s %d /w/out" % (v["dataflow"], v["out_bits"])
    generate = "sbt %r on %s, %s (%s.scala) and %s" % (args, os.path.relpath(elab, ROOT), PINS["gemmini"]["version"],
                                                      ", ".join(table["sources"]), PINS["gemmini"]["hardfloat_version"])
    with tempfile.TemporaryDirectory(dir=os.path.join(ROOT, "logs")) as work:
        os.makedirs(os.path.join(work, "src", "main", "scala", "gemmini"))
        os.makedirs(os.path.join(work, "src", "main", "scala", "hardfloat"))
        os.makedirs(os.path.join(work, "project"))
        shutil.copy(os.path.join(elab, "build.sbt"), work)
        shutil.copy(os.path.join(elab, "Elaborate.scala"), os.path.join(work, "src", "main", "scala"))
        open(os.path.join(work, "project", "build.properties"), "w").write("sbt.version=%s\n" % PINS["chisel"]["sbt_version"])
        for f in table["sources"]:
            shutil.copy(os.path.join(src, f + ".scala"), os.path.join(work, "src", "main", "scala", "gemmini"))
        for f in sorted(os.listdir(hf)):
            shutil.copy(os.path.join(hf, f), os.path.join(work, "src", "main", "scala", "hardfloat"))
        r = contained(["-u", "%d:%d" % (os.getuid(), os.getgid()), "-e", "HOME=/cache", "-v", "%s:/cache" % cache,
                       "-v", "%s:/w" % work, "-w", "/w", PINS["chisel"]["image"], "sbt", "-batch",
                       "-Dsbt.global.base=/cache/.sbt", "-Dsbt.ivy.home=/cache/.ivy2", "-Dsbt.boot.directory=/cache/.sbt/boot",
                       args], deadline)
        if r is None:
            raise Refusal(kernel, name, "gemmini", generate, "the Chisel elaboration did not finish within %d s" % deadline)
        produced = os.path.join(work, "out", "MeshWithDelays.v")
        if r.returncode != 0 or not os.path.exists(produced):
            last = [l for l in (r.stdout + r.stderr).splitlines() if "[error]" in l][:1] or ["no Verilog"]
            raise Refusal(kernel, name, "gemmini", generate, "the Chisel elaboration exited %d: %s" % (r.returncode, last[0]))
        text = open(produced).read()
    out_dir = os.path.join(QOR, "rtl", kernel, name)
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    path = os.path.join(out_dir, "MeshWithDelays.v")
    open(path, "w").write(text)
    ports = yosys_ports([path], "MeshWithDelays", sv=False)
    outputs = [bind("o@%d" % t, "io_resp_bits_data_%d_0" % c, 32 * c, 32) for t in range(8) for c in range(8)]
    if v["out_bits"] < 32:
        for o in outputs:
            o["dut_width"] = v["out_bits"]
    candidate = {
        "kernel": kernel, "name": name, "top": "MeshWithDelays", "clock": "clock",
        "reset": {"port": "reset", "active_high": True}, "protocol": gemmini_beats(v),
        "inputs": [], "outputs": outputs, "mems": [], "batch": 8, "ports": ports,
    }
    sources = [os.path.join(elab, "Elaborate.scala"), os.path.join(elab, "build.sbt")]
    note = {"ws": "weight-stationary MeshWithDelays at one row per cycle: a call's request and weight rows (through `d`, "
                  "last row first) in its %d beats while the previous call's activation rows stream through `a`; bias "
                  "`b` zero",
            "os": "output-stationary MeshWithDelays at one row per cycle: a call's request and activation rows (through "
                  "`a` and Gemmini's transposer) in its %d beats while the previous call's weight rows stream through "
                  "`b`; preload `d` zero; rows leave last first"}[v["dataflow"]] % v["period"]
    note += "; spatialArrayOutputType SInt(%d), accType SInt(32)" % v["out_bits"]
    note += "; results taken from io_resp at fixed offsets; a one-bit tag stands in for the ExecuteController's mesh_tag"
    return candidate, [path], [rel(s) for s in sources], generate, note


VX_DCR_KMU = {"STARTUP_ADDR0": 0x010, "STARTUP_ADDR1": 0x011, "STARTUP_ARG0": 0x014, "STARTUP_ARG1": 0x015,
              "BLOCK_DIM_X": 0x016, "BLOCK_DIM_Y": 0x017, "BLOCK_DIM_Z": 0x018, "GRID_DIM_X": 0x019,
              "GRID_DIM_Y": 0x01A, "GRID_DIM_Z": 0x01B, "LMEM_SIZE": 0x01C, "BLOCK_SIZE": 0x01D,
              "WARP_STEP_X": 0x01E, "WARP_STEP_Y": 0x01F, "WARP_STEP_Z": 0x020, "CLUSTER_DIM_X": 0x021,
              "CLUSTER_DIM_Y": 0x022, "CLUSTER_DIM_Z": 0x023}
VX_DCR_BASE_CACHE_FLUSH = 0x000


def spatial(kernel, fpga, deadline):
    """Spatial's Chisel backend on the kernel's own Spatial app. Its ArgIns take the contract slots kernel.json's
    `arg_ins` names in order and its ArgOuts give those `arg_outs` names, each `[slot, lsb, width]`."""
    meta = json.load(open(os.path.join(QOR, "kernels", kernel, "kernel.json")))["spatial"]
    name = "%s_spatial_%s" % (kernel.lower(), fpga.lower())
    tool = os.path.join(ROOT, "tools", "qor", "spatial")
    if not os.path.isdir(os.path.join(tool, "fringe")):
        sys.exit("Spatial is not installed: scripts/qor/fetch-qor-tools.py spatial")
    src = os.path.join(QOR, "kernels", kernel, meta["file"])
    app = meta["app"]
    cache = os.path.join(ROOT, "tools", "qor", "spatial-cache")
    os.makedirs(cache, exist_ok=True)
    sbt = ("sbt -batch -Dsbt.global.base=/cache/.sbt -Dsbt.ivy.home=/cache/.ivy2 -Dsbt.boot.directory=/cache/.sbt/boot")
    generate = ("Spatial %s: %s compiled `--synth --fpga %s`, its accelerator elaborated by `runMain top.Instantiator "
                "--verilog --testArgs fringeless`" % (PINS["spatial"]["version"], rel(src), fpga))
    script = "\n".join([
        "set -e",
        "trap 'rm -f /s/apps/src/%s.scala' EXIT" % app,
        "cd /s",
        "bash bin/update_resources.sh",
        "%s emul/publishLocal utils/publishLocal models/publishLocal fringe/publishLocal" % sbt,
        "cp /w/%s.scala apps/src/%s.scala" % (app, app),
        "%s 'apps/runMain %s --synth --fpga %s --out /w/gen'" % (sbt, app, fpga),
        "cd /w/gen && mkdir -p project && echo sbt.version=%s > project/build.properties" % PINS["spatial"]["sbt_version"],
        "%s 'runMain top.Instantiator --verilog --testArgs fringeless'" % sbt,
    ])
    with qorlib.locked("spatial"), tempfile.TemporaryDirectory(dir=os.path.join(ROOT, "logs")) as work:
        shutil.copy(src, os.path.join(work, app + ".scala"))
        r = contained(["-u", "%d:%d" % (os.getuid(), os.getgid()), "-e", "HOME=/cache", "-v", "%s:/cache" % cache,
                       "-v", "%s:/s" % tool, "-v", "%s:/w" % work, PINS["spatial"]["image"], "bash", "-c", script],
                      deadline)
        if r is None:
            raise Refusal(kernel, name, "spatial", generate, "the Spatial build did not finish within %d s" % deadline)
        produced = os.path.join(work, "gen", "verilog-fringeless", "Top.v")
        transcript = CONTROL.sub("", r.stdout + r.stderr)
        os.makedirs(os.path.join(qorlib.LOGS, "qor", "build"), exist_ok=True)
        open(os.path.join(qorlib.LOGS, "qor", "build", name + ".log"), "w").write(transcript)
        if r.returncode != 0 or not os.path.exists(produced):
            lines = [l.strip() for l in transcript.splitlines()]
            # Spatial reports its own failure as `[bug]` lines, the cause indented under the first.
            cause = [l[len("[bug]"):].strip() for l in lines if l.startswith("[bug]   ")][:1]
            errors = cause or [l for l in lines if l.startswith("[error]")][:1] or ["no Verilog"]
            raise Refusal(kernel, name, "spatial", generate, "the Spatial build exited %d: %s" % (r.returncode, errors[0]))
        text = open(produced).read()
        # The black boxes the accelerator instantiates (RetimeShiftRegister, …) from the resources Spatial copied
        # beside the app for its target.
        defined = set(re.findall(r"^module (\w+)", text, re.M))
        boxes = []
        for d, _, names in sorted(os.walk(os.path.join(work, "gen"))):
            if ".hw-resources" not in d:
                continue
            for n in sorted(names):
                if not n.endswith((".v", ".sv")):
                    continue
                f = os.path.join(d, n)
                mods = set(re.findall(r"^module (\w+)", open(f, errors="replace").read(), re.M))
                wanted = [m for m in mods - defined if re.search(r"\b%s\b" % m, text)]
                if wanted and n not in [os.path.basename(b) for b in boxes]:
                    boxes.append(f)
                    defined |= mods
        box_text = [(os.path.basename(f), open(f).read()) for f in boxes]
    out_dir = os.path.join(QOR, "rtl", kernel, name)
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    path = os.path.join(out_dir, "Top.v")
    open(path, "w").write(text)
    paths = [path]
    for n, t in box_text:
        paths.append(os.path.join(out_dir, n))
        open(paths[-1], "w").write(t)
    ports = yosys_ports(paths, "AccelTop", sv=True, closed=True)
    inputs = [bind(slot, "io_argIns_%d" % k, lsb, width) for k, (slot, lsb, width) in enumerate(meta["arg_ins"])]
    outputs = []
    for k, (slot, lsb, width) in enumerate(meta["arg_outs"]):
        o = bind(slot, "io_argOuts_%d_port_bits" % k, lsb, width)
        o["valid"] = "io_argOuts_%d_port_valid" % k
        outputs.append(o)
    candidate = {
        "kernel": kernel, "name": name, "top": "AccelTop", "clock": "clock",
        "reset": {"port": "reset", "active_high": True},
        "protocol": {"EnableDone": {"enable": "io_enable", "done": "io_done", "clear": "io_reset"}},
        "inputs": inputs, "outputs": outputs, "mems": [], "batch": meta["batch"], "ports": ports,
    }
    note = ("Spatial's fringeless AccelTop driven as Fringe drives it: enable held until its sticky done, each ArgOut "
            "taken as it is written, the accelerator's own reset between calls; %d ArgIns in, %d ArgOuts out; the "
            "memory-stream, heap and echo inputs Fringe would drive, unused by the app, held at 0; %s from the "
            "target's resources" % (len(inputs), len(outputs), ", ".join(n for n, _ in box_text) or "no black boxes"))
    return candidate, paths, [rel(src)], generate, note


def define_value(defines, name):
    for d in defines:
        if d.startswith(name + "="):
            return int(d.split("=", 1)[1], 0)
    return None


def vortex(kernel, variant, deadline):
    table = json.load(open(os.path.join(QOR, "kernels", kernel, "kernel.json")))["vortex"]
    v = table[variant]
    name = "%s_vortex_%s" % (kernel.lower(), variant)
    home = os.path.join(ROOT, "tools", "qor", "vortex")
    sv2v = os.path.join(ROOT, "tools", "qor", "sv2v-%s" % PINS["sv2v"]["version"], "sv2v")
    if not (os.path.isdir(home) and os.path.exists(sv2v)):
        sys.exit("Vortex or sv2v is not installed: scripts/qor/fetch-qor-tools.py vortex sv2v")
    xlen, batch = v["xlen"], v["batch"]
    threads = define_value(v["defines"], "VX_CFG_NUM_THREADS")
    warps = define_value(v["defines"], "VX_CFG_NUM_WARPS")
    defines = ["VX_CFG_XLEN=%d" % xlen, "VX_CFG_XLEN_%d" % xlen] + v["defines"] + ["SYNTHESIS", "YOSYS", "NDEBUG"]
    src = os.path.join(QOR, "kernels", kernel, table["kernel"])
    startup = int(table["startup_addr"], 0)
    if batch % warps:
        sys.exit("%s: a batch of %d does not divide among %d warps" % (name, batch, warps))
    generate = "Vortex %s configure --xlen=%d; sv2v --top=Vortex %s; clang -march=rv%dim -DBATCH=%d -DWARPS=%d %s; ld.lld -Ttext=%#x" % (
        PINS["vortex"]["version"], xlen, " ".join("-D" + d for d in defines), xlen, batch, warps, rel(src), startup)
    with tempfile.TemporaryDirectory(dir=os.path.join(ROOT, "logs")) as work:
        build = os.path.join(work, "build")
        os.makedirs(build)
        r = subprocess.run([os.path.join(home, "configure"), "--xlen=%d" % xlen, "--tooldir=%s" % os.path.join(work, "tools")],
                           cwd=build, env=dict(os.environ, XLEN=str(xlen)), capture_output=True, text=True)
        if r.returncode != 0:
            raise Refusal(kernel, name, "vortex", generate, "configure exited %d" % r.returncode)
        rtl = os.path.join(home, "hw", "rtl")
        dirs = [rtl] + [os.path.join(rtl, d) for d in ("libs", "interfaces", "core", "mem", "vm", "cache")]
        files = sorted(os.path.join(d, f) for d in dirs for f in os.listdir(d) if f.endswith((".sv", ".v")))
        out = os.path.join(work, "vortex.v")
        try:
            r = subprocess.run([sv2v, "--top=Vortex", "-I" + os.path.join(build, "hw"), "-I" + os.path.join(build, "sw")]
                               + ["-I" + d for d in dirs] + ["-D" + d for d in defines] + files + ["-w", out],
                               capture_output=True, text=True, timeout=deadline)
        except subprocess.TimeoutExpired:
            raise Refusal(kernel, name, "vortex", generate, "sv2v did not finish within %d s" % deadline)
        if r.returncode != 0 or not os.path.exists(out):
            raise Refusal(kernel, name, "vortex", generate, "sv2v exited %d: %s" % (r.returncode, (r.stderr.strip().splitlines() or ["no Verilog"])[0]))
        text = open(out).read()
        obj, elf, binary = (os.path.join(work, f) for f in ("kernel.o", "kernel.elf", "kernel.bin"))
        run(["clang", "--target=riscv%d-unknown-elf" % xlen, "-march=rv%dim" % xlen, "-mabi=%s" % ("lp64" if xlen == 64 else "ilp32"),
             "-DBATCH=%d" % batch, "-DWARPS=%d" % warps, "-c", src, "-o", obj])
        run(["ld.lld", "-Ttext=%#x" % startup, "-e", "_start", obj, "-o", elf])
        run(["llvm-objcopy", "-O", "binary", elf, binary])
        image = open(binary, "rb").read()
    image += bytes((-len(image)) % 64)
    first_line = startup // 64
    lines = [{"line": first_line + i, "hex": image[64 * i:64 * i + 64][::-1].hex()} for i in range(len(image) // 64)]
    out_dir = os.path.join(QOR, "rtl", kernel, name)
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    path = os.path.join(out_dir, "Vortex.v")
    open(path, "w").write(text)
    ports = yosys_ports([path], "Vortex", sv=False)
    width = {p["name"]: p["bits"] for p in ports}
    nports = width["mem_req_valid"]
    ins = ["a", "b", "mask", "x0", "x1", "x2", "x3"]
    outs = ["y0", "y1", "y2", "y3", "sum"]
    regions = [
        {"base_line": int(table["operand_line"], 0), "contract": ["%s@%d" % (p, j) for j in range(batch) for p in ins],
         "words_per_port": 1, "stride_bits": xlen},
        {"base_line": int(table["result_line"], 0), "contract": ["%s@%d" % (p, j) for j in range(batch) for p in outs],
         "words_per_port": 1, "stride_bits": xlen},
    ]
    block = threads * warps
    kmu = [("STARTUP_ADDR0", startup & 0xffffffff), ("STARTUP_ADDR1", startup >> 32), ("STARTUP_ARG0", 0), ("STARTUP_ARG1", 0),
           ("BLOCK_DIM_X", block), ("BLOCK_DIM_Y", 1), ("BLOCK_DIM_Z", 1), ("GRID_DIM_X", 1), ("GRID_DIM_Y", 1),
           ("GRID_DIM_Z", 1), ("LMEM_SIZE", 0), ("BLOCK_SIZE", block), ("WARP_STEP_X", threads % block),
           ("WARP_STEP_Y", 0), ("WARP_STEP_Z", 0), ("CLUSTER_DIM_X", 1), ("CLUSTER_DIM_Y", 1), ("CLUSTER_DIM_Z", 1)]
    protocol = {"Launch": {
        "start": "start", "busy": "busy",
        "dcr": {"valid": "dcr_req_valid", "rw": "dcr_req_rw", "addr": "dcr_req_addr", "data": "dcr_req_data",
                "rsp_valid": "dcr_rsp_valid"},
        "setup": [{"addr": VX_DCR_KMU[k], "data": val} for k, val in kmu],
        "flush": VX_DCR_BASE_CACHE_FLUSH,
        "bus": {"ports": nports, "line_bytes": width["mem_req_data"] // nports // 8,
                "addr_bits": width["mem_req_addr"] // nports, "tag_bits": width["mem_req_tag"] // nports,
                "msb_first": True, "prefix": "mem", "image": lines, "regions": regions},
    }}
    candidate = {
        "kernel": kernel, "name": name, "top": "Vortex", "clock": "clk",
        "reset": {"port": "reset", "active_high": True}, "protocol": protocol,
        "inputs": [], "outputs": [], "mems": [], "batch": batch, "ports": ports,
    }
    split = "one warp takes every operation" if warps == 1 else "the %d warps take operations in turn" % warps
    note = ("Vortex %s: %s; the kernel (%d instructions) launched through the KMU for %d operations a call, each "
            "operation's four lanes a warp's four threads, %s, the sum by butterfly shuffles; operands and results in "
            "64-bit words of the harness's memory, results read after the CACHE_FLUSH read Vortex's runtime issues" % (
                PINS["vortex"]["version"], ", ".join(v["defines"]), len(image) // 4, batch, split))
    return candidate, [path], [rel(src)], generate, note


def load_manifest():
    doc = json.load(open(MANIFEST)) if os.path.exists(MANIFEST) else {"schema": "logos.qor.candidates/1", "candidates": []}
    doc.setdefault("refusals", [])
    return doc


def save_manifest(doc):
    """Write the manifest whole or not at all."""
    tmp = os.path.join(qorlib.LOGS, "qor_candidates.json.tmp")
    with open(tmp, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
        f.write("\n")
    os.replace(tmp, MANIFEST)


def record_refusal(r):
    with qorlib.locked("candidates"):
        doc = load_manifest()
        doc["refusals"] = sorted([x for x in doc["refusals"] if x["name"] != r.name] +
                                 [{"kernel": r.kernel, "name": r.name, "tool": r.tool, "generate": r.generate, "reason": r.reason}],
                                 key=lambda x: x["name"])
        doc["candidates"] = [e for e in doc["candidates"] if e["candidate"]["name"] != r.name]
        save_manifest(doc)
    print("REFUSED %s: %s" % (r.name, r.reason))


def record(tool, candidate, paths, sources, generate, note):
    entry = {
        "candidate": candidate, "tool": tool, "source": " ".join(sources), "generate": generate,
        "files": [{"path": rel(p), "sha3_256": sha3(p)} for p in paths], "interface_note": note,
    }
    with qorlib.locked("candidates"):
        doc = load_manifest()
        doc["refusals"] = [x for x in doc["refusals"] if x["name"] != candidate["name"]]
        doc["candidates"] = sorted([e for e in doc["candidates"] if e["candidate"]["name"] != candidate["name"]] + [entry],
                                   key=lambda e: e["candidate"]["name"])
        save_manifest(doc)
    print(candidate["name"])


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("kernel")
    ap.add_argument("tool", choices=["bambu", "calyx", "gemmini", "vortex", "spatial"])
    ap.add_argument("variant", nargs="+")
    ap.add_argument("--device", default="xc7a100t-1csg324-VVD")
    ap.add_argument("--period", default="10")
    ap.add_argument("--deadline", type=int, default=7200, help="seconds the tool may run")
    a = ap.parse_args()
    try:
        if a.tool == "bambu":
            if len(a.variant) != 2 or a.variant[0] not in ("fsm", "pipe"):
                sys.exit("bambu takes <fsm|pipe> <interface>")
            record("bambu", *bambu(a.kernel, a.variant[0], a.variant[1], a.device, a.period, a.deadline))
        elif a.tool == "calyx":
            if a.variant != ["dahlia"]:
                sys.exit("calyx takes the frontend `dahlia`")
            record("calyx", *calyx(a.kernel, a.deadline))
        elif a.tool == "gemmini":
            if len(a.variant) != 1:
                sys.exit("gemmini takes one variant of kernel.json's `gemmini` table")
            record("gemmini", *gemmini(a.kernel, a.variant[0], a.deadline))
        elif a.tool == "spatial":
            if len(a.variant) != 1:
                sys.exit("spatial takes the one target it compiles for (one of Spatial's `--fpga` names, e.g. Zynq)")
            record("spatial", *spatial(a.kernel, a.variant[0], a.deadline))
        else:
            if len(a.variant) != 1:
                sys.exit("vortex takes one variant of kernel.json's `vortex` table")
            record("vortex", *vortex(a.kernel, a.variant[0], a.deadline))
    except Refusal as r:
        record_refusal(r)
        sys.exit(3)


if __name__ == "__main__":
    main()
