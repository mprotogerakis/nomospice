// Tutorial: your first simulation (docs/tutorial/first-circuit.md). An RC low-pass, as the
// rc_transient example has it: V1 at the left, R1 across the top, C1 to ground at the right.
export default async function (app) {
  await app.theme("light");
  await app.shot("01-start");

  // the source, then the resistor turned on its side, then the capacitor
  await app.key("v");
  await app.move([-32, 80]);
  await app.shot("02-place-source");
  await app.click([-32, 80]);
  await app.key("r");
  await app.key("r", { ctrlKey: true });
  await app.click([192, 48]);
  await app.key("c");
  await app.click([208, 96]);
  await app.key("Escape");
  await app.key(" ");
  await app.shot("03-parts");

  // wires: from the source's top up and across to the resistor; from the resistor to the capacitor
  await app.key("w");
  await app.click([-32, 96]);
  await app.click([-32, 64]);
  await app.move([48, 64]);
  await app.shot("04-wire");
  await app.click([96, 64]);
  await app.click([176, 64]);
  await app.click([224, 64]);
  await app.click([224, 96]);
  await app.key("Escape");

  // ground under the source and the capacitor; the nets named in and out
  await app.key("g");
  await app.click([-32, 176]);
  await app.click([224, 160]);
  await app.key("Escape");
  await app.key("n");
  await app.shot("05-label");
  await app.fill("Net name", "in");
  await app.ok();
  await app.click([-32, 64]);
  await app.key("Escape");
  await app.key("n");
  await app.fill("Net name", "out");
  await app.ok();
  await app.click([224, 64]);
  await app.key("Escape");
  await app.shot("06-wired");

  // values: the capacitor 100 nF (the resistor's 1k is right already)
  await app.doubleClick([224, 128]);
  await app.fill("Value", "100n");
  await app.shot("07-capacitor");
  await app.ok();

  // the source's waveform: a sine of 1 V at 1 kHz
  await app.doubleClick([-32, 136]);
  await app.press("Sine");
  await app.fill("DC value", "");
  await app.shot("08-source");
  await app.ok();

  // the analysis: a transient of 3 ms, its directive placed under the circuit
  await app.command("sim-command");
  await app.fill("Stop time", "3m");
  await app.shot("09-analysis");
  await app.ok();
  await app.click([-32, 232]);
  await app.key("Escape");

  // run: the labelled nets are plotted
  await app.key("Enter", { metaKey: true });
  await app.shot("10-run");

  // a click on the capacitor plots its current
  await app.click([224, 128]);
  await app.shot("11-probe");

  // cursors in the third period, settled: A at the input's peak, B at the output's (89 µs later:
  // a phase of atan(2π·1kHz·1kΩ·100nF) = 32°)
  await app.setPlot({ a: 2.25e-3, b: 2.339e-3 });
  await app.shot("12-cursors");
}
