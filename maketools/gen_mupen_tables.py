#!/usr/bin/env python3
"""Generates rust/src/sm64/mupen/tables.rs: where the game's variables are on
the N64 (US ROM) and in the headless WASM, and how each is laid out.

    python3 maketools/gen_mupen_tables.py <N64 US decomp linker map> [<sm64-port dir>]

The N64 addresses come from a linker map of the US decomp build (for example
STROOP's `Mappings/MappingUS.map`). The WASM addresses come from the headless
build's map (`build/us_web_headless/sm64.map`), which only matches the WASM it
was linked with, so the output records that WASM's build id (the SHA-1 of its
code) and the loader refuses other builds.

The layouts come from the port's own sources: each game and engine file is
compiled again with emcc and `-g` (same defines as the headless build, wasm32
like the game), and the DWARF of its variables is read with llvm-dwarfdump.
N64 struct layouts are the same as wasm32 ones (both have 4-byte pointers and
natural alignment), only the byte order differs, so a layout says which bytes
to swap as 2, 4 or 8-byte values and which words are pointers.
"""

import hashlib
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import tools  # noqa: E402  (strip_wasm_data, the same as the Rust build id)

N64_MAP = sys.argv[1] if len(sys.argv) > 1 else None
PORT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(HERE, "..", "sm64-port")
BUILD = os.path.join(PORT, "build", "us_web_headless")
OUT = os.path.join(HERE, "..", "rust", "src", "sm64", "mupen", "tables.rs")
EMSDK = os.environ.get("EMSDK", "/usr/local/emsdk")
DWARFDUMP = os.path.join(EMSDK, "upstream", "bin", "llvm-dwarfdump")

CFLAGS = (
    "-c -g -O0 -D_LANGUAGE_C -Iinclude -Ibuild/us_web_headless -Ibuild/us_web_headless/include -Isrc -I. "
    "-DHEADLESS_VERSION=1 -DVERSION_US=1 -DF3DEX_GBI_2E=1 -DNON_MATCHING=1 -DAVOID_UB=1 -DTARGET_WEB "
    "-fPIC -DNO_SEGMENTED_MEMORY -DUSE_SYSTEM_MALLOC -DWIDESCREEN -fno-strict-aliasing -fwrapv -w"
).split()

SOURCES = ["src/game", "src/engine", "src/menu", "src/buffers"]

# Symbols only one side has, that the loader needs by name.
N64_ONLY = ["sSurfacePool", "sSurfaceNodePool", "sSurfacePoolSize", "sSegmentTable"]
PORT_ONLY = [("surface_load", "sStaticSurfacePool"), ("surface_load", "sStaticSurfaceNodePool"),
             ("surface_load", "sDynamicSurfacePool"), ("surface_load", "sDynamicSurfaceNodePool")]

# Copied although their units are not.
COPY_EXTRA = {"gObjParentGraphNode", "gAreaUpdateCounter"}

# Paths: behaviors whose code uses these waypoint fields (see s16_slots).
WAYPOINT_FIELDS = ["oPathedStartWaypoint", "oPlatformOnTrackStartWaypoint"]

# Port names whose N64 (older decomp) names differ.
ALIASES = {
    "gMarioAnimsBuf": "D_80339D10", "gDemoInputsBuf": "gDemo",
    "gMarioAnimsMemAlloc": "D_80339CF0", "gDemoInputsMemAlloc": "D_80339CF4",
}

# Files whose variables are game state worth carrying over from a Mupen state.
# Everything else (graphics, audio, threads, the level script interpreter,
# allocators, menus' rendering) stays as the port has it.
COPY_UNITS = {
    "area", "behavior_actions", "behavior_script", "camera", "game_init", "hud", "ingame_menu",
    "interaction", "level_update", "macro_special_objects", "mario", "mario_actions_airborne",
    "mario_actions_automatic", "mario_actions_cutscene", "mario_actions_moving", "mario_actions_object",
    "mario_actions_stationary", "mario_actions_submerged", "mario_misc", "mario_step", "obj_behaviors",
    "obj_behaviors_2", "object_collision", "object_helpers", "object_list_processor", "paintings",
    "platform_displacement", "save_file", "spawn_object", "surface_collision", "surface_load",
    "star_select", "buffers", "envfx_snow", "envfx_bubbles",
}
# ...minus these, which are the port's own (rendering, allocation, level load
# products that are heap pointers or must agree with the port's heap).
NO_COPY = {
    "gAreaData", "gAreas", "gCurGraphNodeRoot", "gLoadedGraphNodes",
    "gMarioAnimsMemAlloc", "gDemoInputsBuf", "gMarioAnimsBuf", "gDemoInputsMemAlloc",
    "gDisplayListHead", "gGfxPool", "gGfxSPTask", "gGfxPoolEnd", "gFBSetColor", "gGfxSPTaskYieldBuffer",
    "gGfxSPTaskStack", "gGfxSPTaskOutputBuffer", "gGfxSPTaskOutputBufferSize", "gFrameBuffers",
    "gFrameBuffer0", "gFrameBuffer1", "gFrameBuffer2", "gZBuffer", "gPhysicalFrameBuffers", "gPhysicalZBuffer",
    "gGlobalTimer_unused", "sCurrFBNum", "gMarioAnimsData", "gDemoInputs",
    "gEnvironmentRegions", "gEnvironmentLevels", "gCCMEnteredSlide",
    "sSurfacePool", "sSurfaceNodePool", "sSurfacePoolSize", "gSurfacesAllocated", "gSurfaceNodesAllocated",
    "gNumStaticSurfaces", "gNumStaticSurfaceNodes", "gStaticSurfacePartition",
    "gWarpTransition",
    # Hardware and profiling state, and heap pointers the port manages itself.
    "gObjectMemoryPool", "gControllerStatuses", "gControllerPads", "gGameVblankHandler", "gNumCalls",
    # Constant tables of pointers the generator cannot tell from data.
    "D_80331A54",
    # Vtx is bigger in the port.
    "gSnowTempVtx", "gBubbleTempVtx",
}


def parse_n64_map(path):
    syms = {}
    pat = re.compile(r"^\s+0x([0-9a-f]{16})\s+([A-Za-z_][A-Za-z0-9_]*)\s*$")
    for line in open(path, errors="replace"):
        m = pat.match(line)
        if m:
            addr = int(m.group(1), 16)
            if addr <= 0xFFFFFFFF and m.group(2) not in syms:
                syms[m.group(2)] = addr
    return syms


def parse_port_map(path):
    """(unit, name) -> (addr, size) for data symbols, unit being the object's base name."""
    syms = {}
    by_name = {}
    unit = None
    sect = re.compile(r"^\s+([0-9a-f]+)\s+[0-9a-f-]+\s+([0-9a-f]+)\s+(\S+\.o):\((\S+)\)$")
    sym = re.compile(r"^\s+([0-9a-f]+)\s+[0-9a-f-]+\s+([0-9a-f]+)\s+([A-Za-z_][A-Za-z0-9_.]*)$")
    for line in open(path):
        m = sect.match(line)
        if m:
            unit = os.path.splitext(os.path.basename(m.group(3)))[0]
            continue
        m = sym.match(line)
        if m and unit:
            v = (int(m.group(1), 16), int(m.group(2), 16))
            syms[(unit, m.group(3))] = v
            by_name.setdefault(m.group(3), []).append((unit, v))
    return syms, by_name


# --- DWARF ------------------------------------------------------------------

DIE = re.compile(r"^(0x[0-9a-f]+):(\s+)(DW_TAG_\w+|NULL)")
ATTR = re.compile(r"^\s+(DW_AT_\w+)\s+\((.*)\)\s*$")


def parse_dwarf(text):
    dies = {}
    stack = []
    for line in text.splitlines():
        m = DIE.match(line)
        if m:
            off = int(m.group(1), 16)
            depth = len(m.group(2))
            tag = m.group(3)
            while stack and stack[-1][1] >= depth:
                stack.pop()
            if tag == "NULL":
                continue
            d = {"tag": tag, "attrs": {}, "children": [], "off": off}
            if stack:
                stack[-1][0]["children"].append(d)
            dies[off] = d
            stack.append((d, depth))
            continue
        m = ATTR.match(line)
        if m and stack:
            stack[-1][0]["attrs"][m.group(1)] = m.group(2)
    return dies


def ref(v):
    return int(v.split()[0], 16)


def num(v):
    return int(v.split()[0], 0)


class Layouts:
    """Flattened layouts: lists of (offset, kind, count, stride) where kind is
    a leaf ('s1', 's2', 's4', 's8', 'w' ambiguous word, 'fn', ('p', pointee),
    ('o', size) opaque) or ('L', layout index) for arrays of structs."""

    def __init__(self):
        self.layouts = []
        self.index = {}
        self.pointees = ["?"]

    def pointee(self, name):
        if name not in self.pointees:
            self.pointees.append(name)
        return self.pointees.index(name)

    def add(self, items):
        key = tuple(items)
        if key not in self.index:
            self.index[key] = len(self.layouts)
            self.layouts.append(items)
        return self.index[key]


def strip(dies, t):
    while t is not None:
        d = dies[t]
        if d["tag"] in ("DW_TAG_typedef", "DW_TAG_const_type", "DW_TAG_volatile_type"):
            t = ref(d["attrs"]["DW_AT_type"]) if "DW_AT_type" in d["attrs"] else None
        else:
            return d
    return None


def type_name(dies, t):
    d = strip(dies, t)
    if d is None:
        return "void"
    if d["tag"] in ("DW_TAG_structure_type", "DW_TAG_union_type"):
        return d["attrs"].get("DW_AT_name", '"anon"').strip('"')
    if d["tag"] == "DW_TAG_pointer_type":
        return "ptr"
    if d["tag"] == "DW_TAG_subroutine_type":
        return "fn"
    if d["tag"] == "DW_TAG_base_type":
        return d["attrs"]["DW_AT_name"].strip('"')
    if d["tag"] == "DW_TAG_array_type":
        return type_name(dies, ref(d["attrs"]["DW_AT_type"]))
    return d["tag"]


def size_of(dies, t):
    d = strip(dies, t)
    if d is None:
        return 0
    a = d["attrs"]
    if "DW_AT_byte_size" in a:
        return num(a["DW_AT_byte_size"])
    if d["tag"] == "DW_TAG_pointer_type":
        return 4
    if d["tag"] == "DW_TAG_array_type":
        n = 1
        for c in d["children"]:
            if c["tag"] == "DW_TAG_subrange_type":
                if "DW_AT_count" in c["attrs"]:
                    n *= num(c["attrs"]["DW_AT_count"])
                elif "DW_AT_upper_bound" in c["attrs"]:
                    n *= num(c["attrs"]["DW_AT_upper_bound"]) + 1
                else:
                    n *= 0
        return n * size_of(dies, ref(a["DW_AT_type"]))
    raise ValueError(f"no size for {d['tag']} at {d['off']:#x}")


def leaves(dies, L, t, base, out):
    """Appends (offset, kind) leaves for type t at offset base. Arrays of
    structs become one ('L', idx) item with a count."""
    d = strip(dies, t)
    if d is None:
        return
    tag, a = d["tag"], d["attrs"]
    if tag == "DW_TAG_base_type":
        size = num(a["DW_AT_byte_size"])
        # uintptr_t (unsigned long on wasm32) holds pointers as often as not
        # (behavior stacks, for one): an ambiguous word.
        if size == 4 and a["DW_AT_name"].strip('"') in ("unsigned long", "long"):
            out.append((base, "w", 1, 4))
        else:
            out.append((base, f"s{size}", 1, size))
    elif tag == "DW_TAG_enumeration_type":
        size = num(a["DW_AT_byte_size"])
        out.append((base, f"s{size}", 1, size))
    elif tag == "DW_TAG_pointer_type":
        target = ref(a["DW_AT_type"]) if "DW_AT_type" in a else None
        tn = type_name(dies, target) if target is not None else "void"
        if tn == "fn":
            out.append((base, "fn", 1, 4))
        else:
            out.append((base, ("p", L.pointee(tn)), 1, 4))
    elif tag == "DW_TAG_array_type":
        elem = ref(a["DW_AT_type"])
        esize = size_of(dies, elem)
        count = size_of(dies, t) // esize if esize else 0
        if count == 0:
            return
        sub = []
        leaves(dies, L, elem, 0, sub)
        if len(sub) == 1 and sub[0][2] == 1:
            out.append((base, sub[0][1], count, esize))
        else:
            out.append((base, ("L", L.add(merge(sub))), count, esize))
    elif tag == "DW_TAG_structure_type":
        for c in d["children"]:
            if c["tag"] != "DW_TAG_member":
                continue
            ca = c["attrs"]
            if "DW_AT_bit_size" in ca:
                # Bitfields: bit order differs between the two byte orders.
                off = num(ca.get("DW_AT_data_member_location", "0"))
                out.append((base + off, ("o", 4), 1, 4))
                continue
            off = num(ca.get("DW_AT_data_member_location", "0"))
            leaves(dies, L, ref(ca["DW_AT_type"]), base + off, out)
    elif tag == "DW_TAG_union_type":
        size = num(a["DW_AT_byte_size"])
        members = []
        for c in d["children"]:
            if c["tag"] == "DW_TAG_member":
                sub = []
                leaves(dies, L, ref(c["attrs"]["DW_AT_type"]), 0, sub)
                members.append(expand(L, sub))
        out.extend((base + o, k, 1, s) for (o, k, s) in union_merge(members, size))
    elif tag == "DW_TAG_subroutine_type":
        out.append((base, "fn", 1, 4))
    else:
        raise ValueError(f"unhandled {tag}")


def expand(L, items):
    """Fully expanded (offset, kind, size) leaves."""
    out = []
    for off, kind, count, stride in items:
        for i in range(count):
            if isinstance(kind, tuple) and kind[0] == "L":
                out.extend((off + i * stride + o, k, s) for (o, k, s) in expand(L, L.layouts[kind[1]]))
            else:
                out.append((off + i * stride, kind, kind_size(kind)))
    return out


def kind_size(kind):
    if isinstance(kind, tuple):
        return 4 if kind[0] == "p" else kind[1]
    if kind in ("fn", "w"):
        return 4
    return int(kind[1:])


def union_merge(members, size):
    """One leaf list for a union: where every member splits the bytes the same
    way, 4-byte scalars and pointers merge to an ambiguous word; anything
    else is opaque."""
    grid = {}
    for m in members:
        for off, kind, s in m:
            grid.setdefault(off, set()).add((kind, s))
    out = []
    covered = 0
    for off in sorted(grid):
        kinds = grid[off]
        sizes = {s for _, s in kinds}
        if off < covered:
            continue
        if len(sizes) != 1:
            # Members split this part differently (e.g. s16[2] vs u32).
            s = max(sizes)
            only4 = s == 4 and all(k in ("s4", "w", "fn") or isinstance(k, tuple) and k[0] == "p" or ss == 2 for k, ss in kinds)
            out.append((off, "w" if only4 else ("o", s), s))
            covered = off + s
            continue
        s = sizes.pop()
        ks = {k for k, _ in kinds}
        if len(ks) == 1:
            out.append((off, ks.pop(), s))
        elif s == 4:
            out.append((off, "w", 4))
        else:
            out.append((off, ("o", s), s))
        covered = off + s
    return out


def merge(items):
    """Runs of the same leaf at a regular stride collapse into one item."""
    items = sorted(items, key=lambda x: x[0])
    out = []
    for off, kind, count, stride in items:
        if out:
            po, pk, pc, ps = out[-1]
            if pk == kind and count == 1 and (pc == 1 or ps == stride) and off == po + pc * (ps if pc > 1 else stride):
                out[-1] = (po, pk, pc + 1, stride if pc == 1 else ps)
                continue
        out.append((off, kind, count, stride))
    return out


def behaviors_using(fields, behavior_files_only=False):
    """{behavior name: bitmask of slots} for the fields ({name: slot}) each
    behavior uses: a behavior uses a field if a file defining one of its
    native functions mentions it."""
    files = [] if behavior_files_only else [
        os.path.join(PORT, "src", "game", f) for f in os.listdir(os.path.join(PORT, "src", "game")) if f.endswith(".c")]
    files += [os.path.join(PORT, "src", "game", "behaviors", f) for f in os.listdir(os.path.join(PORT, "src", "game", "behaviors"))]
    func_mask = {}
    defn = re.compile(r"^(?:static\s+)?(?:inline\s+)?[A-Za-z_][\w ]*?\**\s*\b(\w+)\s*\([^;{]*\)\s*\{?\s*$", re.M)
    for path in files:
        text = open(path, errors="replace").read()
        mask = 0
        for f, slot in fields.items():
            if re.search(r"\b" + f + r"\b", text):
                mask |= 1 << slot
        if mask:
            for m in defn.finditer(text):
                func_mask[m.group(1)] = func_mask.get(m.group(1), 0) | mask
    masks = {}
    jumps = {}
    text = open(os.path.join(PORT, "data", "behavior_data.c")).read()
    for m in re.finditer(r"const BehaviorScript (bhv\w+)\[\] = \{(.*?)\};", text, re.S):
        mask = 0
        for fn in re.findall(r"CALL_NATIVE\((\w+)\)", m.group(2)):
            mask |= func_mask.get(fn, 0)
        masks[m.group(1)] = mask
        # Scripts that jump into or call another run its code too.
        jumps[m.group(1)] = re.findall(r"(?:GOTO|CALL)\((bhv\w+)", m.group(2))
    changed = True
    while changed:
        changed = False
        for b, targets in jumps.items():
            for t in targets:
                new = masks[b] | masks.get(t, 0)
                if new != masks[b]:
                    masks[b] = new
                    changed = True
    return {b: m for b, m in masks.items() if m}


def object_fields(kind):
    fields = {}
    for line in open(os.path.join(PORT, "include", "object_fields.h")):
        m = re.match(r"#define\s+/\*0x[0-9A-Fa-f]+\*/\s+(o\w+)\s+OBJECT_FIELD_" + kind + r"\((0x[0-9A-Fa-f]+)", line)
        if m:
            fields[m.group(1)] = int(m.group(2), 16)
    return fields


def s16_slots():
    """Object field slots each behavior uses as two s16s (rather than one
    32-bit value): {behavior name: bitmask}, and the mask every object uses."""
    fields = object_fields("S16")
    generic = {"oDialogResponse", "oDialogState", "oFloorType", "oFloorRoom"}
    generic_mask = 0
    for f in generic:
        generic_mask |= 1 << fields.pop(f)
    return behaviors_using(fields), generic_mask


def compile_unit(src, tmp):
    obj = os.path.join(tmp, src.replace("/", "_") + ".o")
    subprocess.run(["emcc", *CFLAGS, "-o", obj, src], cwd=PORT, check=True)
    text = subprocess.run([DWARFDUMP, "--debug-info", obj], capture_output=True, text=True, check=True).stdout
    return src, text


def rust_kind(kind):
    if isinstance(kind, tuple):
        if kind[0] == "p":
            return f"K::Ptr({kind[1]})"
        if kind[0] == "o":
            return f"K::Opaque({kind[1]})"
        if kind[0] == "L":
            return f"K::Layout({kind[1]})"
    return {"s1": "K::S1", "s2": "K::S2", "s4": "K::S4", "s8": "K::S8", "w": "K::Word", "fn": "K::Fn"}[kind]


def main():
    if not N64_MAP:
        sys.exit(__doc__)
    n64 = parse_n64_map(N64_MAP)
    for port_name, n64_name in ALIASES.items():
        if n64_name in n64:
            n64[port_name] = n64.pop(n64_name)
    port, port_by_name = parse_port_map(os.path.join(BUILD, "sm64.map"))
    wasm = open(os.path.join(BUILD, "sm64.us.wasm"), "rb").read()
    build_id = hashlib.sha1(tools.strip_wasm_data(wasm)).hexdigest()

    srcs = []
    for d in SOURCES:
        for f in sorted(os.listdir(os.path.join(PORT, d))):
            # Only what the headless build compiled (main.c, for one, is not).
            obj = os.path.join(BUILD, d, f[:-2] + ".o")
            if f.endswith(".c") and os.path.exists(obj):
                srcs.append(f"{d}/{f}")
    L = Layouts()
    variables = {}
    named = {}
    with tempfile.TemporaryDirectory() as tmp, ThreadPoolExecutor(int(os.environ.get("JOBS", "2"))) as ex:
        for src, text in ex.map(lambda s: compile_unit(s, tmp), srcs):
            unit = os.path.splitext(os.path.basename(src))[0]
            dies = parse_dwarf(text)
            cu = next((d for d in dies.values() if d["tag"] == "DW_TAG_compile_unit"), None)
            if cu is None:
                continue
            for d in cu["children"]:
                if d["tag"] == "DW_TAG_structure_type" and "DW_AT_name" in d["attrs"] and "DW_AT_byte_size" in d["attrs"]:
                    sname = d["attrs"]["DW_AT_name"].strip('"')
                    if sname not in named:
                        items = []
                        leaves(dies, L, d["off"], 0, items)
                        named[sname] = (num(d["attrs"]["DW_AT_byte_size"]), L.add(merge(items)))
            for v in cu["children"]:
                a = v["attrs"]
                if v["tag"] != "DW_TAG_variable" or "DW_AT_location" not in a or "DW_AT_name" not in a:
                    continue
                name = a["DW_AT_name"].strip('"')
                t = ref(a["DW_AT_type"])
                tn = type_name(dies, t)
                cst = dies[t]["tag"] == "DW_TAG_const_type"
                if (unit, name) not in port or name not in n64:
                    continue
                size = size_of(dies, t)
                items = []
                leaves(dies, L, t, 0, items)
                variables[(unit, name)] = (size, L.add(merge(items)), cst, tn)

    # Every symbol both maps know, as pointer targets: (n64, port, size, name).
    starts = sorted(set(a for a in n64.values()))
    import bisect
    for (unit, name), (size, li, cst, tn) in sorted(variables.items()):
        a = n64[name]
        i = bisect.bisect_right(starts, a)
        if i < len(starts) and starts[i] - a < size and unit in COPY_UNITS and name not in NO_COPY:
            print(f"warning: {name} ({unit}) is {size} bytes but the next N64 symbol is {starts[i] - a} bytes on", file=sys.stderr)
    targets = []
    for name, addr in n64.items():
        if name in port_by_name and len(port_by_name[name]) == 1:
            unit, (paddr, psize) = port_by_name[name][0]
            if psize:
                targets.append((addr, paddr, psize, name, unit))
    targets.sort()

    s16, s16_generic = s16_slots()
    starts = sorted(set(n64.values()))
    import bisect

    out = []
    w = out.append
    w("// Generated by maketools/gen_mupen_tables.py from the US decomp's linker map,")
    w("// the headless build's map and the DWARF of the port's sources. Do not edit.")
    w("#![allow(dead_code)]")
    w("")
    w("use super::layout::{Item, K, Var, Target};")
    w("")
    w(f'/// The headless WASM these addresses are for (its code\'s SHA-1).')
    w(f'pub const BUILD_ID: &str = "{build_id}";')
    w("")
    w("pub static POINTEES: &[&str] = &[" + ", ".join(f'"{p}"' for p in L.pointees) + "];")
    w("")
    w("pub static LAYOUTS: &[&[Item]] = &[")
    for items in L.layouts:
        w("    &[" + ", ".join(f"Item({o}, {rust_kind(k)}, {c}, {s})" for (o, k, c, s) in items) + "],")
    w("];")
    w("")
    w("/// Variables both builds have: (name, unit, N64 address, port address, size, layout, copy, const).")
    w("pub static VARS: &[Var] = &[")
    for (unit, name), (size, li, cst, tn) in sorted(variables.items(), key=lambda kv: n64[kv[0][1]]):
        paddr, psize = port[(unit, name)]
        copy = (unit in COPY_UNITS and name not in NO_COPY and not cst and (unit != "buffers" or name == "gSaveBuffer")) or name in COPY_EXTRA
        w(f'    Var {{ name: "{name}", unit: "{unit}", n64: {n64[name]:#x}, port: {paddr:#x}, size: {size}, layout: {li}, copy: {str(copy).lower()}, constant: {str(cst).lower()} }},')
    w("];")
    w("")
    w("/// N64 addresses of symbols the port does not have (or names differently).")
    w("pub static N64_ONLY: &[(&str, u32)] = &[" + ", ".join(f'("{n}", {n64[n]:#x})' for n in N64_ONLY if n in n64) + "];")
    w("/// Port addresses of port-only symbols.")
    w("pub static PORT_ONLY: &[(&str, u32)] = &[" + ", ".join(f'("{n}", {port[(u, n)][0]:#x})' for u, n in PORT_ONLY) + "];")
    w("")
    w("/// Named structs: (name, size, layout), for structs reached through pointers.")
    w("pub static NAMED: &[(&str, u32, u16)] = &[")
    for sname, (size, li) in sorted(named.items()):
        w(f'    ("{sname}", {size}, {li}),')
    w("];")
    w("")
    w("/// Object field slots used as two s16s by every object (bit n: slot n).")
    w(f"pub const S16_SLOTS_ALL: u128 = {s16_generic:#x};")
    w("/// ...and by these behaviors.")
    w("pub static S16_SLOTS: &[(&str, u128)] = &[")
    for name, mask in sorted(s16.items()):
        w(f'    ("{name}", {mask:#x}),')
    w("];")
    w("")
    pointer_mask = 0
    for kind in ("ANIMS", "WAYPOINT", "OBJ", "SURFACE", "VPTR", "CVPTR", "S16P", "S32P", "CHAIN_SEGMENT"):
        for slot in object_fields(kind).values():
            pointer_mask |= 1 << slot
    w("/// Object field slots some behavior keeps a pointer in (by their declared types).")
    w(f"pub const POINTER_SLOTS: u128 = {pointer_mask:#x};")
    w("")
    waypoints = object_fields("WAYPOINT")
    path_users = behaviors_using({f: waypoints[f] for f in WAYPOINT_FIELDS}, behavior_files_only=True)
    w("/// Behaviors that follow paths (oPathedStartWaypoint and the like, slot 0x1D).")
    w("pub static PATH_BEHAVIORS: &[&str] = &[" + ", ".join(f'"{n}"' for n in sorted(path_users)) + "];")
    w("")
    # Port symbols named after their N64 segmented address (the decomp's
    # `name_seg6_anims_0600E264` convention): what pointers into loaded ROM
    # segments can be, once the loader has checked the contents agree.
    seg_named = []
    for (unit, name), (paddr, psize) in port.items():
        m = re.search(r"_([0-9A-Fa-f]{8})$", name)
        if m and psize:
            seg = int(m.group(1), 16)
            if 0x01000000 <= seg < 0x20000000:
                seg_named.append((seg, paddr, psize))
    seg_named.sort()
    w("/// Port data named after an N64 segmented address: (segmented address, port address, size).")
    w("pub static SEGMENT_NAMED: &[(u32, u32, u32)] = &[")
    for seg, paddr, psize in seg_named:
        w(f"    ({seg:#x}, {paddr:#x}, {psize:#x}),")
    w("];")
    w("")
    w("/// Symbols both maps have, sorted by N64 address: (N64 address, port address, port size, name).")
    w("/// N64 addresses below 0x80000000 are segmented (0x13: behavior scripts).")
    w("pub static TARGETS: &[Target] = &[")
    for (a, p, sz, name, unit) in targets:
        # Only as far as the next N64 symbol, so that a pointer past the end
        # of the last variable (into the heap) is not taken for one into it.
        i = bisect.bisect_right(starts, a)
        if i < len(starts):
            sz = min(sz, starts[i] - a)
        w(f'    Target({a:#x}, {p:#x}, {sz:#x}, "{name}"),')
    w("];")
    open(OUT, "w").write("\n".join(out) + "\n")
    print(f"{len(variables)} variables ({sum(1 for (u, n), v in variables.items() if u in COPY_UNITS and n not in NO_COPY and not v[2])} copied), "
          f"{len(L.layouts)} layouts, {len(targets)} targets, build {build_id} -> {OUT}")


if __name__ == "__main__":
    main()
