#!/usr/bin/env python3
"""Refuse a design whose large memories are written where Quartus may build them in flip-flops, or whose shallow ones
ask for the block RAM their depth leaves empty.

    ram_check.py <top> <least bits> <source.v>...
        The top elaborated at its parameters (yosys `hierarchy`, every module kept as it is, each memory as its
        declaration sizes it and its `ramstyle` as its parameters make it): every memory of at least <least bits> bits
        is a block-RAM wrapper's own — one of
        rtl::ram's memories (logos_bram_w*, logos_bram_re_w*, logos_ram_w*, logos_ram_ce_w*, and the `_part` stacks a
        memory no power of two of layers deep is built of: one write port, a read registered on a clock edge, the form
        Quartus builds in M20K blocks) or a clock-crossing FIFO's
        (logos_dcfifo_m20k, logos_dcfifo_mlab) — or is declared at its module's own scope. A large memory declared in a
        generate block (its name scoped, `held[0].act`) is refused: the vector side's held float-matrix inputs, four of
        1088 x 512 bits in a generate block, were built in 2.2 Mbit of flip-flops behind 1088-way read multiplexers and
        reported nowhere, where the same rows as wrappers are four M20K memories. And every memory of two to
        thirty-two words (an MLAB's depth) asks for MLABs or registers (`ramstyle` "MLAB" or "logic", with or without
        "no_rw_check"): one asking for nothing or for M20K blocks is built by Quartus in an M20K block for each forty
        bits of its width, its depth a sixteenth of the block's or less — hybrid9c2 spent 5,142 blocks, 44% of the
        device's, on 2.2 Mbit of such memories (a column core's ring of two 5,280-bit beats, 132 blocks a core), and the
        cores spread over the blocks' columns failed to route. A shallow memory in MLABs read without a register after
        it (a read port yosys's `memory_dff` leaves unclocked, or clocked at a registered address and so seeing the
        clock's write: transparent) also asks for no read-during-write check: Quartus builds an MLAB's read only through
        its output register otherwise, and builds such a memory in flip-flops, reported only as "uninferred due to
        unsupported read-during-write behavior" (a probe's 32 x 1,056-bit buffer: 33,797 registers). Each refused memory
        is named with its module, its width and its depth, and a shallow one with what it asks for.

Exit 0 and one summary line when every large memory is a wrapper's or its module's own and every shallow one asks for
MLABs or registers; exit 1 naming each one that does not.
"""
import os
import re
import subprocess
import sys
import tempfile

WRAPPER = re.compile(r"(^|\\|\$paramod\$[0-9a-f]*\\)logos_((bram|bram_re|ram|ram_ce)_w\d+(_part)?|dcfifo_(m20k|mlab))$")

# The deepest memory an MLAB holds whole, and the styles a memory that shallow may ask for.
MLAB_WORDS = 32
SHALLOW_STYLES = ("mlab", "logic")


def run(script, d):
    """`script` run by yosys in `d`, the run refused loudly when yosys fails."""
    ys = os.path.join(d, "ram_check.ys")
    open(ys, "w").write(script)
    r = subprocess.run(["yosys", "-q", "-s", ys], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"yosys failed:\n{r.stdout}\n{r.stderr}")


def memories(top, sources):
    """Every memory of the elaborated top: (module, memory, width, depth, ramstyle), the module as yosys names it and the
    ramstyle as its parameters make it ('' when it asks for none); and the reads of every memory of a module holding one
    of three to thirty-two words in MLABs with a read-during-write check, as yosys finds them once `opt_dff` has made each
    register's hold its enable and `memory_dff` has merged each register a read port's address or word passes through: (module, memory, depth, ramstyle, whether every read port
    is clocked and opaque to the clock's write). Only those modules are put through `proc`, which takes most of an hour
    over a whole engine."""
    read = "".join(f"read_verilog -sv {s}\n" for s in sources) + f"hierarchy -top {top}\n"
    with tempfile.TemporaryDirectory() as d:
        listed, dumped = os.path.join(d, "listed.txt"), os.path.join(d, "dumped.txt")
        run(read + f"tee -q -o {listed} select -list m:*\ntee -q -o {dumped} dump m:*\n", d)
        names = [l.strip() for l in open(listed) if l.strip()]
        text = open(dumped).read()
    # The dump gives each memory's width and depth without its module; the list gives each memory's module, in the
    # same order.
    # A memory one bit wide is dumped with no width; its attributes are the lines before it.
    dims, attributes = [], {}
    for line in text.splitlines():
        a = re.match(r"^\s*attribute \\(\S+) (.*)$", line)
        if a:
            attributes[a.group(1)] = a.group(2).strip().strip('"')
            continue
        m = re.match(r"^\s*memory (?:width (\d+) )?size (\d+) (?:offset -?\d+ )?(\S+)\s*$", line)
        if m:
            dims.append((m.group(3).lstrip("\\"), int(m.group(1) or 1), int(m.group(2)), attributes.get("ramstyle", "")))
        attributes = {}
    if len(names) != len(dims):
        sys.exit(f"yosys listed {len(names)} memories and dumped {len(dims)}")
    found = []
    for name, (memory, width, depth, style) in zip(names, dims):
        module, _, listed_memory = name.rpartition("/")
        if listed_memory.lstrip("\\") != memory:
            sys.exit(f"the list's {name} is not the dump's {memory}")
        found.append((module, memory, width, depth, style))
    checked = sorted({m[0] for m in found if 3 <= m[3] <= MLAB_WORDS and "mlab" in m[4].lower() and "no_rw_check" not in m[4].lower()})
    if not checked:
        return found, []
    with tempfile.TemporaryDirectory() as d:
        cells_listed, cells = os.path.join(d, "cells_listed.txt"), os.path.join(d, "cells.txt")
        chosen = " ".join(checked)
        run(read + f"proc {chosen}\nopt_dff {chosen}\nopt_clean {chosen}\nmemory_dff {chosen}\nmemory_collect {chosen}\n"
            + f"tee -q -o {cells_listed} select -list t:$mem_v2\ntee -q -o {cells} dump t:$mem_v2\n", d)
        reads = read_ports([l.strip() for l in open(cells_listed) if l.strip()], open(cells).read())
    return found, reads


def read_ports(names, text):
    """Each `$mem_v2` cell of a dump, its module from the list of the same cells in the same order: (module, memory,
    depth, ramstyle, every read port clocked and not transparent)."""
    cells, style = [], ""
    for line in text.splitlines():
        a = re.match(r"^\s*attribute \\ramstyle \"(.*)\"\s*$", line)
        if a:
            style = a.group(1)
        elif re.match(r"^\s*cell \$mem_v2 ", line):
            cells.append({"ramstyle": style})
            style = ""
        elif re.match(r"^\s*(cell|wire|module|end)\b", line):
            style = ""
        else:
            p = re.match(r"^\s*parameter \\(\S+) (.*)$", line)
            if p and cells:
                cells[-1][p.group(1)] = p.group(2).strip()
    if len(names) != len(cells):
        sys.exit(f"yosys listed {len(names)} memory cells and dumped {len(cells)}")
    found = []
    for name, cell in zip(names, cells):
        ports = int(cell.get("RD_PORTS", "0"))
        clocked = cell.get("RD_CLK_ENABLE", "0").split("'")[-1]
        transparent = cell.get("RD_TRANSPARENCY_MASK", "0").split("'")[-1]
        registered = ports > 0 and clocked == "1" * ports and "1" not in transparent
        memid = cell.get("MEMID", '""').strip('"').lstrip("\\")
        found.append((name.rpartition("/")[0], memid, int(cell.get("SIZE", "0")), cell["ramstyle"], registered))
    return found

def asks_for_what_its_depth_fills(style):
    """Whether a memory of thirty-two words or fewer asking for `style` is built in MLABs or registers."""
    asked = [s.strip().lower() for s in style.split(",")]
    return any(s in SHALLOW_STYLES for s in asked) and not any(s in ("m20k", "m9k", "m10k", "m144k") for s in asked)


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    top, least, sources = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
    mems, reads = memories(top, sources)
    large = [m for m in mems if m[2] * m[3] >= least]
    wrapped = [m for m in large if WRAPPER.search(m[0])]
    scoped = [m for m in large if m not in wrapped and "." in m[1]]
    shallow = [m for m in mems if 2 <= m[3] <= MLAB_WORDS]
    unfilled = [m for m in shallow if not asks_for_what_its_depth_fills(m[4])]
    unchecked = sorted({(module, name, depth, style) for module, name, depth, style, registered in reads
                        if 3 <= depth <= MLAB_WORDS and "mlab" in style.lower() and "no_rw_check" not in style.lower()
                        and not registered})
    if scoped:
        print(f"{top}: {len(scoped)} memories of {least} bits or more in a generate block, outside a block-RAM wrapper:")
        for module, name, width, depth, _ in sorted(scoped, key=lambda m: -m[2] * m[3]):
            print(f"  {module.lstrip(chr(92))} {name}: {width} x {depth} = {width * depth} bits")
    if unfilled:
        print(f"{top}: {len(unfilled)} memories of {MLAB_WORDS} words or fewer ask for neither MLABs nor registers, where "
              f"Quartus builds each in an M20K block for every forty bits of its width:")
        for module, name, width, depth, style in sorted(unfilled, key=lambda m: (-m[2], m[0], m[1])):
            print(f"  {module.lstrip(chr(92))} {name}: {width} x {depth}, asking for {style or 'nothing'}")
    if unchecked:
        print(f"{top}: {len(unchecked)} memories in MLABs read without a register after them ask for a read-during-write "
              f"check, where Quartus builds each in flip-flops:")
        for module, name, depth, style in unchecked:
            print(f"  {module.lstrip(chr(92))} {name}: {depth} words, asking for {style}")
    if scoped or unfilled or unchecked:
        sys.exit(1)
    print(f"{top}: clean; {len(mems)} memories, {len(large)} of {least} bits or more: {len(wrapped)} block-RAM wrappers', "
          f"{len(large) - len(wrapped)} their modules' own; {len(shallow)} of {MLAB_WORDS} words or fewer, each in MLABs or "
          f"registers")


if __name__ == "__main__":
    main()
