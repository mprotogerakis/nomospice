// Builds THIRD-PARTY-LICENSES.txt: every third-party component in nomospice with its license text.
// Needs cargo-about (`cargo install cargo-about --locked`). Run after changing dependencies:
//   node scripts/licenses.mjs
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");
const run = (cmd, args, cwd = root) => execFileSync(cmd, args, { cwd, encoding: "utf8", maxBuffer: 1 << 28, shell: process.platform === "win32" });
const third = (f) => readFileSync(join(root, "scripts/third-party", f), "utf8").trim();
const rule = (title) => `\n\n${"=".repeat(80)}\n${title}\n${"=".repeat(80)}\n`;
const item = (title, text) => `\n${"-".repeat(80)}\n${title}\n${"-".repeat(80)}\n\n${text.trim()}\n`;

let out = `nomospice - third-party software and licenses

nomospice is free software under the GNU Affero General Public License v3.0 or later (LICENSE);
source code: https://github.com/mprotogerakis/nomospice. It contains the software listed below.
Each component keeps its own license, reproduced here.`;

out += rule("1. Code adapted into nomospice");
out += item("OpenPCB - design tokens, theme handling, UI primitives (src/index.css, src/lib, src/ui)\nhttps://github.com/OpenPCB-app/OpenPCB - Copyright OpenPCB contributors - AGPL-3.0-or-later",
  "The license text is the GNU Affero General Public License v3.0, the same as nomospice's (see LICENSE).");
out += item("SwiftFlow - canvas interaction (viewport, zoom, MiniMap, Controls, selection box)\nhttps://github.com/aaurelions/SwiftFlow - MIT", third("swiftflow.txt"));

out += rule("2. Bundled binaries, loaded at runtime");
out += item("ngspice 47 - simulation engine, bundled with the Windows installer (official ngspice.dll package) and the\nmacOS app (built unmodified from the ngspice-47 source)\nhttps://ngspice.sourceforge.io - source: https://sourceforge.net/projects/ngspice/files/ng-spice-rework/47/\nModified BSD; some parts LGPL-2.0, LGPL-2.0-or-later, GPL-2.0-or-later (XSPICE table model), MPL-2.0, public domain", third("ngspice.txt"));
out += item("LLVM OpenMP runtime (Windows: libomp140.x86_64.dll, part of ngspice's package)\nhttps://github.com/llvm/llvm-project/tree/main/openmp - Apache-2.0 WITH LLVM-exception", third("llvm-openmp.txt"));
out += item("libsndfile (Windows: sndfile.dll, needed by ngspice.dll)\nhttps://github.com/libsndfile/libsndfile - LGPL-2.1-or-later - source: https://github.com/libsndfile/libsndfile/releases", third("libsndfile.txt"));
out += item("libsamplerate (Windows: samplerate.dll, needed by ngspice.dll)\nhttps://github.com/libsndfile/libsamplerate - BSD-2-Clause", third("libsamplerate.txt"));

// JavaScript packages in the frontend bundle: one entry per distinct license text
const npm = JSON.parse(run("pnpm", ["licenses", "list", "--prod", "--json"]));
const texts = new Map(); // text -> [package@version]
const add = (dir, name, versions, license) => {
  const file = readdirSync(dir).find((f) => /^(licen[cs]e|copying)/i.test(f));
  // Vite's file goes on with the licenses of its own build-time dependencies: not in our bundle
  const text = file ? readFileSync(join(dir, file), "utf8").split("# Licenses of bundled dependencies")[0].trim() : `${license} (the package ships no license file)`;
  texts.set(text, [...(texts.get(text) ?? []), `${name} ${versions} (${license})`]);
};
for (const [license, pkgs] of Object.entries(npm)) for (const p of pkgs) add(p.paths[0], p.name, p.versions.join(", "), license);
// dev dependencies whose code ends up in the bundle: Tailwind's base styles, Vite's preload helper
for (const name of ["tailwindcss", "vite"]) {
  const dir = join(root, "node_modules", name);
  const { version, license } = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  add(dir, name, version, license);
}
out += rule("3. JavaScript packages (frontend)");
for (const [text, users] of texts) out += item(users.join("\n"), text);

// Rust crates compiled into the app (cargo-about: shipped targets, no build/dev-only crates)
const about = JSON.parse(run("cargo", ["about", "generate", "--format", "json", "--locked"], join(root, "src-tauri")));
out += rule("4. Rust crates (compiled into the app)");
for (const l of about.licenses) {
  const users = l.used_by.map((u) => u.crate).filter((c) => c.name !== "nomospice");
  if (users.length) out += item(`${l.name} (${l.id}) - used by:\n${users.map((c) => `${c.name} ${c.version}`).join(", ")}`, l.text);
}

writeFileSync(join(root, "THIRD-PARTY-LICENSES.txt"), out + "\n");

// NOTICE.md: the overview (src/third-party.json, which the app's About dialog shows too)
const parts = JSON.parse(readFileSync(join(root, "src/third-party.json"), "utf8"));
const table = (list) => ["| Component | Used for | License |", "|---|---|---|", ...list.map((c) => `| ${c.url ? `[${c.name}](${c.url})` : c.name} | ${c.use} | ${c.license} |`)].join("\n");
writeFileSync(join(root, "NOTICE.md"), `# Third-party software

nomospice is licensed under the GNU Affero General Public License v3.0 or later ([LICENSE](LICENSE)).
It is built on the software below. The full license texts of every component, including all
Rust crates and JavaScript packages, are in [THIRD-PARTY-LICENSES.txt](THIRD-PARTY-LICENSES.txt);
the app shows them under About (the ⓘ button in the toolbar).

${table(parts.included)}

## Used, but not included

${table(parts.external)}

${parts.notice}
<!-- generated by scripts/licenses.mjs from src/third-party.json -->
`);
console.log(`THIRD-PARTY-LICENSES.txt: ${texts.size} npm license texts, ${about.crates.length - 1} crates`);
