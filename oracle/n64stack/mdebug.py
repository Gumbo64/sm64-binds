"""List every procedure (name, frame size, text offset) in IDO .o files from their ECOFF .mdebug."""
import json, struct, sys, pathlib, subprocess
def section(path, name):
    d = pathlib.Path(path).read_bytes()
    shoff, = struct.unpack_from('>I', d, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from('>HHH', d, 0x2e)
    secs = [struct.unpack_from('>IIIIIIIIII', d, shoff + i * shentsize) for i in range(shnum)]
    strtab = secs[shstrndx]
    for s in secs:
        n = d[strtab[4] + s[0]:].split(b'\0', 1)[0].decode()
        if n == name:
            return d, s[4]
    return d, None
def procs(path):
    d, off = section(path, '.mdebug')
    if off is None:
        return []
    h = struct.unpack_from('>hh23i', d, off)
    assert h[0] == 0x7009, hex(h[0])
    ipdMax, cbPdOffset = h[7], h[8]
    cbSymOffset, cbSsOffset = h[10], h[16]
    ifdMax, cbFdOffset = h[19], h[20]
    out = []
    for f in range(ifdMax):
        fd = struct.unpack_from('>IiiiiiiiiihhiiiiIii', d, cbFdOffset + f * 72)
        adr, issBase, isymBase, ipdFirst, cpd = fd[0], fd[2], fd[4], fd[10], fd[11]
        for p in range(ipdFirst, ipdFirst + cpd):
            pd = struct.unpack_from('>IiiIiiIiihhiii', d, cbPdOffset + p * 52)
            sym = struct.unpack_from('>IiI', d, cbSymOffset + (isymBase + pd[1]) * 12)
            name = d[cbSsOffset + issBase + sym[0]:].split(b'\0', 1)[0].decode()
            st = sym[2] >> 26
            out.append(dict(name=name, frame=pd[8], adr=pd[0], static=(st == 14)))
    return out
def main():
    root = pathlib.Path(sys.argv[1])
    res = {}
    for o in sorted(root.rglob('*.o')):
        try:
            ps = procs(o)
        except Exception as e:
            print('skip', o, e, file=sys.stderr); continue
        if ps:
            res[str(o.relative_to(root))] = ps
    json.dump(res, sys.stdout, indent=0)
if __name__ == '__main__':
    main()
