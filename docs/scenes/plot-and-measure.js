// How-to: plot and measure (docs/how-to/plot-and-measure.md).
export default async function (app) {
  await app.theme("light");
  await app.example("rc_transient");
  await app.key("Enter", { metaKey: true });

  // an expression as a trace
  await app.press("Add trace");
  await app.type("input", "V(in)-V(out)");
  await app.shot("01-add-trace");
  await app.ok();
  await app.shot("02-expression");

  // a trace's own menu
  await app.menuAt("tr[title]", 0.3, 0.5);
  await app.shot("03-trace-menu");
  app.useS.setState({ menu: null });

  // range statistics between A and B
  await app.setPlot({ a: 1e-3, b: 2e-3 });
  await app.shot("04-range");

  // the spectrum of A–B
  await app.menuAt(".uplot", 0.5, 0.3);
  await app.press("FFT (spectrum)");
  await app.shot("05-fft");
}
