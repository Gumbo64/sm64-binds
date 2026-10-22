"""Procedures whose address is stored in data (any IDO object), exact for static ones."""
import json, struct, subprocess, sys, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from mdebug import section
root = pathlib.Path(sys.argv[1]); procs = json.load(open(sys.argv[2]))
glob = {p['name']: f'{o}:{p["name"]}' for o, ps in procs.items() for p in ps if not p['static']}
taken = set()
for ofile in sorted(root.rglob('*.o')):
    o = str(ofile.relative_to(root))
    rel = subprocess.run(['mips-linux-gnu-readelf', '-rW', str(ofile)], capture_output=True, text=True).stdout
    sec = None
    for l in rel.splitlines():
        if l.startswith('Relocation section'):
            sec = l.split("'")[1]; continue
        p = l.split()
        if not sec or sec == '.rel.text' or len(p) < 5 or p[2] != 'R_MIPS_32':
            continue
        name = p[4]
        if name in glob:
            taken.add(glob[name])
        elif name == '.text':
            d, off = section(ofile, sec[4:])
            word, = struct.unpack_from('>I', d, off + int(p[0], 16))
            for q in procs.get(o, []):
                if q['adr'] == word:
                    taken.add(f'{o}:{q["name"]}')
json.dump(sorted(taken), sys.stdout)
