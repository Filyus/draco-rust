"""Which functions call a given callee, and what those calls cost.

    python3 callgrind_callers.py cg_0.out cg_1.out memset

Caveat worth knowing before spending a run on it: a call that goes through a
PLT stub is attributed to the stub, so for libc entry points such as `memset`
this answers with an address rather than a caller. It is useful for callees
inside the binary being profiled.
"""
import collections
import re
import sys


def load(path, callee):
    total = collections.Counter()
    names = {}
    cur_fn = None
    pending = None
    for line in open(path, errors='replace'):
        line = line.rstrip('\n')
        if line.startswith(('fn=', 'cfn=')):
            kind, rest = line.split('=', 1)
            m = re.match(r'\((\d+)\)\s*(.*)', rest)
            if m:
                if m.group(2):
                    names[m.group(1)] = m.group(2)
                name = names.get(m.group(1), '?')
            else:
                name = rest
            if kind == 'fn':
                cur_fn = name
                pending = None
            else:
                pending = name
        elif line.startswith('calls='):
            continue
        elif line and line[0].isdigit() and pending is not None:
            if callee in pending and cur_fn:
                parts = line.split()
                if len(parts) >= 2 and parts[1].isdigit():
                    total[cur_fn] += int(parts[1])
            pending = None
    return total


base, head = load(sys.argv[1], sys.argv[3]), load(sys.argv[2], sys.argv[3])
delta = collections.Counter()
for key in set(base) | set(head):
    value = head[key] - base[key]
    if abs(value) > 5000:
        delta[key] = value
for name, value in delta.most_common(12):
    print(f'{value:>10,}  {name.split("::")[-1][:70]}')
print(f'{sum(delta.values()):>10,}  total into {sys.argv[3]}')
