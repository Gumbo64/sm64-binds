"""Insert N64_FRAME(size); as the first line of every located function body."""
import json, collections, sys, pathlib
port = pathlib.Path(sys.argv[1]); located = json.load(open(sys.argv[2]))
by = collections.defaultdict(list)
for key, frame, hits in located:
    for f, line in hits:
        by[f].append((line, frame))
n = 0
for f, ins in by.items():
    path = port / f
    raw = path.read_bytes().decode()
    nl = '\r\n' if '\r\n' in raw else '\n'
    lines = raw.split(nl)
    for line, frame in sorted(ins, reverse=True):
        assert 'N64_FRAME' not in lines[line + 1], (f, line)
        lines.insert(line + 1, f'    N64_FRAME({frame});')
        n += 1
    path.write_bytes(nl.join(lines).encode())
print(n, 'lines in', len(by), 'files')
