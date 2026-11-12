"""Generates assets/snowie.ico (the same snowflake as the tray icon). Standard library only."""
import math
import struct
from pathlib import Path


def seg_dist(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    qx, qy = ax + dx * t - px, ay + dy * t - py
    return math.hypot(qx, qy)


def snowflake(n):
    k = n / 32.0
    c = n / 2.0 - 0.5
    segs = []
    for i in range(6):
        a = i * math.pi / 3 - math.pi / 2
        ca, sa = math.cos(a), math.sin(a)
        p = lambda d: (c + ca * d * k, c + sa * d * k)
        ex, ey = p(14.0)
        segs.append((c, c, ex, ey))
        for d, l in ((6.5, 4.5), (10.5, 3.5)):
            bx, by = p(d)
            for side in (-1, 1):
                b = a + side * 0.75
                segs.append((bx, by, bx + math.cos(b) * l * k, by + math.sin(b) * l * k))
    rows = []
    for y in range(n):
        row = []
        for x in range(n):
            d = min(seg_dist(x + 0.5, y + 0.5, *s) for s in segs) / k
            core = max(0.0, min(1.0, 1.6 - d))
            halo = max(0.0, min(1.0, 2.6 - d))
            a = max(core, halo)
            row.append((int(40 + 190 * core), int(90 + 150 * core), int(170 + 85 * core), int(a * 255)))
        rows.append(row)
    return rows


def bmp_entry(n):
    px = snowflake(n)
    header = struct.pack("<IiiHHIIiiII", 40, n, n * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    data = b"".join(bytes((b, g, r, a)) for row in reversed(px) for (r, g, b, a) in row)
    mask = b"\0" * (((n + 31) // 32) * 4 * n)
    return header + data + mask


sizes = [16, 24, 32, 48, 64, 128, 256]
images = [bmp_entry(n) for n in sizes]
out = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
for n, img in zip(sizes, images):
    out += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(img), offset)
    offset += len(img)
out += b"".join(images)
path = Path(__file__).resolve().parent.parent / "assets" / "snowie.ico"
path.write_bytes(out)
print(f"wrote {path} ({len(out)} bytes)")
