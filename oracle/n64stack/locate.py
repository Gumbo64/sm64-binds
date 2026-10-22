"""Find each needed procedure's definition in sm64-port (dry run report)."""
import json, re, sys, pathlib
port = pathlib.Path(sys.argv[1]); needed = json.load(open(sys.argv[2])); cg = json.load(open(sys.argv[3]))['nodes']
def tu_files(c):
    files, todo = [], [c]
    while todo:
        f = todo.pop()
        if not f.exists() or f in files: continue
        files.append(f)
        for m in re.finditer(r'#include\s+"([^"]+\.c)"', f.read_text(errors='replace')):
            for base in (f.parent, port / 'src', port):
                if (base / m.group(1)).exists():
                    todo.append(base / m.group(1)); break
    return files
def defs(path, name):
    lines = path.read_text(errors='replace').split('\n')
    out = []
    for i, l in enumerate(lines):
        if re.match(r'^[A-Za-z_].*\b' + re.escape(name) + r'\s*\(', l) and not l.rstrip().endswith(';') and not l.startswith(('#', 'return')):
            # find the opening brace of the body
            for j in range(i, min(i + 12, len(lines))):
                s = lines[j].rstrip('\r')
                if s.endswith(';') and j != i: break
                if s.endswith('{'):
                    out.append(j); break
                if s.endswith(')') and j + 1 < len(lines) and lines[j + 1].rstrip('\r') == '{':
                    out.append(j + 1); break
    return out
ALIASES = {'bhv_spawned_coin_loop': 'bhv_coin_loop', 'bhv_treasure_chest_ddd_init': 'bhv_treasure_chest_init',
           'bhv_treasure_chest_ddd_loop': 'bhv_treasure_chest_loop'}  # renamed since sm64-port forked
found, missing = [], []
for k in needed:
    node = cg[k]; c = port / node['o'].replace('.o', '.c')
    hits = [(str(f.relative_to(port)), j) for f in tu_files(c) for j in defs(f, ALIASES.get(node['name'], node['name']))]
    (found if hits else missing).append((k, node['frame'], hits))
print(len(found), 'found', len(missing), 'missing', sum(1 for f in found if len(f[2]) > 1), 'multi')
for m in missing: print('MISSING', m[0], m[1])
for f in found:
    if len(f[2]) > 1: print('MULTI', f[0], f[2])
json.dump(found, open(sys.argv[4], 'w'))
