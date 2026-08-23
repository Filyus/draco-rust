"""Cost and count of calls to a named callee, by the function that made them.

    python3 callgrind_callsites.py cg_0.out cg_1.out memset

The answer `callgrind_callers.py` cannot give for a libc entry point. That one
reads the caller recorded against the callee, which for anything reached
through a PLT stub is the stub -- an address. This reads the other end: the
`fn=` context the `calls=` record sits under, which is the function that
actually made the call, with the call count beside the cost.

The count is what makes a `memset` or `memcpy` row readable, because those cost
one instruction per byte here -- `rep stosb`/`rep movsb`, which valgrind counts
per iteration. One call of `110,610` is a 110 KB fill, not a hot loop, and it
costs about a thirtieth of what the number says.
"""
import collections
import re
import sys


def load(path, callee):
    total = collections.Counter()
    fl_names, fn_names = {}, {}
    cur_fl = cur_fn = '?'
    cfl = cfn = None
    pending = None
    counts = collections.Counter()
    for line in open(path, errors='replace'):
        line = line.rstrip('\n')
        if line[:3] in ('fl=', 'fi=', 'fe='):
            m = re.match(r'..=\((\d+)\)\s*(.*)', line)
            if m:
                if m.group(2):
                    fl_names[m.group(1)] = m.group(2)
                cur_fl = fl_names.get(m.group(1), '?')
            else:
                cur_fl = line[3:]
        elif line.startswith('fn='):
            m = re.match(r'fn=\((\d+)\)\s*(.*)', line)
            if m:
                if m.group(2):
                    fn_names[m.group(1)] = m.group(2)
                cur_fn = fn_names.get(m.group(1), '?')
            else:
                cur_fn = line[3:]
        elif line.startswith(('cfi=', 'cfl=')):
            m = re.match(r'...=\((\d+)\)\s*(.*)', line)
            if m:
                if m.group(2):
                    fl_names[m.group(1)] = m.group(2)
                cfl = fl_names.get(m.group(1), '?')
            else:
                cfl = line[4:]
        elif line.startswith('cfn='):
            m = re.match(r'cfn=\((\d+)\)\s*(.*)', line)
            if m:
                if m.group(2):
                    fn_names[m.group(1)] = m.group(2)
                cfn = fn_names.get(m.group(1), '?')
            else:
                cfn = line[4:]
        elif line.startswith('calls='):
            pending = int(line[6:].split()[0])
        elif pending is not None:
            parts = line.split()
            n, pending = pending, None
            if len(parts) >= 2 and callee in (cfn or ''):
                try:
                    cost = int(parts[1])
                except ValueError:
                    continue
                key = f'{cur_fn}  -> {cfn}'
                total[key] += cost
                counts[key] += n
    return total, counts


a, ac = load(sys.argv[1], sys.argv[3])
b, bc = load(sys.argv[2], sys.argv[3])
dc = bc - ac
for key, cost in sorted((b - a).items(), key=lambda kv: -kv[1])[:30]:
    print(f'{cost:>12,}  x{dc[key]:<6}  {key}')
