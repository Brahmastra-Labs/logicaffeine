#!/usr/bin/env python3
"""Exclusive stage x engine attribution of a Lean-import perf profile.

Reads `perf script` output (frame-pointer or DWARF call graphs) and assigns
EVERY sample to exactly one cell: the importer stage is the INNERMOST importer
stage frame, the engine is the INNERMOST semantic kernel engine (independent
recheck, primary conversion, primary typing), falling back to the innermost
utility frame (term store, JSON, importer) and then the allocator only when no
more specific frame is on the stack. Cells sum to 100% of samples, so they can be added --
unlike inclusive percentages, where a sample inside `recheck` called from
`register_checked` counts toward both.

Usage: perf script -i perf.data | profile-lean-stages.py > attribution.json
       profile-lean-stages.py --self-test
"""
import json
import re
import sys
import unittest
from collections import Counter

# Innermost importer stage frame decides the stage (first match scanning leaf -> root).
STAGES = [
    ('intake', re.compile(r'interface::lean::(pipeline::|intake::|check_import_with_options|run_cli)')),
    ('declaration.inductive', re.compile(r'interface::lean::Importer::(inductive|ordinary_mutual_inductive|native_nested_recursors|quotient|structure_view)')),
    ('declaration.source_decode', re.compile(r'interface::lean::Importer::(shared_term|term_core|term)\b')),
    ('declaration.check', re.compile(r'interface::lean::Importer::record_inner|context::Context::declare')),
    ('record.parse', re.compile(r'serde_json::')),
]
# Innermost semantic engine decides the engine (first match scanning leaf -> root).
ENGINES = [
    ('independent_recheck', re.compile(r'logicaffeine_kernel::recheck')),
    ('primary_conversion', re.compile(r'logicaffeine_kernel::(type_checker::(def_eq|conversion|rigid_conversion|bounded_head|alpha_eq)|conversion_session|reduction::)')),
    ('primary_typing', re.compile(r'logicaffeine_kernel::(type_checker::(infer_type|check_type|infer_sort)|interface::lean::lean_type)')),
]
# Used only when no semantic engine is on the stack.
UTILITIES = [
    ('term_store', re.compile(r'logicaffeine_kernel::term_store::')),
    ('json_parse', re.compile(r'serde_json::')),
    ('importer', re.compile(r'logicaffeine_kernel::interface::lean::')),
]
# Used only when neither a semantic engine nor a utility frame is on the stack.
ALLOCATOR = [
    ('allocator', re.compile(r'(^|[^A-Za-z])(malloc|free|realloc|_int_malloc|_int_free|cfree|memmove|memcpy)')),
]


def samples(stream):
    """Yield call stacks, root first, from `perf script` text."""
    stack = []
    for line in stream:
        line = line.rstrip('\n')
        if not line.strip():
            if stack:
                yield list(reversed(stack))
                stack = []
            continue
        if line[:1] in (' ', '\t'):
            parts = line.strip().split(None, 1)
            if len(parts) == 2:
                symbol = parts[1].rsplit(' (', 1)[0]
                stack.append(symbol)
        elif stack:
            yield list(reversed(stack))
            stack = []
    if stack:
        yield list(reversed(stack))


def innermost(stack, table):
    for frame in reversed(stack):
        for name, pattern in table:
            if pattern.search(frame):
                return name
    return None


def classify(stack):
    stage = innermost(stack, STAGES) or 'other'
    engine = (innermost(stack, ENGINES) or innermost(stack, UTILITIES)
              or innermost(stack, ALLOCATOR) or 'other')
    return stage, engine


def attribute(stream):
    cells = Counter()
    total = 0
    for stack in samples(stream):
        cells[classify(stack)] += 1
        total += 1
    rows = [{'stage': s, 'engine': e, 'samples': n, 'percent': 100.0 * n / total}
            for (s, e), n in cells.most_common()] if total else []
    by = lambda key: {k: 100.0 * sum(r['samples'] for r in rows if r[key] == k) / total
                      for k in sorted({r[key] for r in rows})} if total else {}
    return {'schema': 'logos-lean-stage-attribution/1', 'samples': total,
            'exclusive': True, 'cells': rows, 'by_stage': by('stage'), 'by_engine': by('engine')}


class SelfTest(unittest.TestCase):
    PERF = '''lean_arena 1 1.0: 1 cycles:u:
\t1 _int_malloc (/lib/libc.so.6)
\t2 logicaffeine_kernel::recheck::whnf (/bin/x)
\t3 logicaffeine_kernel::interface::lean::dag::Sharing::register_checked (/bin/x)
\t4 logicaffeine_kernel::interface::lean::Importer::shared_term (/bin/x)
\t5 logicaffeine_kernel::interface::lean::Importer::record_inner (/bin/x)
\t6 main (/bin/x)

lean_arena 1 1.1: 1 cycles:u:
\t1 logicaffeine_kernel::type_checker::def_eq (/bin/x)
\t2 logicaffeine_kernel::interface::lean::Importer::record_inner (/bin/x)

lean_arena 1 1.2: 1 cycles:u:
\t1 serde_json::de::from_str (/bin/x)
\t2 logicaffeine_kernel::interface::lean::check_import_with_options (/bin/x)

'''

    def test_each_sample_lands_in_exactly_one_cell(self):
        result = attribute(self.PERF.splitlines(True))
        self.assertEqual(result['samples'], 3)
        self.assertAlmostEqual(sum(r['percent'] for r in result['cells']), 100.0)
        self.assertAlmostEqual(sum(result['by_stage'].values()), 100.0)
        self.assertAlmostEqual(sum(result['by_engine'].values()), 100.0)

    def test_innermost_stage_and_innermost_semantic_engine(self):
        cells = {(r['stage'], r['engine']) for r in attribute(self.PERF.splitlines(True))['cells']}
        # The helper check runs inside source decoding, and the allocator leaf
        # is charged to the independent kernel that called it.
        self.assertIn(('declaration.source_decode', 'independent_recheck'), cells)
        self.assertIn(('declaration.check', 'primary_conversion'), cells)
        self.assertIn(('record.parse', 'json_parse'), cells)

    def test_a_utility_frame_is_used_only_without_a_semantic_engine(self):
        stack = ['main', 'logicaffeine_kernel::interface::lean::Importer::term_core',
                 'logicaffeine_kernel::term_store::TermStore::intern', 'malloc']
        self.assertEqual(classify(stack), ('declaration.source_decode', 'term_store'))
        stack = ['main', 'logicaffeine_kernel::recheck::recheck',
                 'logicaffeine_kernel::term_store::TermStore::intern']
        self.assertEqual(classify(stack)[1], 'independent_recheck')

    def test_an_empty_profile_is_empty_not_a_division_error(self):
        self.assertEqual(attribute([])['samples'], 0)


if __name__ == '__main__':
    if sys.argv[1:] == ['--self-test']:
        unittest.main(argv=[sys.argv[0]], verbosity=2)
    else:
        json.dump(attribute(sys.stdin), sys.stdout, indent=2)
        sys.stdout.write('\n')
