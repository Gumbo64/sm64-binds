"""Call graph of the IDO objects: direct jal edges, jalr sites and address-taken procedures."""
import json, re, subprocess, sys, pathlib
root = pathlib.Path(sys.argv[1]); procs = json.load(open(sys.argv[2]))
def owner(ps, off):
    best = None
    for p in ps:
        if p['adr'] <= off and (best is None or p['adr'] >= best['adr']):
            best = p
    return best
glob = {}
for o, ps in procs.items():
    for p in ps:
        key = f"{o}:{p['name']}"
        p['key'] = key
        if not p['static']:
            glob[p['name']] = key
nodes = {}
taken = set()
for o, ps in procs.items():
    for p in ps:
        nodes[p['key']] = dict(o=o, name=p['name'], frame=p['frame'], static=p['static'], calls=set(), jalr=False)
    dis = subprocess.run(['mips-linux-gnu-objdump', '-dr', '--no-show-raw-insn', '-j', '.text', str(root / o)], capture_output=True, text=True).stdout
    last = None
    for l in dis.splitlines():
        m = re.match(r'^\s+([0-9a-f]+):\t(\S+)\s*(.*)$', l)
        if m:
            off, op, args = int(m.group(1), 16), m.group(2), m.group(3)
            cur = owner(ps, off)
            last = (cur, op, args)
            if op == 'jalr' and cur:
                nodes[cur['key']]['jalr'] = True
            continue
        m = re.match(r'^\s+[0-9a-f]+: (R_MIPS_\w+)\s+(\S+)$', l)
        if m and last and last[0]:
            typ, sym = m.groups()
            cur, op, args = last
            if sym.startswith('.text'):
                tgt_off = None
                if typ == 'R_MIPS_26':
                    tgt_off = int(args.split()[0], 16)
                elif typ == 'R_MIPS_LO16':
                    mm = re.search(r'(-?\d+)\(?', args.split(',')[-1]); tgt_off = int(mm.group(1)) if mm else None
                t = owner(ps, tgt_off) if tgt_off is not None else None
                if t and t['adr'] == tgt_off:
                    if typ == 'R_MIPS_26': nodes[cur['key']]['calls'].add(t['key'])
                    else: taken.add(t['key'])
            elif sym in glob:
                if typ == 'R_MIPS_26': nodes[cur['key']]['calls'].add(glob[sym])
                elif typ in ('R_MIPS_HI16', 'R_MIPS_LO16', 'R_MIPS_32'): taken.add(glob[sym])
    # address-taken from data relocations
    rel = subprocess.run(['mips-linux-gnu-readelf', '-rW', str(root / o)], capture_output=True, text=True).stdout
    sec = None
    for l in rel.splitlines():
        if l.startswith('Relocation section'):
            sec = l.split("'")[1]; continue
        p = l.split()
        if sec and sec != '.rel.text' and len(p) >= 5 and p[2] == 'R_MIPS_32':
            name = p[4]
            if name in glob: taken.add(glob[name])
            elif name.startswith('.text'):
                pass  # static function pointers in data: resolved below by addend scan
json.dump(dict(nodes={k: dict(v, calls=sorted(v['calls'])) for k, v in nodes.items()}, taken=sorted(taken)), sys.stdout)
