// How-to: poles, zeros and the root locus (docs/how-to/poles-and-zeros.md).
export default async function (app) {
  await app.theme("light");
  await app.example("pz_sallen_key");
  await app.key("Enter", { metaKey: true });
  await app.shot("01-map");
}
