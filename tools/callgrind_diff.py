"""Per-function cost of one operation, as the difference between two runs.

The callgrind drivers (`decode_drc`, `encode_drc` and their C++ counterparts)
take an iteration count, so running each at `0` and at `1` and subtracting
leaves exactly one decode or encode: the file read, the parse and the dynamic
linker cancel out. That is exact rather than approximate, because callgrind
counts instructions.

    callgrind_annotate --threshold=99.9 cg_0.out > ann_0.txt
    callgrind_annotate --threshold=99.9 cg_1.out > ann_1.txt
    python3 callgrind_diff.py ann_0.txt ann_1.txt

Read the result by *stage*, not by symbol: the two sides draw function
boundaries in different places, and this project has three times been sent
after a target that turned out to be a naming artifact. See PERFORMANCE.md.
"""
import re
import sys


def load(path):
    totals = {}
    for line in open(path, encoding='utf-8', errors='replace'):
        m = re.match(r'\s*([\d,]+) \(\s*[\d.]+%\)\s+(.*)', line)
        if not m:
            continue
        count = int(m.group(1).replace(',', ''))
        name = m.group(2).strip()
        if name == 'PROGRAM TOTALS':
            continue
        totals[name] = totals.get(name, 0) + count
    return totals


base, head = load(sys.argv[1]), load(sys.argv[2])
rows = []
for name in set(base) | set(head):
    delta = head.get(name, 0) - base.get(name, 0)
    if abs(delta) > 20000:
        rows.append((delta, name))
rows.sort(reverse=True)
for delta, name in rows[:28]:
    short = name.split('[')[0].strip()
    if len(short) > 120:
        short = short[:117] + '...'
    print(f'{delta:>12,}  {short}')
