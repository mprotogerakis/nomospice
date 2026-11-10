// How-to: use model libraries (docs/how-to/model-libraries.md).
export default async function (app) {
  await app.theme("light");
  await app.example("ce_amplifier");
  await app.key("p");
  await app.type("input", "2N3904");
  await app.shot("01-search");
  await app.press("Libraries");
  await app.shot("02-libraries");
}
