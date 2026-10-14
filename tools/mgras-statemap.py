#!/usr/bin/env python3
"""Map OpenGL state queries to IMPACT (MGRAS) GE readbacks.

  tools/mgras-statemap.py REC --results glstate.txt [--case N|NAME]... [--map-only]

REC is an `mgras rec` recording of test/gltest/glstate.c; glstate.txt its
output. glstate marks each case with glLineWidth(1 + n / 1000): the
LINE_WIDTH token (0x045) starts a case, glLineWidth(1 + (n + 0.5) / 1000)
ends it (glPushAttrib / glPopAttrib traffic falls outside). For each case this prints the
commands libGLcore sent (the state being set and the query) and the
answers the host read back (reads of the readback registers), then a
consolidated map: every readback token and GE address, with the GL queries
that use it. Queries that send nothing are answered from libGLcore's own
shadow of the state.
"""
import argparse
import importlib.util
import re
import struct
from collections import OrderedDict, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("pixdecode", HERE / "mgras-pixdecode.py")
pd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pd)

LINE_WIDTH = 0x045
# Host reads that carry readback answers (flags at 0x70008 are polled).
ANSWER_REGS = {0x70010, 0x70014, 0x70018, 0x7001C}
# Tokens that ask the GE for something back.
READBACK_HINT = re.compile(r"GET|RETURN|READ|SPIN")
# Push/pop attrib noise around every case (shown only with --all).
NOISE = {0x36}


def events(path):
    """('cmd', kind, word, data, slot) and ('read', off, val) in order."""
    parsers = {}
    slot = None
    image = 0
    for t, bits, off, val in pd.records(path):
        if t == "R" and off in ANSWER_REGS:
            yield ("read", off, val & 0xFFFFFFFF)
            continue
        if t != "W":
            continue
        if off == 0x50050:
            image = 63
            continue
        if off not in (0x70080, 0x70084, 0x74080):
            continue
        ws = [(val >> 32) & 0xFFFFFFFF, val & 0xFFFFFFFF] if bits == 64 else [val & 0xFFFFFFFF]
        for w in ws:
            if image:
                if image == 63:
                    slot = w
                image -= 1
                continue
            p = parsers.setdefault(slot, pd.Parser())
            c = p.push(w)
            if c:
                yield ("cmd",) + c + (slot,)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("rec")
    ap.add_argument("--results", required=True)
    ap.add_argument("--case", action="append", default=[])
    ap.add_argument("--map-only", action="store_true")
    ap.add_argument("--all", action="store_true", help="also show push/pop attrib noise")
    a = ap.parse_args()
    names = pd.token_names()
    cases = {}
    for line in open(a.results):
        m = re.match(r"(PASS|FAIL|SKIP|XFAIL|XPASS) #(\d+) ([^:\s]+):?(.*)", line.strip())
        if m:
            cases[int(m.group(2))] = (m.group(3), m.group(1), m.group(4).lstrip(": "))
    want = set()
    for c in a.case:
        want |= {int(c)} if c.isdigit() else {k for k, v in cases.items() if c in v[0]}

    segs = OrderedDict()
    cur = None
    for ev in events(a.rec):
        if ev[0] == "cmd":
            _, kind, w, d, slot = ev
            if kind == "cmd" and (w >> 8) & 0x1FFF == LINE_WIDTH and d:
                x = (pd.f32(d[0]) - 1.0) * 1000
                n = round(x)
                if 0 < n < 100000 and n in cases and abs(x - n) < 0.2:
                    cur = n
                    segs.setdefault(cur, [])
                    continue
                if abs(x - n) > 0.3 and round(x - 0.5) == cur:
                    cur = None
                    continue
            if cur is not None:
                segs[cur].append(ev)
        elif cur is not None:
            segs[cur].append(ev)

    by_readback = defaultdict(list)
    for n, evs in segs.items():
        name, res, det = cases[n]
        lines = []
        last_rb = None
        for ev in evs:
            if ev[0] == "read":
                lines.append("      -> %s" % pd.fmt_val(ev[2]))
                if last_rb:
                    by_readback[last_rb][-1][2].append(ev[2])
                continue
            _, kind, w, d, slot = ev
            if kind == "pix":
                lines.append("    pixel data %d bytes: %s" % (w & 0xFFFFF, " ".join("%08x" % x for x in d[:8])))
                continue
            cmd = (w >> 8) & 0x1FFF
            if cmd in NOISE and not a.all:
                continue
            tname = names.get(cmd, "tok%03x" % cmd)
            args = " ".join(pd.fmt_val(v) for v in d[:12])
            if READBACK_HINT.search(tname) or cmd in (0xA1, 0xA2):
                key = "%s [%#x] %s" % (tname, cmd, pd.fmt_val(d[0]) if d else "")
                by_readback[key].append((n, name, []))
                last_rb = key
                lines.append("    ? %s [%#x] %s" % (tname, cmd, args))
            else:
                lines.append("    %s [%#x] %s" % (tname, cmd, args))
        if a.map_only or (want and n not in want):
            continue
        print("==== #%d %s %s%s" % (n, name, res, (": " + det) if det else ""))
        if not any(l.lstrip().startswith("?") for l in lines):
            print("    (no readback: answered from libGLcore's shadow)")
        for l in lines:
            print(l)

    print("\n==== readback map: token, first argument (GE address) -> queries")
    def addr_of(k):
        m = re.search(r"\] (0x[0-9a-f]+)", k)
        return int(m.group(1), 16) if m else -1
    for key in sorted(by_readback, key=addr_of):
        uses = by_readback[key]
        qs = sorted({nm for _, nm, _ in uses})
        answers = sorted({pd.fmt_val(v) for _, _, vs in uses for v in vs})
        print("%-40s %s" % (key, ", ".join(qs)))
        if answers:
            print("%-40s   answered: %s" % ("", " ".join(answers[:8]) + (" ..." if len(answers) > 8 else "")))


if __name__ == "__main__":
    main()
