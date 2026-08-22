"""Self cost per source file, as the difference between two callgrind runs.

The view that survives inlining. Function-level annotation attributes a callee
to whichever symbol swallowed it -- under full LTO a decoder can appear inside
`main` -- while the source file each instruction came from is recorded per
line and cannot be merged away. Use it to find *what kind* of work a run
spends its instructions on: `slice/index.rs` is bounds checking, `raw_vec` and
`vec/mod.rs` are growth, `num/uint_macros.rs` is integer helper code.

    python3 callgrind_by_file.py cg_0.out cg_1.out

Needs a build carrying debug info (`CARGO_PROFILE_RELEASE_DEBUG=2`); without
it every Rust frame reads `?`.
"""
import collections
import re
import sys


def load(path):
    total = collections.Counter()
    names = {}
    current = None
    skip_next_cost = False
    for line in open(path, errors='replace'):
        line = line.rstrip('\n')
        if line[:3] in ('fl=', 'fi=', 'fe='):
            m = re.match(r'..=\((\d+)\)\s*(.*)', line)
            if m:
                if m.group(2):
                    names[m.group(1)] = m.group(2)
                current = names.get(m.group(1), '?')
            else:
                current = line[3:]
            skip_next_cost = False
        elif line.startswith('calls='):
            skip_next_cost = True
        elif line.startswith(('fn=', 'cfn=', 'cfi=', 'cfl=', 'ob=', 'cob=')):
            continue
        elif line and (line[0].isdigit() or line[0] in '+-*') and current:
            parts = line.split()
            if len(parts) >= 2 and parts[1].isdigit():
                if skip_next_cost:
                    skip_next_cost = False
                else:
                    total[current] += int(parts[1])
    return total


base, head = load(sys.argv[1]), load(sys.argv[2])
delta = collections.Counter()
for key in set(base) | set(head):
    value = head[key] - base[key]
    if abs(value) > 10000:
        delta['/'.join(key.split('/')[-2:])] += value
for name, value in delta.most_common(24):
    print(f'{value:>10,}  {name}')
print(f'{sum(delta.values()):>10,}  total')
