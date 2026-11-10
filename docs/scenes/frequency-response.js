// Tutorial: frequency response (docs/tutorial/frequency-response.md). The RC low-pass of the
// first tutorial (the rc_transient example), its AC analysis in a result tab of its own.
export default async function (app) {
  await app.theme("light");
  await app.example("rc_transient");

  // the source gets an AC amplitude
  await app.doubleClick([-32, 136]);
  await app.fill("AC amplitude", "1");
  await app.shot("01-ac-amplitude");
  await app.ok();
  await app.key("Enter", { metaKey: true });
  await app.shot("02-transient");

  // a second analysis: the + beside the result tab
  await app.clickAt("button[aria-label='Add an analysis']");
  await app.shot("03-add-analysis");
  await app.press("AC");
  await app.shot("04-ac");
  await app.ok();
  await app.key("Escape");

  // its tab, run: magnitude and phase over frequency
  await app.press("AC");
  await app.key("Enter", { metaKey: true });
  await app.shot("05-bode");

  // the corner frequency, 1/(2π·1kΩ·100nF) = 1.59 kHz: -3 dB and -45°
  await app.setPlot({ a: 1591.5 });
  await app.shot("06-corner");

  // the same as a Nyquist plot
  await app.menuAt(".uplot", 0.5, 0.3);
  await app.shot("07-plot-menu");
  await app.press("Nyquist plot");
  await app.shot("08-nyquist");

  // back to the transient: its results kept (run again: a comment was added since)
  await app.press("Transient");
  await app.key("Enter", { metaKey: true });
  await app.shot("09-tabs");
}
