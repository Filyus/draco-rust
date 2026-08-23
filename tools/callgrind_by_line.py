"""Self cost per source line of one file, optionally within one function.

    python3 callgrind_by_line.py cg_0.out cg_1.out src/corner_table.rs
    python3 callgrind_by_line.py cg_0.out cg_1.out vec/mod.rs reconstruct_mesh

The third step after `callgrind_by_file.py` and `callgrind_by_function.py`:
those say which file and which inlining parent, this says which line. The
function filter matters, because an `#[inline(always)]` helper accumulates its
line from every call site in the binary.

Two cautions the file view already carries apply here twice over. A line number
inside a standard-library file comes from an inline line table, so a count
implying an integer helper costs dozens of instructions is an artifact of that
table rather than a reading. And the file name is matched as a substring, so
`vec/mod.rs` also catches `raw_vec/mod.rs`.
"""
import collections
import re
import sys


def load(path, want_file, want_fn):
    total = collections.Counter()
    fl_names, fn_names = {}, {}
    cur_fl = cur_fn = '?'
    line_no = 0
    skip = False
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
            line_no = 0
            skip = False
        elif line.startswith('fn='):
            m = re.match(r'fn=\((\d+)\)\s*(.*)', line)
            if m:
                if m.group(2):
                    fn_names[m.group(1)] = m.group(2)
                cur_fn = fn_names.get(m.group(1), '?')
            else:
                cur_fn = line[3:]
            line_no = 0
            skip = False
        elif line.startswith('calls='):
            skip = True
        elif line.startswith('cfn='):
            m = re.match(r'cfn=\((\d+)\)\s*(.*)', line)
            if m and m.group(2):
                fn_names[m.group(1)] = m.group(2)
        elif line.startswith(('cfi=', 'cfl=', 'ob=', 'cob=', 'jump', 'jcnd')):
            continue
        elif line and (line[0].isdigit() or line[0] in '+-*'):
            parts = line.split()
            pos = parts[0]
            if pos == '*':
                pass
            elif pos[0] in '+-':
                line_no += int(pos)
            else:
                line_no = int(pos)
            if skip:
                skip = False
                continue
            if len(parts) < 2:
                continue
            if want_file in cur_fl and (want_fn is None or want_fn in cur_fn):
                try:
                    total[line_no] += int(parts[1])
                except ValueError:
                    pass
    return total


want_fn = sys.argv[4] if len(sys.argv) > 4 else None
a = load(sys.argv[1], sys.argv[3], want_fn)
b = load(sys.argv[2], sys.argv[3], want_fn)
d = b - a
for ln, cost in sorted(d.items(), key=lambda kv: -kv[1])[:25]:
    print(f'{cost:>12,}  {sys.argv[3]}:{ln}')
print(f'{sum(d.values()):>12,}  total')
