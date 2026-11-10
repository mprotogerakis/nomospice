#!/usr/bin/env python3
"""A tutorial page as a video: each picture is shown while the text after it is spoken.

    scripts/docs-video.py docs/tutorial/first-circuit.md            -> build/videos/first-circuit.mp4
    scripts/docs-video.py docs/tutorial/*.md -o build/videos

The page's title and the text before its first picture are spoken over that picture. A picture's
text runs until the next picture; code, notes (!!!) and tables are not spoken, so a page can say
more than its video. A section with text after a picture (the closing summary) is a cue of its own,
over the same picture, its heading spoken first. The spoken text goes into the video as subtitles too, and beside it
as an .srt file.

Speech: macOS `say` (NOMOSPICE_VOICE picks the voice, e.g. "Samantha"), else espeak-ng.
Needs ffmpeg and ffprobe (e.g. `nix shell nixpkgs#ffmpeg -c scripts/docs-video.py …`).
"""
import argparse
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

PAUSE = 0.8  # seconds of quiet after each picture's text

KEYS = {"cmd": "Command", "ctrl": "Control", "shift": "Shift", "alt": "Option", "enter": "Return", "esc": "Escape", "space": "Space"}
SYMBOLS = {"⌘": "Command ", "⌃": "Control ", "⇧": "Shift ", "⌥": "Option ", "↵": "Return", "⌫": "Delete"}


def spoken(md: str) -> str:
    """Markdown as it is read aloud."""
    md = re.sub(r"\+\+([^+]+(?:\+[^+]+)*)\+\+", lambda m: " ".join(KEYS.get(k, k.upper()) for k in m.group(1).split("+")), md)
    md = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", md)  # links: their text
    md = re.sub(r"`\.([a-z]+)", r"dot \1", md)  # `.tran` -> dot tran
    md = re.sub(r"(^|\s)[-*] ", r"\1", md)  # list items: their text
    md = re.sub(r"[`*_]", "", md)
    for s, w in SYMBOLS.items():
        md = md.replace(s, w)
    return re.sub(r"\s+", " ", md).strip()


def scenes(page: pathlib.Path):
    """[(picture, text)] in order, and the page's title."""
    title, out, intro, text = "", [], [], None
    fence = note = False
    for line in page.read_text(encoding="utf-8").splitlines():
        if line.startswith("```"):
            fence = not fence
            continue
        if fence:
            continue
        if line.startswith("!!!") or line.startswith("???"):
            note = True
            continue
        if note:
            if line.startswith("    ") or not line.strip():
                continue
            note = False
        if line.startswith("# "):
            title = line[2:].strip()
            continue
        if line.startswith("#") and text is not None:
            # (a section after a picture: its heading and text a cue of its own, over the same
            # picture; one with no text before the next picture says nothing)
            text = []
            out.append((out[-1][0], text, line.lstrip("#").strip()))
            continue
        if line.startswith("#") or line.lstrip().startswith("|"):
            continue
        m = re.match(r"\s*!\[[^\]]*\]\(([^)]+)\)\s*$", line)
        if m:
            text = []
            out.append(((page.parent / m.group(1)).resolve(), text, ""))
            continue
        (intro if text is None else text).append(line)
    if not out:
        raise ValueError(f"{page}: no pictures, no video")
    result = [(img, spoken(f"{h}: {' '.join(t)}" if h and "".join(t).strip() else " ".join(t))) for img, t, h in out]
    result = [r for k, r in enumerate(result) if r[1] or k == 0]
    result[0] = (result[0][0], spoken(f"{title}. {' '.join(intro)}") + " " + result[0][1])
    return title, result


def say(text: str, to: pathlib.Path):
    """Speech for `text` as a WAV file."""
    if shutil.which("say"):
        voice = os.environ.get("NOMOSPICE_VOICE")
        subprocess.run(["say", *(["-v", voice] if voice else []), "-o", str(to), "--data-format=LEI16@22050", text], check=True)
    elif shutil.which("espeak-ng"):
        subprocess.run(["espeak-ng", "-w", str(to), text], check=True)
    else:
        sys.exit("no speech: needs macOS `say` or espeak-ng")


def seconds(f: pathlib.Path) -> float:
    r = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(f)], check=True, capture_output=True, text=True)
    return float(r.stdout)


def stamp(t: float) -> str:
    ms = round(t * 1000)
    return f"{ms // 3600000:02}:{ms // 60000 % 60:02}:{ms // 1000 % 60:02},{ms % 1000:03}"


def video(page: pathlib.Path, out: pathlib.Path):
    title, parts = scenes(page)
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="nomospice-video-"))
    try:
        clips, srt, t = [], [], 0.0
        for k, (img, text) in enumerate(parts):
            if not img.exists():
                sys.exit(f"{page}: {img} is missing (scripts/docs-shots.sh makes it)")
            wav, clip = tmp / f"{k}.wav", tmp / f"{k}.mp4"
            say(text, wav)
            d = seconds(wav) + PAUSE
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-loop", "1", "-i", str(img), "-i", str(wav),
                            "-vf", "scale=1600:-2,format=yuv420p", "-r", "25", "-c:v", "libx264", "-tune", "stillimage",
                            "-af", f"apad=pad_dur={PAUSE}", "-c:a", "aac", "-b:a", "128k", "-ar", "44100", "-t", f"{d:.3f}", str(clip)], check=True)
            clips.append(clip)
            srt.append(f"{k + 1}\n{stamp(t)} --> {stamp(t + d - PAUSE)}\n{text}\n")
            t += d
        (tmp / "list.txt").write_text("".join(f"file '{c}'\n" for c in clips))
        joined = tmp / "joined.mp4"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "concat", "-safe", "0", "-i", str(tmp / "list.txt"), "-c", "copy", str(joined)], check=True)
        out.parent.mkdir(parents=True, exist_ok=True)
        subs = out.with_suffix(".srt")
        subs.write_text("\n".join(srt), encoding="utf-8")
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(joined), "-i", str(subs), "-c", "copy", "-c:s", "mov_text",
                        "-metadata", f"title={title}", "-metadata:s:s:0", "language=eng", str(out)], check=True)
        print(f"{out}: {len(parts)} pictures, {t:.0f} s")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("pages", nargs="+", type=pathlib.Path)
    ap.add_argument("-o", "--out", type=pathlib.Path, default=pathlib.Path("build/videos"), help="a folder, or one .mp4 for one page")
    a = ap.parse_args()
    for page in a.pages:
        out = a.out if a.out.suffix == ".mp4" else a.out / f"{page.stem}.mp4"
        try:
            video(page, out)
        except ValueError as e:
            print(e)  # (an overview page among the tutorials)


if __name__ == "__main__":
    main()
