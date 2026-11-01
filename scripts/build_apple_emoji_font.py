#!/usr/bin/env python3
"""Build the Apple emoji font that the interface uses.

Apple's artwork cannot live in this repository, so the font is generated on the
machine that owns the images. The input is the `img/apple/64` folder of the
`emoji-datasource-apple` package, which is the same art WhatsApp Web ships.
The output is a TrueType font with a CBDT/CBLC strike: DirectWrite and FreeType
both hand those bitmaps to the application, so the same file works on Windows
and on Linux.

    python scripts/build_apple_emoji_font.py --input <dir> --output <file>

The font has one glyph per emoji, an empty outline and the bitmap in the
strike. Multi-codepoint emoji (flags, skin tones, families) are glued by
`liga` rules; `ccmp` drops the variation selector so the rules also match the
text that arrives with U+FE0F.

Requires fontTools (`pip install fonttools`).
"""

from __future__ import annotations

import argparse
import struct
from pathlib import Path

from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.fontBuilder import FontBuilder
from fontTools.ttLib.tables._g_l_y_f import Glyph as Outline
from fontTools.ttLib.tables.DefaultTable import DefaultTable

# The em box is 2048 units, like any TrueType font of the system. The strike
# keeps the 64 px of the source images: the app draws emoji at 15 to 40 px, so
# there is no need for a bigger strike.
UPEM = 2048
STRIKE_PPEM = 64
ASCENT = round(UPEM * 0.8)
DESCENT = UPEM - ASCENT

# Characters that take no room: the zero-width joiner, the variation selector
# that asks for the emoji presentation, and the keycap combiner.
ZERO_WIDTH = {0x200D, 0xFE0F, 0x20E3}

# CBDT image format 17: small metrics, the length of the image and the PNG.
IMAGE_FORMAT_SMALL_METRICS_PNG = 17
# Index format 1: one offset per glyph, relative to the image data offset.
INDEX_FORMAT_OFFSET_ARRAY = 1


class RawTable(DefaultTable):
    """A table whose bytes are written by hand, like CBLC and CBDT."""

    def __init__(self, tag: str, data: bytes) -> None:
        self.tableTag = tag
        self.data = data

    def compile(self, ttFont) -> bytes:  # noqa: ANN001 - fontTools signature
        return self.data

    def toXML(self, writer, ttFont) -> None:  # noqa: ANN001 - fontTools signature
        raise NotImplementedError("the table is generated as binary data")


def line_metrics(ascender: int, descender: int, width: int) -> bytes:
    """SbitLineMetrics of one strike: 12 bytes of small integers."""
    return struct.pack(
        ">bbBbbbbbbbbb",
        ascender,
        descender,
        width,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    )


def build_bitmap_tables(
    bitmaps: list[tuple[int, bytes]], ppem: int, bearing_y: int
) -> tuple[bytes, bytes]:
    """Write the CBLC (location) and CBDT (image) tables of one strike.

    `bitmaps` holds the glyph id and the PNG of every emoji, in glyph order and
    without gaps: the index subtable covers one range.
    """
    first_glyph = bitmaps[0][0]
    last_glyph = bitmaps[-1][0]
    if last_glyph - first_glyph + 1 != len(bitmaps):
        raise SystemExit("The glyphs with bitmap must be consecutive")

    payload = bytearray()
    offsets = []
    for _glyph_id, png in bitmaps:
        offsets.append(len(payload))
        payload += struct.pack(">BBbbB", ppem, ppem, 0, bearing_y, ppem)
        # The length of the image goes between the metrics and the PNG, as Noto
        # and nanoemoji write it: DirectWrite rejects the strike without it.
        payload += struct.pack(">I", len(png))
        payload += png
    offsets.append(len(payload))

    # The CBDT table starts with its version, like the EBDT of the outlines:
    # DirectWrite looks for the images after those four bytes.
    header_size = 4
    subtable = struct.pack(
        ">HHI", INDEX_FORMAT_OFFSET_ARRAY, IMAGE_FORMAT_SMALL_METRICS_PNG, header_size
    )
    subtable += b"".join(struct.pack(">I", offset) for offset in offsets)

    index_array = struct.pack(">HHI", first_glyph, last_glyph, 8)

    header = struct.pack(">HHI", 3, 0, 1)
    size_record = struct.pack(
        ">IIII12s12sHHBBBb",
        len(header) + 48,  # indexSubTableArrayOffset
        len(index_array) + len(subtable),  # indexTablesSize
        1,  # numberOfIndexSubTables
        0,  # colorRef
        line_metrics(bearing_y, bearing_y - ppem, ppem),
        line_metrics(0, 0, 0),
        first_glyph,
        last_glyph,
        ppem,
        ppem,
        32,  # bitDepth: the PNG carries its own colors
        1,  # flags: horizontal
    )

    cblc = header + size_record + index_array + subtable
    return cblc, struct.pack(">HH", 3, 0) + bytes(payload)



def glyph_name(sequence: tuple[int, ...]) -> str:
    """Name of the glyph that draws `sequence`, for example `u1F600`."""
    return "u" + "_".join(f"{codepoint:04X}" for codepoint in sequence)


def read_art(folder: Path) -> tuple[dict[tuple[int, ...], Path], list[tuple[int, ...]]]:
    """Map every emoji to its image, without the variation selector.

    The package names the files with the codepoints in hexadecimal, joined by
    dashes. Six sequences exist twice, with and without U+FE0F, and the art is
    the same: the first file wins. The raw sequences come back too, because the
    `ccmp` rules are built from the text that arrives with U+FE0F.
    """
    art: dict[tuple[int, ...], Path] = {}
    raw: list[tuple[int, ...]] = []
    for path in sorted(folder.glob("*.png")):
        sequence = tuple(int(part, 16) for part in path.stem.split("-"))
        raw.append(sequence)
        key = tuple(codepoint for codepoint in sequence if codepoint != 0xFE0F)
        if key:
            art.setdefault(key, path)
    if not art:
        raise SystemExit(f"No PNG files in {folder}")
    return art, raw


def build_feature_file(sequences: list[tuple[int, ...]], raw: list[tuple[int, ...]], drawn: set[str]) -> str:
    """Write the `ccmp` and `liga` rules that compose the sequences."""
    selectors = set()
    for sequence in raw:
        for current, following in zip(sequence, sequence[1:]):
            if following == 0xFE0F:
                selectors.add(current)

    rules = ["languagesystem DFLT dflt;", "", "feature ccmp {"]
    for codepoint in sorted(selectors):
        base = glyph_name((codepoint,))
        if base in drawn:
            rules.append(f"    sub {base} uFE0F by {base};")
    rules += ["} ccmp;", "", "feature liga {"]
    for sequence in sequences:
        if len(sequence) > 1:
            parts = " ".join(glyph_name((codepoint,)) for codepoint in sequence)
            rules.append(f"    sub {parts} by {glyph_name(sequence)};")
    rules += ["} liga;", ""]
    return "\n".join(rules)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path, help="folder with the 64 px PNG files")
    parser.add_argument("--output", required=True, type=Path, help="font file to write")
    parser.add_argument("--family", default="Apple Color Emoji", help="family name of the font")
    args = parser.parse_args()

    art, raw = read_art(args.input)
    sequences = sorted(art)
    singles = [sequence[0] for sequence in sequences if len(sequence) == 1]

    # Codepoints that only live inside a sequence: the regional indicators of
    # the flags, the tag characters, the medical sign of "health worker". They
    # need a glyph, because the shaper only builds a ligature when every
    # character belongs to the same font. The ones without art stay blank.
    components = sorted({codepoint for sequence in sequences for codepoint in sequence})
    blanks = [glyph_name((codepoint,)) for codepoint in components if (codepoint,) not in art]

    glyph_order = [".notdef"]
    glyph_order += [glyph_name(sequence) for sequence in sequences]
    glyph_order += ["u200D", "uFE0F", "u20E3"]
    glyph_order += blanks
    drawn = {glyph_name(sequence) for sequence in sequences}

    cmap = {codepoint: glyph_name((codepoint,)) for codepoint in singles}
    cmap.update({codepoint: glyph_name((codepoint,)) for codepoint in components if (codepoint,) not in art})
    for codepoint in ZERO_WIDTH:
        cmap[codepoint] = glyph_name((codepoint,))

    zero_width = set(blanks) | {"u200D", "uFE0F", "u20E3"}
    metrics = {}
    for name in dict.fromkeys(glyph_order):
        metrics[name] = (0 if name in zero_width else UPEM, 0)

    glyph_order = list(dict.fromkeys(glyph_order))

    font = FontBuilder(UPEM, isTTF=True)
    font.setupGlyphOrder(glyph_order)
    font.setupCharacterMap(cmap)
    font.setupGlyf({name: Outline() for name in glyph_order})
    font.setupHorizontalMetrics(metrics)
    font.setupHorizontalHeader(ascent=ASCENT, descent=-DESCENT)
    font.setupNameTable(
        {
            "familyName": args.family,
            "styleName": "Regular",
            "uniqueFontIdentifier": f"{args.family};1.0",
            "fullName": args.family,
            "psName": args.family.replace(" ", ""),
            "version": "Version 1.0",
        }
    )
    font.setupOS2(
        sTypoAscender=ASCENT,
        sTypoDescender=-DESCENT,
        sTypoLineGap=0,
        usWinAscent=ASCENT,
        usWinDescent=DESCENT,
        sxHeight=ASCENT,
        sCapHeight=ASCENT,
    )
    font.setupPost(keepGlyphNames=False)

    # The image fills the em box and hangs a little under the baseline, the
    # same place where Segoe UI Emoji draws its glyphs.
    bearing_y = STRIKE_PPEM - round(DESCENT * STRIKE_PPEM / UPEM)
    bitmaps = [
        (font.font.getGlyphID(glyph_name(sequence)), art[sequence].read_bytes())
        for sequence in sequences
    ]
    cblc, cbdt = build_bitmap_tables(bitmaps, STRIKE_PPEM, bearing_y)
    font.font["CBLC"] = RawTable("CBLC", cblc)
    font.font["CBDT"] = RawTable("CBDT", cbdt)

    addOpenTypeFeaturesFromString(font.font, build_feature_file(sequences, raw, drawn))

    args.output.parent.mkdir(parents=True, exist_ok=True)
    font.font.save(args.output)
    size = args.output.stat().st_size
    print(f"{args.output}: {len(sequences)} emoji, {size / 1_000_000:.1f} MB")


if __name__ == "__main__":
    main()
