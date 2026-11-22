#!/usr/bin/env python3
"""Generates rust/src/sm64/portable/tables.rs: how the game's state is laid out
in the headless WASM (build/us_web_headless) and in the rendering WASM
(build/us_web), for translating a save state from one build to the other.

    python3 maketools/gen_portable_tables.py [<sm64-port dir>]

Both builds are linked from the same objects, so their linker maps
(`sm64.map`, written with -Wl,-Map) list the same input sections with the same
sizes at different addresses. The tables record:

- RUNS: every input section of the game's data, as (headless address, visual
  address, size), so a pointer into constant data or a global translates by
  offset. Neighbouring sections that moved together are merged.
- VARS: the mutable variables that make up the game's state (everything in the
  game's objects that is not const), with their DWARF layout (which bytes are
  pointers and function pointers), for both builds.
- FNS: function table indices (what the game's function pointers are in WASM)
  of each function that both builds have, by build.
- the layouts of named structs, for the objects reached through pointers.

The headless build's code must not change (it is consensus-critical for
SM64Coin); this only reads the maps and WASMs of both builds.
"""

import bisect
import hashlib
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
_args = sys.argv[1:]
sys.argv = [sys.argv[0]]  # gen_mupen_tables reads argv at import
sys.path.insert(0, HERE)
import gen_mupen_tables as G  # noqa: E402
import tools  # noqa: E402

PORT = _args[0] if _args else os.path.join(HERE, "..", "sm64-port")
G.PORT = PORT
BUILD_H = os.path.join(PORT, "build", "us_web_headless")
BUILD_V = os.path.join(PORT, "build", "us_web")
OUT = os.path.join(HERE, "..", "rust", "src", "sm64", "portable", "tables.rs")

# Where DWARF types come from. levels/castle_inside holds the paintings, which
# are mutable structs in .data.
SOURCES = ["src/game", "src/engine", "src/menu", "src/audio", "src/goddard", "src/buffers"]
EXTRA_SOURCES = ["levels/castle_inside/leveldata.c", "levels/hmc/leveldata.c", "levels/ttm/leveldata.c"]

# Objects whose sections are part of the game's state. Everything else is
# build specific (libc, the allocator, SDL and GL, the display list
# interpreter, the headless render walk cache) or constant data.
STATE_DIRS = ("src/game/", "src/engine/", "src/menu/", "src/audio/", "src/goddard/", "src/buffers/")
# Never copied: per-frame render output that each build rebuilds, and big
# buffers nothing reads between frames.
NO_COPY_PREFIX = ("src/goddard/", "src/audio/")
NO_COPY_UNITS = {"src/buffers/framebuffers", "src/buffers/zbuffer", "src/buffers/gfx_output_buffer"}
NO_COPY_NAMES = {
    # the display list pool and what points into it, rebuilt by select_gfx_pool
    "gGfxPools", "gGfxPool", "gGfxSPTask", "gDisplayListHead", "gDisplayListHeadInChunk",
    "gDisplayListEndInChunk", "gGfxPoolEnd", "gGfxSPTaskYieldBuffer", "gGfxSPTaskStack",
    "gGfxSPTaskOutputBuffer", "gGfxSPTaskOutputBufferSize", "gFrameBuffers", "gFrameBuffer0",
    "gFrameBuffer1", "gFrameBuffer2", "gZBuffer", "gPhysicalFrameBuffers", "gPhysicalZBuffer",
    "gFBSetColor", "gFBFillColor", "gAudioHeap",
    # render walk and skybox state, recomputed by every frame that draws
    "gMatStackInterpolated", "sSkyBoxInfo", "gNumCalls",
}

# Mutable data (.data) that holds pointers is only state if it ever changes: the
# rest is tables the linker filled in (strings, display lists) that the build
# has its own copy of. Names come from maketools/portable_data_changed.txt,
# written by `PORTABLE_DUMPS=<dir with mem_0.bin, mem_*.bin> gen_portable_tables.py`:
# the variables that differ from mem_0.bin in any other dump.
CHANGED_FILE = os.path.join(HERE, "portable_data_changed.txt")
# Raw (untyped) data in levels/: the moving textures and paintings the game
# edits as it plays (found by diffing memory over a 120 star run).
RAW_PATTERNS = [re.compile(r"^\.data\..*(_water_data|_mist_data|_movtex_tris_\w+|_quicksand_large_mist|_quicksand_short_mist)$"),
                re.compile(r"^\.data\.\w+_painting$")]


def parse_map(path, tag):
    """(object, section) -> (addr, size), and (object, symbol) -> (addr, size,
    section) for the symbols listed under each section."""
    secs, syms, funcs = {}, {}, {}
    code_start = None
    sect = re.compile(r"^\s+([0-9a-f-]+)\s+([0-9a-f-]+)\s+([0-9a-f]+)\s+(\S+\.o|\S+\.a\([^)]*\)):\((\S+)\)$")
    sym = re.compile(r"^\s+([0-9a-f-]+)\s+([0-9a-f-]+)\s+([0-9a-f]+)\s+([A-Za-z_.][A-Za-z0-9_.$]*)$")
    cur = None
    for line in open(path):
        m = re.match(r"^\s+-\s+([0-9a-f]+)\s+[0-9a-f]+\s+CODE$", line)
        if m:
            code_start = int(m.group(1), 16)
            continue
        m = sect.match(line)
        if m:
            obj = m.group(4).replace("build/" + tag + "/", "")
            cur = (obj, m.group(5))
            if m.group(1) != "-":
                secs[cur] = (int(m.group(1), 16), int(m.group(3), 16))
            continue
        m = sym.match(line)
        if m and cur:
            size = int(m.group(3), 16)
            if m.group(1) == "-":
                # code: the second column is the file offset of the function
                funcs[(cur[0], m.group(4))] = int(m.group(2), 16)
            else:
                syms[(cur[0], m.group(4))] = (int(m.group(1), 16), size, cur[1])
    funcs["code_start"] = code_start
    return secs, syms, funcs


def leb(b, i):
    r = s = 0
    while True:
        x = b[i]
        i += 1
        r |= (x & 0x7F) << s
        s += 7
        if x < 0x80:
            return r, i


def wasm_functions(path):
    """(file offset of each defined function's body entry, table: list of
    function indices by table index, imported function count)."""
    b = open(path, "rb").read()
    i = 8
    secs = {}
    while i < len(b):
        sid = b[i]
        n, j = leb(b, i + 1)
        secs.setdefault(sid, []).append((j, n))
        if sid == 10:
            code_start = i
        i = j + n
    # imports
    j, n = secs[2][0]
    cnt, k = leb(b, j)
    imported = 0
    for _ in range(cnt):
        l, k = leb(b, k)
        k += l
        l, k = leb(b, k)
        k += l
        kind = b[k]
        k += 1
        if kind == 0:
            _, k = leb(b, k)
            imported += 1
        elif kind == 1:
            k += 1
            fl = b[k]
            k += 1
            _, k = leb(b, k)
            if fl & 1:
                _, k = leb(b, k)
        elif kind == 2:
            fl = b[k]
            k += 1
            _, k = leb(b, k)
            if fl & 1:
                _, k = leb(b, k)
        elif kind == 3:
            k += 2
    # code
    j, n = secs[10][0]
    cnt, k = leb(b, j)
    offsets = []
    for _ in range(cnt):
        offsets.append(k)
        sz, k2 = leb(b, k)
        k = k2 + sz
    # elements
    j, n = secs[9][0]
    cnt, k = leb(b, j)
    table = {}
    for _ in range(cnt):
        flag, k = leb(b, k)
        assert flag == 0, flag
        assert b[k] == 0x41
        off, k = leb_signed(b, k + 1)
        assert b[k] == 0x0B
        k += 1
        num, k = leb(b, k)
        for t in range(num):
            f, k = leb(b, k)
            table[off + t] = f
    return offsets, table, imported, code_start


def leb_signed(b, i):
    r = s = 0
    while True:
        x = b[i]
        i += 1
        r |= (x & 0x7F) << s
        s += 7
        if x < 0x80:
            if x & 0x40:
                r -= 1 << s
            return r, i


_offset_set = {}


def set_offsets(offsets):
    if id(offsets) not in _offset_set:
        _offset_set[id(offsets)] = set(offsets)
    return _offset_set[id(offsets)]


def function_table(build_dir, funcs):
    """{table index: (object, function name)} for the build."""
    offsets, table, imported, code_start = wasm_functions(os.path.join(build_dir, "sm64.us.wasm"))
    # the map's offsets are from before the link's last rewrite of the module
    raw = {key: off for key, off in funcs.items() if key != "code_start"}
    lowest = min(raw.values())
    delta = max((offsets[j] - lowest for j in range(8)),
                key=lambda d: sum(1 for off in raw.values() if off + d in set_offsets(offsets)))
    by_off = {}
    for key, off in raw.items():
        by_off.setdefault(off + delta, key)
    names = {}
    for idx, f in table.items():
        d = f - imported
        if 0 <= d < len(offsets) and offsets[d] in by_off:
            names[idx] = by_off[offsets[d]]
    return names, table


def has_pointers(L, li, seen=None):
    for off, kind, count, stride in L.layouts[li]:
        if isinstance(kind, tuple):
            if kind[0] in ("p",):
                return True
            if kind[0] == "L" and has_pointers(L, kind[1]):
                return True
        elif kind in ("fn", "w"):
            return True
    return False


def changed_data(syh):
    """Names ((object, symbol)) of data that differ between mem_0.bin and the
    other dumps in $PORTABLE_DUMPS; saved to CHANGED_FILE."""
    d = os.environ.get("PORTABLE_DUMPS")
    if not d:
        if os.path.exists(CHANGED_FILE):
            return {tuple(l.rstrip("\n").split(" ", 1)) for l in open(CHANGED_FILE) if l.strip()}
        return None
    base = open(os.path.join(d, "mem_0.bin"), "rb").read()
    dumps = [open(os.path.join(d, f), "rb").read() for f in sorted(os.listdir(d)) if f.startswith("mem_") and f != "mem_0.bin"]
    out = set()
    for (obj, sym), (a, n, sec) in syh.items():
        if a >= 0x7B4620 and n and any(m[a:a + n] != base[a:a + n] for m in dumps):
            out.add((obj, sym))
    with open(CHANGED_FILE, "w") as f:
        for obj, sym in sorted(out):
            f.write(f"{obj} {sym}\n")
    return out


def main():
    sh, syh, fh = parse_map(os.path.join(BUILD_H, "sm64.map"), "us_web_headless")
    sv, syv, fv = parse_map(os.path.join(BUILD_V, "sm64.map"), "us_web")
    id_h = hashlib.sha1(tools.strip_wasm_data(open(os.path.join(BUILD_H, "sm64.us.wasm"), "rb").read())).hexdigest()
    id_v = hashlib.sha1(tools.strip_wasm_data(open(os.path.join(BUILD_V, "sm64.us.wasm"), "rb").read())).hexdigest()

    # --- DWARF -------------------------------------------------------------
    srcs = []
    for d in SOURCES:
        for f in sorted(os.listdir(os.path.join(PORT, d))):
            obj = os.path.join(BUILD_H, d, f[:-2] + ".o")
            if f.endswith(".c") and os.path.exists(obj):
                srcs.append(f"{d}/{f}")
    srcs += EXTRA_SOURCES
    L = G.Layouts()
    variables = {}  # (object, symbol) -> (size, layout, const)
    named = {}
    with tempfile.TemporaryDirectory() as tmp, ThreadPoolExecutor(int(os.environ.get("JOBS", "4"))) as ex:
        for src, text in ex.map(lambda s: G.compile_unit(s, tmp), srcs):
            unit = src[:-2]
            dies = G.parse_dwarf(text)
            cu = next((d for d in dies.values() if d["tag"] == "DW_TAG_compile_unit"), None)
            if cu is None:
                continue
            for d in cu["children"]:
                if d["tag"] == "DW_TAG_structure_type" and "DW_AT_name" in d["attrs"] and "DW_AT_byte_size" in d["attrs"]:
                    sname = d["attrs"]["DW_AT_name"].strip('"')
                    if sname not in named:
                        items = []
                        G.leaves(dies, L, d["off"], 0, items)
                        named[sname] = (G.num(d["attrs"]["DW_AT_byte_size"]), L.add(G.merge(items)))

            def add(v, symname):
                a = v["attrs"]
                if "DW_AT_location" not in a or "DW_AT_type" not in a:
                    return
                t = G.ref(a["DW_AT_type"])
                cst = dies[t]["tag"] == "DW_TAG_const_type"
                try:
                    items = []
                    G.leaves(dies, L, t, 0, items)
                    size = G.size_of(dies, t)
                except (ValueError, KeyError):
                    return
                variables[(unit + ".o", symname)] = (size, L.add(G.merge(items)), cst)

            for v in cu["children"]:
                if v["tag"] == "DW_TAG_variable" and "DW_AT_name" in v["attrs"]:
                    add(v, v["attrs"]["DW_AT_name"].strip('"'))
                elif v["tag"] == "DW_TAG_subprogram" and "DW_AT_name" in v["attrs"]:
                    fn = v["attrs"]["DW_AT_name"].strip('"')

                    def walk(node):
                        for c in node["children"]:
                            if c["tag"] == "DW_TAG_variable" and "DW_AT_name" in c["attrs"]:
                                add(c, fn + "." + c["attrs"]["DW_AT_name"].strip('"'))
                            elif c["tag"] == "DW_TAG_lexical_block":
                                walk(c)

                    walk(v)

    # --- runs of data that moved together -------------------------------------
    DATA_START = 0x10000
    runs = []
    for key, (a, s) in sh.items():
        if a < DATA_START or s == 0 or key[0].endswith(")") and False:
            continue
        if key in sv and sv[key][1] == s:
            # skip code/section-less entries
            runs.append((a, sv[key][0], s))
    runs.sort()
    merged = []
    for a, v, s in runs:
        if merged and merged[-1][0] + merged[-1][2] == a and merged[-1][1] + merged[-1][2] == v:
            merged[-1] = (merged[-1][0], merged[-1][1], merged[-1][2] + s)
        else:
            merged.append((a, v, s))

    # --- variables ---------------------------------------------------------------
    rodata_end_h = max(a + s for (o, sec), (a, s) in sh.items() if sec == ".rodata" or False) if False else None
    changed = changed_data(syh)
    bss_start = None
    for line in open(os.path.join(BUILD_H, "sm64.map")):
        m = re.match(r"^\s+([0-9a-f]+)\s+[0-9a-f]+\s+[0-9a-f]+\s+\.bss$", line)
        if m:
            bss_start = int(m.group(1), 16)
    vars_out = []
    stats = {"typed": 0, "raw": 0, "const": 0, "skipped": 0}
    for (obj, symname), (ah, sz, secname) in sorted(syh.items(), key=lambda kv: kv[1][0]):
        if ah < 0x7B4620 or sz == 0:  # data and bss only (.rodata is below)
            continue
        if symname.startswith((".", "__const.")):
            continue
        if not obj.endswith(".o") or not obj.startswith(("src/", "levels/")) or "/dynlists/" in obj:
            continue
        unit = obj[:-2]
        if obj.startswith(NO_COPY_PREFIX):
            continue
        if (obj, symname) not in syv or syv[(obj, symname)][1] != sz:
            continue
        av = syv[(obj, symname)][0]
        typed = variables.get((obj, symname))
        in_state = obj.startswith(STATE_DIRS) and unit not in NO_COPY_UNITS and symname not in NO_COPY_NAMES
        if typed is not None and in_state:
            size, li, cst = typed
            if cst:
                stats["const"] += 1
                continue
            if size != sz:
                li, size = None, sz
            if li is not None and ah < bss_start and changed is not None and has_pointers(L, li) \
                    and (obj, symname) not in changed:
                stats["skipped"] += 1
                continue
            vars_out.append((obj, symname, ah, av, sz, li))
            stats["typed" if li is not None else "raw"] += 1
        elif in_state:
            # no DWARF match (function statics numbered `.N`, compiler
            # generated data): raw, with no pointers
            vars_out.append((obj, symname, ah, av, sz, None))
            stats["raw"] += 1
        elif obj.startswith("levels/") and any(p.match(secname) for p in RAW_PATTERNS):
            li = typed[1] if typed is not None and typed[0] == sz and not typed[2] else None
            vars_out.append((obj, symname, ah, av, sz, li))
            stats["typed" if li is not None else "raw"] += 1
        else:
            stats["skipped"] += 1

    # --- functions ----------------------------------------------------------------
    names_h, _ = function_table(BUILD_H, fh)
    names_v, _ = function_table(BUILD_V, fv)
    by_name_v = {}
    for idx, key in names_v.items():
        by_name_v.setdefault(key, []).append(idx)
    fns = []
    for idx, key in sorted(names_h.items()):
        if key in by_name_v and len(by_name_v[key]) == 1:
            fns.append((idx, by_name_v[key][0], key[1]))
    n_h = len(set(names_h.values()))

    # --- graph node types ------------------------------------------------------------
    node_types = []
    text = open(os.path.join(PORT, "src/engine/graph_node.h")).read()
    for m in re.finditer(r"#define GRAPH_NODE_TYPE_(\w+)\s+\(?(0x[0-9A-Fa-f]+)(?:\s*\|\s*GRAPH_NODE_TYPE_FUNCTIONAL)?\)?", text):
        name, val = m.group(1), int(m.group(2), 16)
        if "GRAPH_NODE_TYPE_FUNCTIONAL" in m.group(0):
            val |= 0x100
        if name in ("FUNCTIONAL", "400"):
            continue
        camel = "GraphNode" + "".join(w.capitalize() for w in name.split("_"))
        camel = {"GraphNodeHeldObj": "GraphNodeHeldObject", "GraphNodeGeneratedList": "GraphNodeGenerated",
                 "GraphNodeObject": "GraphNodeObject"}.get(camel, camel)
        node_types.append((val, camel))
    missing = [n for _, n in node_types if n not in named]
    if missing:
        print("warning: no struct for", missing, file=sys.stderr)

    # --- object slots ------------------------------------------------------------------
    pointer_mask = 0
    for kind in ("ANIMS", "WAYPOINT", "OBJ", "SURFACE", "VPTR", "CVPTR", "S16P", "S32P", "CHAIN_SEGMENT"):
        for slot in G.object_fields(kind).values():
            pointer_mask |= 1 << slot

    gm_h = syh[next(k for k in syh if k[0].endswith("dlmalloc.o)") and k[1] == "_gm_")][0]
    gm_v = syv[next(k for k in syv if k[0].endswith("dlmalloc.o)") and k[1] == "_gm_")][0]

    data_starts = []
    for b in (BUILD_H, BUILD_V):
        for line in open(os.path.join(b, "sm64.map")):
            m = re.match(r"^\s+([0-9a-f]+)\s+[0-9a-f]+\s+[0-9a-f]+\s+\.data$", line)
            if m:
                data_starts.append(int(m.group(1), 16))
    data_start = data_starts[0]
    tex = syv[("src/pc/gfx/gfx_pc.o", "gfx_texture_cache")]
    geo_gen = syh[("src/game/rendering_graph_node.o", "sGeoCacheGeneration")][0]
    geo_count = syh[("src/game/rendering_graph_node.o", "sGeoCacheCount")][0]

    # --- write ---------------------------------------------------------------------------
    out = []
    w = out.append
    w("// Generated by maketools/gen_portable_tables.py from the linker maps and WASMs of the headless")
    w("// and the rendering builds and the DWARF of the sources. Do not edit.")
    w("#![allow(dead_code)]")
    w("")
    w("use super::layout::{Item, K};")
    w("")
    w(f'pub const BUILD_ID_HEADLESS: &str = "{id_h}";')
    w(f'pub const BUILD_ID_VISUAL: &str = "{id_v}";')
    w(f"/// dlmalloc's `_gm_` (the malloc state), headless then visual.")
    w(f"pub const GM: [u32; 2] = [{gm_h:#x}, {gm_v:#x}];")
    w("")
    w("/// Where .data starts in the headless build: below it is constant data.")
    w(f"pub const DATA_START: u32 = {data_start:#x};")
    w("/// Where .data starts (the end of constant data) in the headless and the visual build.")
    w(f"pub const DATA_STARTS: [u32; 2] = [{data_starts[0]:#x}, {data_starts[1]:#x}];")
    w("/// The visual build's texture cache (address, size): it maps texture addresses to GL textures.")
    w(f"pub const TEXTURE_CACHE: (u32, u32) = ({tex[0]:#x}, {tex[1]});")
    w("/// The headless build's render walk cache (its generation and count), which a loaded state must invalidate.")
    w(f"pub const GEO_CACHE: [u32; 2] = [{geo_gen:#x}, {geo_count:#x}];")
    w("pub static POINTEES: &[&str] = &[" + ", ".join(f'"{p}"' for p in L.pointees) + "];")
    w("")
    w("pub static LAYOUTS: &[&[Item]] = &[")
    for items in L.layouts:
        w("    &[" + ", ".join(f"Item({o}, {G.rust_kind(k)}, {c}, {s})" for (o, k, c, s) in items) + "],")
    w("];")
    w("")
    w("/// The game's mutable variables: (name, headless address, visual address, size, layout or 0xFFFF for raw bytes).")
    w("pub static VARS: &[(&str, u32, u32, u32, u16)] = &[")
    for obj, name, ah, av, sz, li in vars_out:
        w(f'    ("{obj[:-2]}:{name}", {ah:#x}, {av:#x}, {sz}, {0xFFFF if li is None else li}),')
    w("];")
    w("")
    w("/// Sections that moved together: (headless address, visual address, size), sorted by headless address.")
    w("pub static RUNS: &[(u32, u32, u32)] = &[")
    for a, v, s in merged:
        w(f"    ({a:#x}, {v:#x}, {s:#x}),")
    w("];")
    w("")
    w("/// Function table indices of functions both builds have: (headless, visual, name).")
    w("pub static FNS: &[(u16, u16, &str)] = &[")
    for h, v, n in fns:
        w(f'    ({h}, {v}, "{n}"),')
    w("];")
    w("")
    w("/// Named structs: (name, size, layout).")
    w("pub static NAMED: &[(&str, u32, u16)] = &[")
    for sname, (size, li) in sorted(named.items()):
        w(f'    ("{sname}", {size}, {li}),')
    w("];")
    w("")
    w("/// GraphNode.type -> the struct that node is.")
    w("pub static NODE_TYPES: &[(u16, &str)] = &[")
    for val, n in node_types:
        w(f'    ({val:#x}, "{n}"),')
    w("];")
    w("")
    w("/// Object field slots some behavior keeps a pointer in (by their declared types).")
    w(f"pub const POINTER_SLOTS: u128 = {pointer_mask:#x};")
    w("")
    open(OUT, "w").write("\n".join(out) + "\n")
    print(f"{stats} ; {len(merged)} runs, {len(fns)}/{n_h} functions, {len(L.layouts)} layouts, "
          f"headless {id_h[:8]} visual {id_v[:8]} -> {OUT}")
    tot = sum(v[4] for v in vars_out)
    print(f"copied bytes: {tot}")
    import collections
    byu = collections.Counter()
    rawu = collections.Counter()
    for obj, name, ah, av, sz, li in vars_out:
        byu[obj] += sz
        if li is None:
            rawu[obj] += sz
    print("raw bytes by unit:", rawu.most_common(25))
    print("biggest:", sorted(((sz, o, n) for o, n, ah, av, sz, li in vars_out), reverse=True)[:25])


if __name__ == "__main__":
    main()
