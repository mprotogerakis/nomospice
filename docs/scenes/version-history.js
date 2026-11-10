// How-to: version history with git (docs/how-to/version-history.md).
export default async function (app) {
  await app.theme("light");
  await app.example("rc_transient");
  const path = app.useS.getState().path;
  await app.invoke("git_init", { path });
  await app.invoke("git_commit", { path, message: "RC low-pass, 1k and 100n" });
  // R1 becomes 2k, saved and committed
  await app.rightClick([136, 64]);
  await app.press("Edit…");
  await app.fill("Value", "2k");
  await app.ok();
  await app.key("s", { metaKey: true });
  await app.invoke("git_commit", { path, message: "R1 = 2k: corner at 796 Hz" });
  // C1 changed, not committed yet
  await app.rightClick([224, 128]);
  await app.press("Edit…");
  await app.fill("Value", "47n");
  await app.ok();
  await app.key("s", { metaKey: true });
  await app.command("history");
  await app.shot("01-history");
  await app.press("Compare");
  await app.shot("02-compare");
}
