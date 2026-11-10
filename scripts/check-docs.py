#!/usr/bin/env python3
"""Checks of the documentation (docs/, see docs/documentation.md):

  - every picture and every relative link in the pages resolves;
  - every picture in docs/img/ is used by a page, and every scene has its pictures;
  - with a built site (`check-docs.py site`): it fetches nothing from third parties. A web font
    from Google, say, sends every visitor's IP address there before anything is shown.

    python3 scripts/check-docs.py [site]
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
LINK = re.compile(r"!?\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|<img\b[^>]*\bsrc=[\"']([^\"']+)")  # (a title after it; an <img>)
# fetched by the browser on its own (an <a href> is followed only if the visitor chooses to)
FETCHED = re.compile(r"<(?:link|script|img|iframe|source|video|audio|embed|object)\b[^>]*?\b(?:href|src|data)\s*=\s*[\"']((?:https?:)?//[^\"']+)", re.I)
# fetched by scripts and styles: CSS url()/@import, and Material's repository widget (repo_url:
# stars and forks from api.github.com)
# the site's own address (site_url): its own pages, not a third party (a canonical link names it)
OWN = (re.search(r"^site_url:\s*(\S+)", (ROOT / "mkdocs.yml").read_text(), re.M) or [None, "\0"])[1]
STYLE = re.compile(r"(?:url\(\s*[\"']?|@import\s+[\"'])((?:https?:)?//[^\"')]+)", re.I)


def pages():
    for p in sorted(DOCS.rglob("*.md")):
        text = re.sub(r"```.*?```", "", p.read_text(encoding="utf-8"), flags=re.S)  # (code is no link)
        yield p, re.sub(r"`[^`\n]+`", "", text)


def main():
    errors, used = [], set()
    for page, text in pages():
        for target in (a or b for a, b in LINK.findall(text)):
            if re.match(r"[a-z][a-z0-9+.-]*:", target) or target.startswith("#"):
                continue  # (another site, mail)
            path = (page.parent / target.split("#")[0]).resolve()
            if not path.exists():
                errors.append(f"{page.relative_to(ROOT)}: {target} does not exist")
            used.add(path)
    for img in sorted((DOCS / "img").rglob("*.png")):
        if img.resolve() not in used:
            errors.append(f"{img.relative_to(ROOT)}: no page shows it")
    scenes = {p.stem for p in (DOCS / "scenes").glob("*.js")}
    shots = {p.name for p in (DOCS / "img").iterdir() if p.is_dir()}
    errors += [f"docs/scenes/{s}.js: no pictures (scripts/docs-shots.sh {s})" for s in sorted(scenes - shots)]
    errors += [f"docs/img/{s}/: no scene makes these (docs/scenes/{s}.js)" for s in sorted(shots - scenes)]
    if len(sys.argv) > 1:
        site = pathlib.Path(sys.argv[1])
        for html in sorted(site.rglob("*.html")):
            text = html.read_text(encoding="utf-8")
            for url in FETCHED.findall(text) + STYLE.findall(text):
                if not url.startswith(OWN):
                    errors.append(f"{html.relative_to(site)}: fetches {url}")
            if 'data-md-component="source"' in text:
                errors.append(f"{html.relative_to(site)}: the repository widget fetches from api.github.com (repo_url)")
        for css in sorted(site.rglob("*.css")):
            for url in STYLE.findall(css.read_text(encoding="utf-8", errors="replace")):
                if not url.startswith(OWN):
                    errors.append(f"{css.relative_to(site)}: fetches {url}")
    for e in errors:
        print(e)
    print(f"docs: {len(errors)} problem(s)" if errors else "docs: ok")
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
