//! A model's characteristic curves for the component library's preview: a diode's I(V), a bipolar
//! transistor's Ic(Vce) per base current, a MOSFET's or JFET's Id(Vds) per gate voltage. Run in a
//! process of its own (`nomospice --curve`): its ngspice is not the app's, so a simulation's results
//! stay as they are.

use serde::{Deserialize, Serialize};

/// What the app sends: the model's kind and name, and its definitions (with what they use).
#[derive(Serialize, Deserialize)]
pub struct Request {
    pub kind: String,
    pub name: String,
    pub defs: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Curve {
    pub label: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Curves {
    pub x: String,
    pub y: String,
    pub curves: Vec<Curve>,
}

/// How a kind is measured: the elements and sweep (`{m}` the model), the stepped parameter, the y
/// expression, the axes' names, whether x and y swap (a diode is swept by its current), whether the
/// sweep runs negative (PNP, PMOS, PJF: drawn as magnitudes).
struct Spec {
    body: &'static str,
    step: Option<&'static str>,
    y: &'static str,
    axes: (&'static str, &'static str),
    swap: bool,
}

fn spec(kind: &str) -> Option<Spec> {
    let s = |body, step, y, axes| Some(Spec { body, step, y, axes, swap: false });
    match kind.to_ascii_uppercase().as_str() {
        "D" => Some(Spec { body: "I1 0 a 0\nD1 a 0 {m}\n.dc I1 0 100m 0.25m", step: None, y: "v(a)", axes: ("V", "I"), swap: true }),
        "NPN" => s("Vce c 0 0\nIb 0 b {ib}\nQ1 c b 0 {m}\n.param ib=20u\n.dc Vce 0 10 0.05", Some(".step param ib list 20u 40u 60u 80u 100u"), "-i(vce)", ("Vce", "Ic")),
        "PNP" | "LPNP" => s("Vce c 0 0\nIb b 0 {ib}\nQ1 c b 0 {m}\n.param ib=20u\n.dc Vce 0 -10 -0.05", Some(".step param ib list 20u 40u 60u 80u 100u"), "i(vce)", ("|Vce|", "|Ic|")),
        "NMOS" => s("Vds d 0 0\nVgs g 0 {vg}\nM1 d g 0 0 {m}\n.param vg=2\n.dc Vds 0 10 0.05", Some(".step param vg list 2 3 4 6 8 10"), "-i(vds)", ("Vds", "Id")),
        "VDMOS" => s("Vds d 0 0\nVgs g 0 {vg}\nM1 d g 0 {m}\n.param vg=2\n.dc Vds 0 10 0.05", Some(".step param vg list 2 3 4 6 8 10"), "-i(vds)", ("Vds", "Id")),
        "PMOS" => s("Vds d 0 0\nVgs g 0 {vg}\nM1 d g 0 0 {m}\n.param vg=-2\n.dc Vds 0 -10 -0.05", Some(".step param vg list -2 -3 -4 -6 -8 -10"), "i(vds)", ("|Vds|", "|Id|")),
        "NJF" => s("Vds d 0 0\nVgs g 0 {vg}\nJ1 d g 0 {m}\n.param vg=0\n.dc Vds 0 10 0.05", Some(".step param vg list 0 -0.5 -1 -1.5 -2"), "-i(vds)", ("Vds", "Id")),
        "PJF" => s("Vds d 0 0\nVgs g 0 {vg}\nJ1 d g 0 {m}\n.param vg=0\n.dc Vds 0 -10 -0.05", Some(".step param vg list 0 0.5 1 1.5 2"), "i(vds)", ("|Vds|", "|Id|")),
        _ => None,
    }
}

/// Whether models of `kind` have curves here.
pub fn shown(kind: &str) -> bool {
    spec(kind).is_some()
}

/// At most `n` points of `v`, evenly picked (the last kept).
fn thin(v: &[f64], n: usize) -> Vec<f64> {
    if v.len() <= n {
        return v.to_vec();
    }
    (0..n).map(|k| v[k * (v.len() - 1) / (n - 1)]).collect()
}

/// The curves of `req`'s model, run in this process's ngspice.
pub fn run(req: &Request) -> Result<Curves, String> {
    let s = spec(&req.kind).ok_or_else(|| format!("no curves for a {} model", req.kind))?;
    let text = format!("* {} curves\n{}\n{}\n.end\n", req.name, s.body.replace("{m}", &req.name), req.defs.join("\n"));
    let deck = crate::schematic::prepare(&text, None, &mut vec![]).with_steps(s.step.iter().map(|l| l.to_string()).collect());
    let run = crate::ngspice::run(&deck).map_err(|log| log.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_else(|| "the model did not run".into()))?;
    let t = crate::ngspice::trace(s.y, &run.scale, run.id)?;
    // (a .step run: one curve per step, end to end)
    let parts: Vec<(String, std::ops::Range<usize>)> = match t.steps.as_deref() {
        Some(ix) if !ix.is_empty() && t.re.len() % ix.len() == 0 => {
            let n = t.re.len() / ix.len();
            ix.iter().enumerate().map(|(k, &i)| (run.steps.get(i).cloned().unwrap_or_default(), k * n..(k + 1) * n)).collect()
        }
        _ => vec![(String::new(), 0..t.re.len())],
    };
    let curves = parts.into_iter().map(|(label, r)| {
        let (x, y) = (&t.x[r.clone()], &t.re[r]);
        let (x, y) = if s.swap { (y, x) } else { (x, y) };
        Curve { label, x: thin(x, 100).iter().map(|v| v.abs()).collect(), y: thin(y, 100).iter().map(|v| v.abs()).collect() }
    }).collect();
    Ok(Curves { x: s.axes.0.into(), y: s.axes.1.into(), curves })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diode_and_transistor_curves() {
        let _s = crate::ngspice::TEST_SIM.lock();
        // a diode: I(V), rising; an NPN: a curve per base current, Ic about BF·Ib in the active region
        let d = run(&Request { kind: "D".into(), name: "DX".into(), defs: vec![".model DX D(IS=1n N=1.8)".into()] }).unwrap();
        let c = &d.curves[0];
        assert!(d.curves.len() == 1 && c.x.windows(2).all(|w| w[1] >= w[0]) && *c.y.last().unwrap() > 0.09, "{d:?}");
        let q = run(&Request { kind: "NPN".into(), name: "QX".into(), defs: vec![".model QX NPN(BF=100)".into()] }).unwrap();
        assert_eq!(q.curves.len(), 5, "{q:?}");
        let ic = q.curves[0].y[q.curves[0].y.len() / 2];
        assert!((ic - 2e-3).abs() < 3e-4 && q.curves[0].label.contains("ib"), "{ic} {}", q.curves[0].label);
        // a MOSFET: a curve per gate voltage; a JFET: per gate voltage, pinched off at the last
        let m = run(&Request { kind: "NMOS".into(), name: "MX".into(), defs: vec![".model MX NMOS(VTO=1 KP=1m)".into()] }).unwrap();
        assert!(m.curves.len() == 6 && m.curves[5].y.last().unwrap() > m.curves[0].y.last().unwrap(), "{m:?}");
        let j = run(&Request { kind: "PJF".into(), name: "JX".into(), defs: vec![".model JX PJF(VTO=-2 BETA=1m)".into()] }).unwrap();
        assert!(j.curves.len() == 5 && *j.curves[0].y.last().unwrap() > 1e-3 && *j.curves[4].y.last().unwrap() < 1e-6, "{j:?}");
        assert!(run(&Request { kind: "SUBCKT".into(), name: "X".into(), defs: vec![] }).is_err());
    }
}
