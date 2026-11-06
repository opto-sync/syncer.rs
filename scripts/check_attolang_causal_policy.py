#!/usr/bin/env python3
"""Finite causal-order proof slice; Kani is authoritative for full u64 domain.

This enumerates pairs of 2-replica clocks, includes negative cases, and checks
an independent AttoLang source reference without executing its compiler.
"""
from itertools import product
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def before(a: tuple[int, int], b: tuple[int, int]) -> bool:
    return a[0] <= b[0] and a[1] <= b[1] and a != b


def relation(a: tuple[int, int], b: tuple[int, int]) -> str:
    if a == b:
        return 'equal'
    if before(a, b):
        return 'before'
    if before(b, a):
        return 'after'
    return 'concurrent'


def main() -> int:
    clocks = list(product(range(4), repeat=2))
    counts = {'equal': 0, 'before': 0, 'after': 0, 'concurrent': 0}
    for a, b in product(clocks, repeat=2):
        r = relation(a, b)
        counts[r] += 1
        inv = {'equal': 'equal', 'before': 'after',
               'after': 'before', 'concurrent': 'concurrent'}
        assert relation(b, a) == inv[r]
        assert before(a, b) == (r == 'before')
        assert sum([a == b, before(a, b), before(b, a), r == 'concurrent']) == 1
        join = tuple(max(x, y) for x, y in zip(a, b))
        assert all(join[i] >= a[i] and join[i] >= b[i] for i in (0, 1))
        assert tuple(max(x, x) for x in a) == a, 'join idempotence'
        assert join == tuple(max(x, y) for x, y in zip(b, a)), 'join commutativity'
    assert sum(counts.values()) == 256
    assert all(count > 0 for count in counts.values()), counts
    assert relation((0, 2), (2, 0)) == 'concurrent'
    assert relation((0, 1), (1, 1)) == 'before'
    assert relation((1, 1), (1, 1)) == 'equal'
    source = (ROOT / 'src/causal_policy.atto').read_text()
    for anchor in ['define module causal_policy', 'clock_before(',
                   'clock_equal(', 'clock_concurrent(', 'self_check()',
                   'a0 <= b0 && a1 <= b1', '!clock_before(b0,b1,a0,a1)']:
        assert anchor in source, anchor
    print(f'PASS finite causal ordering: {counts}; full u64 proof via Kani pending; '
          'AttoLang compilation NOT_EXECUTED')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
