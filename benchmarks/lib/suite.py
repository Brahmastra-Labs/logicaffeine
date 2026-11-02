"""The benchmark suite's registry for Python: the same rows lib/suite.sh loads for bash.

    import suite                      # with benchmarks/lib on sys.path
    suite.benchmarks()                # [{id, category, name, description, …, proof, output}, …] in page order
    suite.benchmark_ids()             # ["fib", "ackermann", …]
    suite.collapses()                 # ids the LOGOS optimizer collapses
    suite.languages()                 # [{id, label, color, tier, …}, …]
    suite.compilers()                 # [{id, label, command}, …]
"""

import pathlib

LIB = pathlib.Path(__file__).resolve().parent
ROOT = LIB.parent


def tsv(name):
    """The rows of benchmarks/lib/<name> as header → field dicts; every row is the header's width."""
    lines = (LIB / name).read_text().splitlines()
    header = lines[0].split("\t")
    rows = []
    for line in lines[1:]:
        if not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) != len(header):
            raise ValueError(f"{name}: a row has {len(fields)} fields, the header {len(header)}: {line!r}")
        rows.append(dict(zip(header, fields)))
    return rows


def benchmarks():
    return tsv("benchmarks.tsv")


def benchmark_ids():
    return [b["id"] for b in benchmarks()]


def collapses():
    return [b["id"] for b in benchmarks() if b["collapse"]]


def languages():
    return tsv("languages.tsv")


def compilers():
    return tsv("compilers.tsv")
