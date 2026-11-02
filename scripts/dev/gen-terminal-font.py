#!/usr/bin/env python3
"""gen-terminal-font — the Fabric Terminal's 128-glyph font ROM, from the X11-licensed VGA 8x16 console font.

A cell's glyph field is 7 bits, so the ROM holds 128 glyphs of 16 rows x 8 pixels: 2,048 bytes, one block RAM.
Slots 0x20-0x7E are ASCII. The 32 control slots (and 0x7F) hold what a terminal UI draws with: box lines, blocks,
shades, arrows and markers, in the order of `terminal_protocol::UI_GLYPHS` (read through the protocol table, as every
host reads it). The half blocks and the dark shade are not in the VGA font; they are drawn here, row by row (DRAWN).
Each output line is one glyph row as two hex digits, MSB the leftmost pixel, glyph-major ($readmemh order).

    python3 scripts/dev/gen-terminal-font.py [/usr/share/consolefonts/Uni2-VGA16.psf.gz] > OUT.hex

The font is u_vga16 (Copyright 2000, 2001 Dmitry Bolkhovityanov), shipped by console-setup under the X11 license;
its notice travels with the generated ROM (terminal/term_font.LICENSE).
"""
import gzip
import os
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO, "scripts", "lib"))
import terminal_link  # noqa: E402

# Glyphs drawn rather than taken from the font: 16 row bytes each, MSB the leftmost pixel.
DRAWN = {
    "▓": bytes([0xEE, 0xBB] * 8),   # dark shade: three pixels lit of every four, staggered by row
    "▀": bytes([0xFF] * 8 + [0x00] * 8),
    "▄": bytes([0x00] * 8 + [0xFF] * 8),
    "▌": bytes([0xF0] * 16),
    "▐": bytes([0x0F] * 16),
}


def psf1(data):
    """(glyph bitmaps, {codepoint: glyph index}) of a PSF1 font with a Unicode table."""
    assert data[:2] == b"\x36\x04", "not a PSF1 font"
    mode, height = data[2], data[3]
    count = 512 if mode & 1 else 256
    assert height == 16 and mode & 2, "need an 8x16 PSF1 font with a Unicode table"
    glyphs = [data[4 + i * 16: 4 + (i + 1) * 16] for i in range(count)]
    table, p, index = {}, 4 + count * 16, 0
    while index < count and p + 1 < len(data):
        u = data[p] | (data[p + 1] << 8)
        p += 2
        if u == 0xFFFF:
            index += 1
        elif u != 0xFFFE:
            table.setdefault(u, index)
    return glyphs, table


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/usr/share/consolefonts/Uni2-VGA16.psf.gz"
    glyphs, table = psf1(gzip.open(path).read())
    t = terminal_link.table()
    assert len(t["screen"]["ui_glyphs"]) == 32, t["screen"]["ui_glyphs"]
    rom = []
    for slot in range(128):
        ch = terminal_link.char_of(slot, t)
        if ch in DRAWN:
            rom.extend(DRAWN[ch])
            continue
        if ord(ch) not in table:
            sys.exit(f"the font has no glyph for {ch!r} (slot 0x{slot:02X})")
        rom.extend(glyphs[table[ord(ch)]])
    sys.stdout.write("".join(f"{b:02x}\n" for b in rom))


if __name__ == "__main__":
    main()
