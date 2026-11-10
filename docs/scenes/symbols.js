// How-to: draw and edit symbols (docs/how-to/symbols.md). A new symbol: a square, two pins.
export default async function (app) {
  await app.theme("light");
  await app.command("new-symbol");
  await app.sleep(1000); // (the new symbol loaded)
  await app.key("w");
  for (const p of [[-32, -32], [32, -32], [32, 32], [-32, 32], [-32, -32]]) await app.click(p);
  await app.key("Escape");
  for (const [name, x] of [["A", -32], ["B", 32]]) {
    await app.key("n");
    await app.fill("Pin name", name);
    await app.ok();
    await app.click([x, 0]);
    await app.key("Escape");
  }
  await app.key(" ");
  await app.shot("01-drawn");
  await app.rightClick([96, 96]);
  await app.shot("02-menu");
  await app.press("Symbol attributes");
  await app.shot("03-attributes");
}
