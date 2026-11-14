#!/usr/bin/env python3
# SPDX-License-Identifier: MIT - Copyright (c) 2026 DBinvent
"""[coverage] The line coverage of an lcov report as a badge - no third party: the SVG is
drawn here, in shields.io's flat look (20 px, 11 px Verdana), and CI keeps it on the
`badges` branch, which the README shows. Prints the percent.

    python3 .github/coverage-badge.py lcov.info coverage.svg [--min PERCENT]

--min: the check (owner, 2026-11-13: yt-light's target is 80) - the badge is written, then the
run fails (exit 1) when the line coverage is below PERCENT, saying by how much.
"""
import os
import sys


def width(text):
    """the text's width in px, at 11 px Verdana"""
    w = 0.0
    for c in text:
        w += (7 if c.isdigit() else 3.6 if c in "ilj.,:;|!'" else 3.9 if c == " "
              else 7.6 if c.isupper() else 10.7 if c in "mw" else 6.7)
    return int(w + 0.999)


def color(pct):
    """shields.io's colours, by how much is covered"""
    return ("#4c1" if pct >= 90 else "#97ca00" if pct >= 75 else "#a4a61d" if pct >= 60
            else "#dfb317" if pct >= 40 else "#e05d44")


def covered(lcov):
    """the percent of the lines found that were hit: the report's LF and LH, summed"""
    found = hit = 0
    with open(lcov) as f:
        for line in f:
            if line.startswith("LF:"):
                found += int(line[3:])
            elif line.startswith("LH:"):
                hit += int(line[3:])
    return 100.0 * hit / found if found else 0.0


def badge(label, value, fill):
    lt, vt = width(label), width(value)
    lw, vw = lt + 10, vt + 10
    w = lw + vw

    def text(x, t, n):
        return (f'<text x="{x}" y="15" fill="#010101" fill-opacity=".3" textLength="{n}" lengthAdjust="spacingAndGlyphs">{t}</text>'
                f'<text x="{x}" y="14" textLength="{n}" lengthAdjust="spacingAndGlyphs">{t}</text>')

    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="20" role="img" aria-label="{label}: {value}">'
            f'<title>{label}: {value}</title>'
            '<linearGradient id="s" x2="0" y2="100%"><stop offset="0" stop-color="#bbb" stop-opacity=".1"/>'
            '<stop offset="1" stop-opacity=".1"/></linearGradient>'
            f'<clipPath id="r"><rect width="{w}" height="20" rx="3" fill="#fff"/></clipPath>'
            f'<g clip-path="url(#r)"><rect width="{lw}" height="20" fill="#555"/>'
            f'<rect x="{lw}" width="{vw}" height="20" fill="{fill}"/><rect width="{w}" height="20" fill="url(#s)"/></g>'
            '<g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">'
            + text(lw / 2, label, lt) + text(lw + vw / 2, value, vt) + "</g></svg>\n")


def main(lcov, out, least=None):
    pct = covered(lcov)
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "w") as f:
        f.write(badge("coverage", f"{pct:.1f}%", color(pct)))
    print(f"{pct:.1f}")
    # the check, after the badge: a figure under the target is shown, and fails the run
    if least is not None and round(pct, 1) < least:
        print(f"line coverage {pct:.1f}% is under the target {least:g}% by {least - pct:.1f}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    least = None
    if "--min" in args:
        i = args.index("--min")
        try:
            least = float(args[i + 1])
        except (IndexError, ValueError):
            sys.exit(__doc__)
        del args[i:i + 2]
    if len(args) != 2:
        sys.exit(__doc__)
    sys.exit(main(args[0], args[1], least))
