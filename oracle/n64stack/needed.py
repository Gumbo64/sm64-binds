"""Procedures that can be on the stack when the modelled slots are read or written."""
import collections, json, sys
cg = json.load(open(sys.argv[1])); n = cg['nodes']
taken = set(cg['taken']) | set(json.load(open(sys.argv[2])))
def module(k):
    o = n[k]['o']
    return 'goddard' if o.startswith('src/goddard') else 'lib' if o.startswith(('lib/', 'src/audio')) else 'game'
callers = collections.defaultdict(set)
for k, v in n.items():
    for c in v['calls']:
        callers[c].add(k)
jalrs = [k for k, v in n.items() if v['jalr']]
# The model's three places: the z3 store, the box's read and set_camera_mode (whose sp
# nop_update_water_camera returns).
targets = ['src/engine/surface_collision.o:find_floor_from_list',
           'src/game/behavior_actions.o:bhv_jrb_sliding_box_loop', 'src/game/camera.o:set_camera_mode']
seen, todo = set(targets), list(targets)
while todo:
    k = todo.pop()
    # An indirect call may reach any address-taken procedure of its own module.
    for c in callers[k] | ({j for j in jalrs if module(j) == module(k)} if k in taken else set()):
        if c not in seen:
            seen.add(c); todo.append(c)
# Leave out the render walk: procedures only ever called directly from geo_process_root's
# walk (geo_process_root itself gets N64_STACK_OFF() by hand instead).
root = next(k for k, v in n.items() if v['name'] == 'geo_process_root')
walk, todo = {root}, [root]
while todo:
    for c in n[todo.pop()]['calls']:
        if c not in walk and c not in taken:
            walk.add(c); todo.append(c)
changed = True
while changed:
    changed = False
    for k in list(walk):
        if k != root and not callers[k] <= walk:
            walk.discard(k); changed = True
json.dump(sorted(k for k in seen - walk if n[k]['frame'] > 0), sys.stdout)
