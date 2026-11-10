# How this documentation is made

This documentation follows the [Diátaxis](https://diataxis.fr) framework: **tutorials** teach by doing, **how-to guides** solve particular tasks, the **reference** lists facts, and the **explanation** gives background. Its sources are the Markdown files in `docs/` of the repository; [MkDocs](https://www.mkdocs.org) with the Material theme makes the website.

## Build the website

```sh
python3 -m venv .venv && .venv/bin/pip install -r requirements-docs.txt
.venv/bin/mkdocs serve            # preview at http://127.0.0.1:8000, rebuilt on every change
.venv/bin/mkdocs build --strict   # the site in site/
python3 scripts/check-docs.py site
```

`check-docs.py` checks that every picture and link resolves, that every picture is used, and that the built site fetches nothing from third parties (no web fonts, no scripts from elsewhere). CI runs the same.

## Screenshots

Every picture comes from the real app, played by a **scene**: `docs/scenes/<name>.js` drives a development build of nomospice as a user would, pressing keys, clicking the schematic and filling in dialogs, and every `shot(id)` saves a picture of the window as `docs/img/<name>/<id>.png`.

```sh
scripts/docs-shots.sh                    # every scene (macOS)
scripts/docs-shots.sh first-circuit      # one
```

A scene is a function of `app`:

```js
export default async function (app) {
  await app.theme("light");
  await app.key("r");                    // a key, as pressed on the schematic
  await app.click([192, 48]);            // a click at a schematic point
  await app.doubleClick([224, 128]);     // edit the part there
  await app.fill("Value", "100n");       // a field of the open dialog, by its label
  await app.ok();
  await app.key("Enter", { metaKey: true });
  await app.shot("01-run");              // docs/img/<scene>/01-run.png
}
```

Other helpers: `move`, `rightClick`, `type` (into a field by selector), `press` (a button, tab or menu item by its text), `clickAt` and `menuAt` (an element by selector), `load` (a schematic as items, saved in the scene's own folder), `example` (one of the app's examples), `command` (a menu bar command by its id), `setPlot` (cursors, zoom), `useS` (the app's state) and `invoke` (a backend command). See `src/scenes.ts`.

With pngquant installed, the pictures are stored with a palette of their own, a quarter of the size. The app runs under its own identifier, so the installed app's settings and recovery files are not touched, and git commits a scene makes are signed by *nomospice docs*. Change the app, run the scenes again, and the pictures are current.

## Videos

Every tutorial can be turned into a narrated video:

```sh
nix shell nixpkgs#ffmpeg -c scripts/docs-video.py docs/tutorial/*.md    # into build/videos/
```

Each picture of the page is shown while the text that follows it is spoken; the page's title and the text before the first picture are spoken over the first one. Headings, code, tables and notes are not spoken, so a page can say more than its video. The spoken text is in the video as subtitles and beside it as an `.srt` file. The speech comes from macOS `say` (`NOMOSPICE_VOICE` picks a voice) or from espeak-ng.

So write a tutorial as pictures, each followed by a short paragraph that says what to do and what you see.
