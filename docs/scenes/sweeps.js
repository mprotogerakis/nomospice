// Tutorial: sweeps and measurements (docs/tutorial/sweeps.md). The RC low-pass in AC (the rc_bode
// example): its capacitor swept over three values, the corner frequency measured for each.
export default async function (app) {
  await app.theme("light");
  await app.example("rc_bode");
  await app.key("Enter", { metaKey: true });
  await app.shot("01-bode");

  // the capacitor's value becomes a parameter
  await app.rightClick([224, 128]);
  await app.press("Edit…");
  await app.fill("Value", "{C}");
  await app.shot("02-param");
  await app.ok();

  // .step gives it three values
  await app.key("s");
  await app.type("textarea", ".step param C list 47n 100n 220n");
  await app.shot("03-step");
  await app.ok();
  await app.click([-32, 296]);
  await app.key("Escape");
  await app.key("Enter", { metaKey: true });
  await app.shot("04-family");

  // .meas: where the output is 3 dB down, per step
  await app.key("s");
  await app.press("Measurement");
  await app.fill("Analysis", "ac");
  await app.fill("Name", "fc");
  await app.fill("Measure", "when");
  await app.fill("Signal", "vdb(out)");
  await app.fill("Reaches", "-3");
  await app.shot("05-meas");
  await app.ok();
  await app.click([-32, 328]);
  await app.key("Escape");
  await app.key("Enter", { metaKey: true });
  await app.shot("06-measured");

  // tolerances: the Monte Carlo example
  await app.example("monte_carlo");
  await app.key("Enter", { metaKey: true });
  await app.shot("07-monte-carlo");
}
