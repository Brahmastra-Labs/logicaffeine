#!/usr/bin/env python3
"""A prefix of a lean4export NDJSON file that is itself a valid export.

lean4export writes every name, level, expression and declaration before its first use, so any
prefix that ends on a record boundary only refers to what it contains. The prefix must also not end
inside a mutual block (the reader holds such a block until all its members arrive).

    scripts/lean/arena-crown/slice-export.py <export.ndjson> <out.ndjson> <bytes>
    scripts/lean/arena-crown/slice-export.py <export.ndjson> <out.ndjson> --decl <index>

With <bytes> (a K/M/G suffix means powers of 1000) the cut is made just after the first theorem, at
or past that size, that is alone in its `all` list. With --decl the cut is made just after the
declaration record with that 0-based index — the `index` column of `logos_kernel --decl-costs` — so
the slice ends with that declaration, which is then the last thing checked. Prints the cut.
"""
import re
import sys

LONE_THEOREM = re.compile(rb'^\{"thm":\{"all":\[\d+\],')
DECLARATION = re.compile(rb'^\{"(?:thm|def|axiom|opaque|quot|inductive)":')


def parse_size(text):
    scale = {"K": 10**3, "M": 10**6, "G": 10**9}
    if text[-1:].upper() in scale:
        return int(float(text[:-1]) * scale[text[-1:].upper()])
    return int(text)


def main():
    args = sys.argv[1:]
    if len(args) == 4 and args[2] == "--decl":
        src, dst, target, by_index = args[0], args[1], int(args[3]), True
    elif len(args) == 3:
        src, dst, target, by_index = args[0], args[1], parse_size(args[2]), False
    else:
        sys.exit(__doc__)
    written = lines = decls = 0
    with open(src, "rb") as fin, open(dst, "wb") as fout:
        for line in fin:
            fout.write(line)
            written += len(line)
            lines += 1
            if DECLARATION.match(line):
                decls += 1
                if by_index and decls == target + 1:
                    break
                if not by_index and written >= target and LONE_THEOREM.match(line):
                    break
        else:
            print(f"{src} ended before the cut; copied it whole", file=sys.stderr)
    print(f"{dst}: {written} bytes, {lines} lines, {decls} declarations")


if __name__ == "__main__":
    main()
