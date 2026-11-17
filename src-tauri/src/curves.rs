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
        "VDMOS PCHAN" => s("Vds d 0 0\nVgs g 0 {vg}\nM1 d g 0 {m}\n.param vg=-2\n.dc Vds 0 -10 -0.05", Some(".step param vg list -2 -3 -4 -6 -8 -10"), "i(vds)", ("|Vds|", "|Id|")),
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

/// The curves of `req`'s model, run in this process's ngspice.
pub fn run(req: &Request) -> Result<Curves, String> {
    // (a p-channel VDMOS: `pchan` a word of its own among its parameters, not in a comment or a
    // `mfg=…complement_of_pchan…`; swept negative)
    let words = |l: &str| l.split([';', '$']).next().unwrap_or("").split(|c: char| c.is_whitespace() || "(),+".contains(c)).any(|w| w.eq_ignore_ascii_case("pchan"));
    let pchan = req.kind.eq_ignore_ascii_case("VDMOS") && req.defs.iter().flat_map(|d| d.lines()).filter(|l| !l.trim_start().starts_with('*')).any(words);
    let s = spec(if pchan { "VDMOS PCHAN" } else { &req.kind }).ok_or_else(|| format!("no curves for a {} model", req.kind))?;
    let text = format!("* {} curves\n{}\n{}\n.end\n", req.name, s.body.replace("{m}", &req.name), req.defs.join("\n"));
    let deck = crate::schematic::prepare(&text, None, &mut vec![]).with_steps(s.step.iter().map(|l| l.to_string()).collect());
    let run = crate::ngspice::run(&deck).map_err(|log| log.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_else(|| "the model did not run".into()))?;
    let t = crate::ngspice::trace(s.y, &run.scale, run.id)?;
    // (a .step run: one curve per step, end to end over a shared x)
    let parts: Vec<(String, std::ops::Range<usize>)> = match t.steps.as_deref() {
        Some(ix) if !ix.is_empty() => {
            if t.re.len() % ix.len() != 0 || t.x.len() != t.re.len() {
                return Err("the model's curves came out uneven".into());
            }
            let n = t.re.len() / ix.len();
            ix.iter().enumerate().map(|(k, &i)| (run.steps.get(i).cloned().unwrap_or_default(), k * n..(k + 1) * n)).collect()
        }
        _ => vec![(String::new(), 0..t.re.len().min(t.x.len()))],
    };
    let curves: Vec<Curve> = parts.into_iter().filter_map(|(label, r)| {
        // (a step that did not converge has no data: NaN there, no JSON number; left out)
        let pts: Vec<(f64, f64)> = t.x[r.clone()].iter().zip(&t.re[r]).filter(|(a, b)| a.is_finite() && b.is_finite()).map(|(a, b)| if s.swap { (b.abs(), a.abs()) } else { (a.abs(), b.abs()) }).collect();
        let keep = crate::ngspice::even(pts.len(), 100);
        (!keep.is_empty()).then(|| Curve { label, x: keep.iter().map(|&k| pts[k].0).collect(), y: keep.iter().map(|&k| pts[k].1).collect() })
    }).collect();
    if curves.is_empty() {
        return Err("the model did not run".into());
    }
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
        let p = run(&Request { kind: "VDMOS".into(), name: "MP".into(), defs: vec![".model MP VDMOS(Vto=-2 Kp=1\n+pchan)".into()] }).unwrap();
        assert!(p.x == "|Vds|" && *p.curves[5].y.last().unwrap() > 1.0, "{p:?}");
        let n = run(&Request { kind: "VDMOS".into(), name: "MN".into(), defs: vec![".model MN VDMOS(Vto=2 Kp=1 mfg=complement_of_pchan) $ not pchan".into()] }).unwrap();
        assert!(n.x == "Vds" && *n.curves[5].y.last().unwrap() > 1.0, "{n:?}");
        assert!(run(&Request { kind: "SUBCKT".into(), name: "X".into(), defs: vec![] }).is_err());
    }
}
