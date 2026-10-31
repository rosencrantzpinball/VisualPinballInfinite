"""Finds the programs and libraries Chromium's source tarball carries prebuilt (ELF files: x86-64 ones for Google's
builders, among them its clang, Node.js, gperf, tsgo and DevTools' esbuild) and, with --disarm, takes the execute
bit off every one of them, so that the build fails rather than run one: the build image runs x86-64 programs through
Rosetta, and nothing downloaded is to run. (A library loads without the bit.) What the build needs of them is built
from source instead. Lists each with its machine and kind: a program has an interpreter (as the position-independent
ones, whose header reads as a library's, do) or is a fixed-address executable.

   python3 -I prebuilts.py <chromium src> [--disarm]
"""
import os
import stat
import sys

MACHINES = {0x3E: 'x86-64', 0xB7: 'arm64', 0x28: 'arm', 0x03: 'x86', 0xF3: 'riscv', 0x08: 'mips'}
PT_INTERP = 3


def elf(path):
    try:
        with open(path, 'rb') as f:
            head = f.read(64)
            if len(head) < 52 or head[:4] != b'\x7fELF':
                return None
            wide = head[4] == 2
            order = 'little' if head[5] == 1 else 'big'
            field = lambda at, size: int.from_bytes(head[at:at + size], order)
            kind = field(16, 2)
            machine = field(18, 2)
            if wide:
                phoff, phentsize, phnum = field(32, 8), field(54, 2), field(56, 2)
            else:
                phoff, phentsize, phnum = field(28, 4), field(42, 2), field(44, 2)
            interp = False
            if 0 < phnum < 4096 and phentsize >= 4:
                f.seek(phoff)
                table = f.read(phentsize * phnum)
                interp = any(int.from_bytes(table[i:i + 4], order) == PT_INTERP
                             for i in range(0, len(table) - 3, phentsize))
    except OSError:
        return None
    if kind == 2 or (kind == 3 and interp):
        what = 'program'
    elif kind == 3:
        what = 'library'
    elif kind == 1:
        what = 'object'
    else:
        what = 'other'
    return MACHINES.get(machine, hex(machine)), what


def main():
    src = sys.argv[1]
    disarm = '--disarm' in sys.argv[2:]
    found = []
    for root, dirs, files in os.walk(src):
        # what the build makes, and git's own files, aren't the tarball's
        dirs[:] = [d for d in dirs if not (root == src and d == 'out') and d != '.git']
        for name in files:
            path = os.path.join(root, name)
            if os.path.islink(path):
                continue
            what = elf(path)
            if what is None:
                continue
            mode = os.stat(path).st_mode
            runnable = bool(mode & 0o111)
            if disarm and runnable:
                os.chmod(path, stat.S_IMODE(mode) & ~0o111)
                runnable = False
            found.append((os.path.relpath(path, src), what[0], what[1], runnable))
    for path, machine, kind, runnable in sorted(found):
        print(f'{machine:7} {kind:8} {"runnable" if runnable else "-":8} {path}')
    print(f'{len(found)} ELF files, {sum(1 for f in found if f[2] == "program")} of them programs; '
          f'{sum(1 for f in found if f[3])} with an execute bit', file=sys.stderr)


main()
