"""Self cost of one source file, split by the function it was inlined into.

The follow-up to `callgrind_by_file.py`: once that says `slice/index.rs` is a
million instructions, this says which functions they are in.

    python3 callgrind_by_function.py cg_0.out cg_1.out slice/index.rs
"""
import collections
import re
import sys


def load(path, want):
    total = collections.Counter()
    fnames, flnames = {}, {}
    cur_fl = cur_fn = None
    skip_next_cost = False
    for line in open(path, errors='replace'):
        line = line.rstrip('\n')
        if line[:3] in ('fl=', 'fi=', 'fe='):
            m = re.match(r'..=\((\d+)\)\s*(.*)', line)
            if m:
                if m.group(2):
                    flnames[m.group(1)] = m.group(2)
                cur_fl = flnames.get(m.group(1), '?')
            else:
                cur_fl = line[3:]
            skip_next_cost = False
        elif line.startswith('fn='):
            m = re.match(r'fn=\((\d+)\)\s*(.*)', line)
            if m:
                if m.group(2):
                    fnames[m.group(1)] = m.group(2)
                cur_fn = fnames.get(m.group(1), '?')
            else:
                cur_fn = line[3:]
            skip_next_cost = False
        elif line.startswith('calls='):
            skip_next_cost = True
        elif line.startswith('cfn='):
            # `cfn=` shares the name namespace with `fn=` and is often where an
            # id is first given its name; dropping these leaves later `fn=(id)`
            # references nameless, which reads as one huge anonymous bucket.
            m = re.match(r'cfn=\((\d+)\)\s*(.*)', line)
            if m and m.group(2):
                fnames[m.group(1)] = m.group(2)
        elif line.startswith(('cfi=', 'cfl=', 'ob=', 'cob=')):
            continue
        elif line and (line[0].isdigit() or line[0] in '+-*') and cur_fl and cur_fn:
            parts = line.split()
            if len(parts) >= 2 and parts[1].isdigit():
                if skip_next_cost:
                    skip_next_cost = False
                elif want in cur_fl:
                    total[cur_fn] += int(parts[1])
    return total


base, head = load(sys.argv[1], sys.argv[3]), load(sys.argv[2], sys.argv[3])
delta = collections.Counter()
for key in set(base) | set(head):
    value = head[key] - base[key]
    if abs(value) > 5000:
        delta[key] = value
for name, value in delta.most_common(12):
    short = name.split('::')[-1][:70]
    print(f'{value:>10,}  {short}')
print(f'{sum(delta.values()):>10,}  total in {sys.argv[3]}')
