#!/usr/bin/env python3
"""Writes the static fonts the share-card renderer embeds (crates/core/src/cover/card.rs).

The site's variable Latin fonts (@fontsource-variable/inter and /newsreader, installed in apps/web)
are pinned to the weights the card draws and cut down to printable ASCII plus the few symbols the
covers print, with the family renamed to plain "Inter" / "Newsreader" so the SVG's font-family
matches. Needs fonttools and brotli (WOFF2): `pip install fonttools brotli`. Run from the repo root:
`python3 scripts/build-card-fonts.py`. Rerun only when the cover text gains new characters.
"""
from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

SRC = "apps/web/node_modules/@fontsource-variable/{0}/files/{0}-latin-wght-normal.woff2"
OUT = "crates/core/data/fonts/{}-{}.ttf"
# The caption and labels use 500/600/700 (graphic.rs), the wordmark Newsreader 700 like the masthead.
FONTS = [("inter", "Inter", 500), ("inter", "Inter", 600), ("inter", "Inter", 700), ("newsreader", "Newsreader", 700)]
# Printable ASCII, the U+2212 minus, en/em dash, middle dot, right quote.
CHARS = list(range(0x20, 0x7F)) + [0x2212, 0x2013, 0x2014, 0x00B7, 0x2019]

for pkg, family, weight in FONTS:
    font = instantiateVariableFont(TTFont(SRC.format(pkg)), {"wght": weight})
    font.flavor = None
    opts = subset.Options()
    opts.layout_features = ["kern"]
    opts.hinting = False
    opts.name_IDs = [0, 1, 2, 4, 6, 13, 14]
    s = subset.Subsetter(opts)
    s.populate(unicodes=CHARS)
    s.subset(font)
    name = font["name"]
    for rec in list(name.names):
        if rec.nameID in (16, 17):
            name.removeNames(nameID=rec.nameID)
    style = {500: "Medium", 600: "SemiBold", 700: "Bold"}[weight]
    name.setName(family, 1, 3, 1, 0x409)
    name.setName(style, 2, 3, 1, 0x409)
    name.setName(f"{family} {style}", 4, 3, 1, 0x409)
    name.setName(f"{family}-{style}", 6, 3, 1, 0x409)
    font["OS/2"].usWeightClass = weight
    font.save(OUT.format(pkg, weight))
    print(OUT.format(pkg, weight))
