// Tutorial: your own block (docs/tutorial/blocks.md). The RC low-pass becomes a block, rcfilter;
// two of them in series make a second-order filter; its schematic and its symbol open from it.
export default async function (app) {
  await app.theme("light");

  // the filter alone: the rc_transient example without its source, in a folder of its own
  const ex = (await app.invoke("examples")).find(([n]) => n === "rc_transient")[1];
  const sheet = ex.filter((it) => !(it.t === "sym" && it.sym === "voltage") && !(it.t === "flag" && it.name === "0" && it.x === -32) && !(it.t === "wire" && it.x1 === -32 && it.x2 === -32) && it.t !== "text");
  await app.load(sheet, "rcfilter.asc");
  await app.key(" ");
  await app.shot("01-sheet");

  // its labels become ports: in an input, out an output
  await app.doubleClick([-56, 64]);
  await app.fill("Port", "In");
  await app.shot("02-port");
  await app.ok();
  await app.doubleClick([248, 64]);
  await app.fill("Port", "Out");
  await app.ok();
  await app.key("s", { metaKey: true });

  // the canvas' menu offers its block symbol
  await app.rightClick([96, 200]);
  await app.shot("03-canvas-menu");
  await app.key("Escape");
  await app.invoke("block_symbol", { path: app.useS.getState().path, items: app.useS.getState().items, replace: true });

  // a new schematic in the same folder: the block is a part there
  await app.load([], "top.asc");
  await app.key("p");
  await app.type("input", "rcfilter");
  await app.shot("04-parts");
  await app.press("Cancel");

  // two blocks in series, driven by an AC source: a second-order low-pass
  const top = [
    { t: "sym", sym: "voltage", x: -160, y: 16, rot: "R0", attrs: { InstName: "V1", Value: "AC 1" } },
    { t: "flag", x: -160, y: 112, name: "0" },
    { t: "wire", x1: -160, y1: 32, x2: -160, y2: 0 },
    { t: "wire", x1: -160, y1: 0, x2: -16, y2: 0 },
    { t: "sym", sym: "rcfilter", x: 32, y: 0, rot: "R0", attrs: { InstName: "X1" } },
    { t: "wire", x1: 80, y1: 0, x2: 224, y2: 0 },
    { t: "flag", x: 128, y: 0, name: "mid" },
    { t: "sym", sym: "rcfilter", x: 272, y: 0, rot: "R0", attrs: { InstName: "X2" } },
    { t: "wire", x1: 320, y1: 0, x2: 368, y2: 0 },
    { t: "flag", x: 368, y: 0, name: "out" },
    { t: "text", x: -160, y: 160, text: ".ac dec 50 10 1Meg", directive: true },
  ];
  await app.load(top, "top.asc");
  await app.key("Enter", { metaKey: true });
  await app.shot("05-cascade");

  // a block's menu: its schematic, its symbol
  await app.rightClick([32, 0]);
  await app.shot("06-block-menu");
  await app.press("Open its schematic");
  await app.shot("07-inside");

  await app.command("up");
  await app.sleep(1500); // (the schematic above, read again)
  await app.rightClick([272, 0]);
  await app.press("Edit its symbol");
  await app.key(" ");
  await app.shot("08-symbol");
}
