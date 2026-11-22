#!/usr/bin/env python3
"""Rasterize `homelab render --cells` JSON into a PNG (half blocks become square pixels).

    homelab --data demo render --width 140 --height 40 --cells | scripts/screenshot.py out.png
"""
import json, sys
from PIL import Image, ImageDraw, ImageFont

d = json.load(sys.stdin)
W, H, cw, ch = d["w"], d["h"], 9, 18
img = Image.new("RGB", (W * cw, H * ch), (11, 11, 13))
dr = ImageDraw.Draw(img)
font = None
for f in ["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", "/System/Library/Fonts/Menlo.ttc"]:
    try:
        font = ImageFont.truetype(f, 13); break
    except OSError:
        pass
font = font or ImageFont.load_default()
PARTIALS = {"▏": 1, "▎": 2, "▍": 3, "▌": 4, "▋": 5, "▊": 6, "▉": 7, "█": 8}
BRAILLE_BITS = ((0x01, 0x08), (0x02, 0x10), (0x04, 0x20), (0x40, 0x80))

for c in d["cells"]:
    x, y = c["x"] * cw, c["y"] * ch
    fg = tuple(c["fg"]) if c["fg"] else (220, 220, 220)
    bg = tuple(c["bg"]) if c["bg"] else (11, 11, 13)
    s = c["s"]
    dr.rectangle([x, y, x + cw - 1, y + ch - 1], fill=bg)
    if s == "▀":
        dr.rectangle([x, y, x + cw - 1, y + ch // 2 - 1], fill=fg)
    elif s == "▄":
        dr.rectangle([x, y + ch // 2, x + cw - 1, y + ch - 1], fill=fg)
    elif s in PARTIALS:
        w = max(1, round(cw * PARTIALS[s] / 8))
        dr.rectangle([x, y + 3, x + w - 1, y + ch - 4], fill=fg)
    elif s == "╱":
        dr.line([(x + 1, y + ch - 4), (x + cw - 2, y + 3)], fill=fg, width=1)
    elif len(s) == 1 and 0x2800 <= ord(s) <= 0x28FF:
        mask = ord(s) - 0x2800
        for dy in range(4):
            for dx in range(2):
                if mask & BRAILLE_BITS[dy][dx]:
                    px = x + 1 + dx * 4
                    py = y + 1 + dy * 4
                    dr.rectangle([px, py, px + 2, py + 2], fill=fg)
    elif s.strip():
        dr.text((x, y + 2), s, fill=fg, font=font)
img.save(sys.argv[1])
