#!/usr/bin/env python3
"""Build bigram/trigram count tables used by the cracker's language scoring.

Usage: build_ngrams.py OUT_FILE corpus1.txt [corpus2.txt ...]

Text is normalised the way Enigma operators wrote it: German umlauts are
expanded (AE, OE, UE, SS) and everything except A-Z is dropped, so the
letters run together with no spaces or punctuation.
"""
import re
import sys
from collections import Counter

SUBST = {"Ä": "AE", "Ö": "OE", "Ü": "UE", "ß": "SS", "É": "E", "È": "E", "À": "A"}


def normalise(text: str) -> str:
    # Drop Project Gutenberg boilerplate if present.
    start = re.search(r"\*\*\* ?START OF.*?\*\*\*", text)
    end = re.search(r"\*\*\* ?END OF", text)
    if start:
        text = text[start.end():end.start() if end else None]
    text = text.upper()
    for k, v in SUBST.items():
        text = text.replace(k, v)
    return re.sub(r"[^A-Z]", "", text)


def main() -> None:
    out, *corpora = sys.argv[1:]
    bi, tri = Counter(), Counter()
    total = 0
    for path in corpora:
        with open(path, encoding="utf-8", errors="ignore") as f:
            s = normalise(f.read())
        total += len(s)
        bi.update(s[i:i + 2] for i in range(len(s) - 1))
        tri.update(s[i:i + 3] for i in range(len(s) - 2))
    with open(out, "w") as f:
        f.write(f"# letters={total} sources={len(corpora)}\n")
        for k, v in sorted(bi.items()):
            f.write(f"{k} {v}\n")
        for k, v in sorted(tri.items()):
            f.write(f"{k} {v}\n")
    print(f"{out}: {total} letters, {len(bi)} bigrams, {len(tri)} trigrams")


if __name__ == "__main__":
    main()
