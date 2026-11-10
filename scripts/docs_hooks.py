"""MkDocs hook: the tutorials' videos are served with the site.

scripts/docs-video.py makes them in build/videos/ (not in git: 20 MB, and made again whenever a
tutorial changes). A page shows one with `<video>` from `../videos/<name>.mp4`; after the build this
copies build/videos/<name>.mp4 into the site, and its subtitles (.srt) as WebVTT for the player.

`mkdocs gh-deploy` refuses to publish a page whose video is missing (an empty player); a plain
build or `mkdocs serve` only says so, so the pages can be checked without the videos (CI).
"""
import pathlib
import re
import shutil

from mkdocs.exceptions import PluginError

SHOWN = re.compile(r'src="(?:\.\./)*videos/([\w-]+)\.mp4"')
state = {"command": "build", "wanted": set()}


def on_startup(command, **_):
    state["command"] = command


def on_pre_build(config, **_):
    state["wanted"] = set()


def on_page_content(html, page, **_):
    state["wanted"].update(SHOWN.findall(html))
    return html


def on_post_build(config, **_):
    made = pathlib.Path(config.config_file_path).parent / "build" / "videos"
    out = pathlib.Path(config.site_dir) / "videos"
    missing = sorted(n for n in state["wanted"] if not (made / f"{n}.mp4").exists())
    if missing:
        why = f"videos missing in {made}: {', '.join(missing)} (scripts/docs-video.py makes them)"
        if state["command"] == "gh-deploy":
            raise PluginError(why)
        print(f"WARNING - {why}")
    out.mkdir(parents=True, exist_ok=True)
    for name in sorted(state["wanted"] - set(missing)):
        shutil.copy2(made / f"{name}.mp4", out / f"{name}.mp4")
        srt = made / f"{name}.srt"
        if srt.exists():
            vtt = re.sub(r"(\d\d:\d\d:\d\d),(\d\d\d)", r"\1.\2", srt.read_text(encoding="utf-8"))
            (out / f"{name}.vtt").write_text("WEBVTT\n\n" + vtt, encoding="utf-8")
