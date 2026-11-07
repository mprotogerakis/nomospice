//! Laplace sources (`E1 out 0 in 0 Laplace=1/(1+s*tau)`, `G…` likewise) for ngspice, which has
//! none: the expression becomes a rational function of s, built of plain elements (a state-space
//! form: capacitor voltages as its states), so .op, .ac and .tran all see the same H(s); XSPICE's
//! s_xfer holds its states at 0 in the operating point (H(0) lost) and crashes on a constant.
//! Coefficients that hold parameters stay expressions (`{tau}`), so `.param` and `.step` still apply.

/// A polynomial coefficient: a number, or an expression ngspice evaluates (`{...}`, parameters).
#[derive(Clone, Debug, PartialEq)]
enum C {
    N(f64),
    E(String),
}

impl C {
    fn text(&self) -> String {
        match self {
            C::N(v) => format!("{v:e}"),
            C::E(e) => format!("({e})"),
        }
    }
    fn add(&self, o: &C) -> C {
        match (self, o) {
            (C::N(a), C::N(b)) => C::N(a + b),
            (C::N(z), x) | (x, C::N(z)) if *z == 0.0 => x.clone(),
            _ => C::E(format!("{}+{}", self.text(), o.text())),
        }
    }
    fn mul(&self, o: &C) -> C {
        match (self, o) {
            (C::N(a), C::N(b)) => C::N(a * b),
            (C::N(z), _) | (_, C::N(z)) if *z == 0.0 => C::N(0.0),
            (C::N(one), x) | (x, C::N(one)) if *one == 1.0 => x.clone(),
            _ => C::E(format!("{}*{}", self.text(), o.text())),
        }
    }
    fn neg(&self) -> C {
        match self {
            C::N(v) => C::N(-v),
            C::E(e) => C::E(format!("-({e})")),
        }
    }
    fn div(&self, o: &C) -> C {
        match (self, o) {
            (C::N(a), C::N(b)) => C::N(a / b),
            (C::N(z), _) if *z == 0.0 => C::N(0.0),
            (x, C::N(one)) if *one == 1.0 => x.clone(),
            _ => C::E(format!("{}/{}", self.text(), o.text())),
        }
    }
    fn powi(&self, k: i32) -> C {
        match self {
            C::N(v) => C::N(v.powi(k)),
            _ if k == 0 => C::N(1.0),
            _ if k == 1 => self.clone(),
            _ => C::E(format!("pow({},{k})", self.text())),
        }
    }
    /// As SPICE reads it: a number, or `{expression}`.
    fn spice(&self) -> String {
        match self {
            C::N(v) => format!("{v:e}"),
            C::E(e) => format!("{{{e}}}"),
        }
    }
    fn zero(&self) -> bool {
        *self == C::N(0.0)
    }
}

/// Coefficients by ascending power of s.
type Poly = Vec<C>;

fn padd(a: &Poly, b: &Poly) -> Poly {
    let mut out: Poly = (0..a.len().max(b.len())).map(|k| a.get(k).unwrap_or(&C::N(0.0)).add(b.get(k).unwrap_or(&C::N(0.0)))).collect();
    trim(&mut out);
    out
}

fn pmul(a: &Poly, b: &Poly) -> Poly {
    let mut out = vec![C::N(0.0); a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j] = out[i + j].add(&x.mul(y));
        }
    }
    trim(&mut out);
    out
}

fn trim(p: &mut Poly) {
    while p.len() > 1 && p.last().is_some_and(C::zero) {
        p.pop();
    }
}

/// num / den
#[derive(Clone, Debug)]
struct R(Poly, Poly);

impl R {
    fn c(c: C) -> R {
        R(vec![c], vec![C::N(1.0)])
    }
    fn add(&self, o: &R, sub: bool) -> R {
        let on = if sub { o.0.iter().map(C::neg).collect() } else { o.0.clone() };
        if self.1 == o.1 {
            return R(padd(&self.0, &on), self.1.clone()); // (the same denominator: kept, not squared)
        }
        R(padd(&pmul(&self.0, &o.1), &pmul(&on, &self.1)), pmul(&self.1, &o.1))
    }
    fn mul(&self, o: &R) -> R {
        R(pmul(&self.0, &o.0), pmul(&self.1, &o.1))
    }
    fn div(&self, o: &R) -> R {
        R(pmul(&self.0, &o.1), pmul(&self.1, &o.0))
    }
}

/// The expression as a rational function of s: a recursive descent over `+ - * / ^ ( )`, numbers
/// (with SPICE suffixes), `s`, and anything without s kept as written (a parameter, `sqrt(2)`).
struct P<'a> {
    s: &'a str,
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.s[self.i..].starts_with(char::is_whitespace) {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<char> {
        self.ws();
        self.s[self.i..].chars().next()
    }
    fn sum(&mut self) -> Result<R, String> {
        let mut a = self.prod()?;
        while let Some(c @ ('+' | '-')) = self.peek() {
            self.i += 1;
            a = a.add(&self.prod()?, c == '-');
        }
        Ok(a)
    }
    fn prod(&mut self) -> Result<R, String> {
        let mut a = self.unary()?;
        loop {
            match self.peek() {
                Some('*') if !self.s[self.i..].starts_with("**") => {
                    self.i += 1;
                    a = a.mul(&self.unary()?);
                }
                Some('/') => {
                    self.i += 1;
                    a = a.div(&self.unary()?);
                }
                _ => return Ok(a),
            }
        }
    }
    fn unary(&mut self) -> Result<R, String> {
        match self.peek() {
            Some('-') => {
                self.i += 1;
                Ok(R::c(C::N(0.0)).add(&self.unary()?, true))
            }
            Some('+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }
    fn power(&mut self) -> Result<R, String> {
        let a = self.atom()?;
        self.ws();
        let op = if self.s[self.i..].starts_with("**") { 2 } else if self.s[self.i..].starts_with('^') { 1 } else { return Ok(a) };
        self.i += op;
        let e = self.unary()?;
        let Some(y) = constant(&e) else {
            return Err("no s in an exponent".into());
        };
        if let Some(x) = constant(&a) {
            // (no s: one constant, as ngspice reads it)
            return Ok(R::c(match (&x, &y) {
                (C::N(x), C::N(y)) => C::N(x.powf(*y)),
                _ => C::E(format!("pow({},{})", x.text(), y.text())),
            }));
        }
        let k = match y {
            C::N(v) if v.fract() == 0.0 && v.abs() <= 64.0 => v as i32,
            _ => return Err("a power of s must be a whole number (at most 64)".into()),
        };
        let mut out = R::c(C::N(1.0));
        for _ in 0..k.unsigned_abs() {
            out = out.mul(&a);
        }
        Ok(if k < 0 { R::c(C::N(1.0)).div(&out) } else { out })
    }
    fn atom(&mut self) -> Result<R, String> {
        match self.peek() {
            // `{…}`: SPICE's expression braces, read as a group
            Some('{') => {
                let rest = &self.s[self.i..];
                let end = rest.find('}').ok_or("a { without its }")?;
                let mut inner = P { s: &rest[1..end], i: 0 };
                let a = inner.sum()?;
                if inner.peek().is_some() {
                    return Err(format!("cannot read `{}`", &rest[1..end]));
                }
                self.i += end + 1;
                Ok(a)
            }
            Some('(') => {
                self.i += 1;
                let a = self.sum()?;
                if self.peek() != Some(')') {
                    return Err("a ( without its )".into());
                }
                self.i += 1;
                Ok(a)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => {
                let rest = &self.s[self.i..];
                let n = rest.char_indices().find(|&(k, c)| !(c.is_ascii_alphanumeric() || c == '.' || ((c == '+' || c == '-') && k > 0 && rest[..k].ends_with(['e', 'E']) && rest[..k].bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'e' || b == b'E')))).map_or(rest.len(), |p| p.0);
                let v = crate::schematic::spice_num(&rest[..n]).ok_or_else(|| format!("`{}` is not a number", &rest[..n]))?;
                self.i += n;
                Ok(R::c(C::N(v)))
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let start = self.i;
                let rest = &self.s[self.i..];
                let n = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
                let name = &rest[..n];
                self.i += n;
                if name.eq_ignore_ascii_case("s") && self.peek() != Some('(') {
                    return Ok(R(vec![C::N(0.0), C::N(1.0)], vec![C::N(1.0)]));
                }
                // a function call or a parameter: kept as written, if s is not in it
                if self.peek() == Some('(') {
                    let mut depth = 0;
                    for (k, c) in self.s[self.i..].char_indices() {
                        depth += match c { '(' => 1, ')' => -1, _ => 0 };
                        if depth == 0 {
                            self.i += k + 1;
                            break;
                        }
                    }
                    if depth != 0 {
                        return Err("a ( without its )".into());
                    }
                }
                let text = self.s[start..self.i].trim();
                if mentions_s(text) {
                    return Err(format!("`{text}`: only a ratio of polynomials in s runs (no delay exp(-s*T), no sqrt(s))"));
                }
                Ok(R::c(C::E(text.to_string())))
            }
            _ => Err(format!("cannot read `{}`", &self.s[self.i..])),
        }
    }
}

/// The value of an R without s; None: s is in it.
fn constant(r: &R) -> Option<C> {
    (r.0.len() == 1 && r.1.len() == 1).then(|| r.0[0].div(&r.1[0]))
}

/// Whether `s` occurs in `t` as a word of its own.
fn mentions_s(t: &str) -> bool {
    let b = t.as_bytes();
    (0..b.len()).any(|k| {
        b[k].eq_ignore_ascii_case(&b's') && (k == 0 || !(b[k - 1].is_ascii_alphanumeric() || b[k - 1] == b'_')) && (k + 1 == b.len() || !(b[k + 1].is_ascii_alphanumeric() || b[k + 1] == b'_'))
    })
}

/// `expr` (the Laplace= value) as numerator and denominator, by ascending power of s; common
/// factors of s cancelled (`s/(s*(1+s))`).
fn coefficients(expr: &str) -> Result<R, String> {
    let mut p = P { s: expr.trim(), i: 0 };
    let R(mut num, mut den) = p.sum()?;
    if p.peek().is_some() {
        return Err(format!("cannot read `{}`", &p.s[p.i..]));
    }
    while num.len() > 1 && den.len() > 1 && num[0].zero() && den[0].zero() {
        num.remove(0);
        den.remove(0);
    }
    if num.len() > den.len() {
        return Err("more powers of s above than below (an improper transfer function): it cannot run".into());
    }
    if den.iter().all(C::zero) {
        return Err("its denominator is 0".into());
    }
    Ok(R(num, den))
}

/// H(s) = num/den as plain elements, in the controllable canonical form: a state per power of s
/// below, each a capacitor's voltage `<name>_zk` fed by a B source; the output an E (or G) of
/// their sum (`i(E1)` still a current). Time is scaled by w (s = w·p, capacitors of 1/w) and
/// the states by the lowest power below, so they stay near the input's size (ngspice's voltage
/// tolerance is 1 µV: unscaled, a 1 MHz pole's state would be ~1e-13 V). Integrators (no s⁰
/// below) leak through 1 GΩ, so the operating point exists without a loop around them too.
fn realise(name: &str, n: &[&str], h: &R) -> String {
    let R(num, den) = h;
    let ord = den.len() - 1;
    let (out, inp) = (format!("{} {}", n[0], n[1]), format!("V({},{})", n[2], n[3]));
    if ord == 0 {
        return format!("{name} {out} {} {} {}", n[2], n[3], num[0].div(&den[0]).spice());
    }
    let r = den.iter().position(|c| !c.zero()).unwrap_or(0);
    // w: where the poles are, roughly (the zeros' if all poles sit at 0)
    let scale = |lo: &C, hi: &C, k: usize| match lo.div(hi) {
        C::N(v) => C::N(v.abs().powf(1.0 / k as f64)),
        q if k == 1 => C::E(format!("abs({})", q.text())),
        q => C::E(format!("pow(abs({}),{:e})", q.text(), 1.0 / k as f64)),
    };
    let nz: Vec<usize> = (0..num.len()).filter(|&k| !num[k].zero()).collect();
    let w = if ord > r {
        scale(&den[r], &den[ord], ord - r)
    } else if let (Some(&lo), Some(&hi)) = (nz.first(), nz.last()) {
        if hi > lo { scale(&num[lo], &num[hi], hi - lo) } else { C::N(1.0) }
    } else {
        C::N(1.0)
    };
    let w = if w == C::N(0.0) || matches!(w, C::N(v) if !v.is_finite()) { C::N(1.0) } else { w };
    let scaled = |c: &C, k: usize| c.mul(&w.powi(k as i32 - r as i32)).div(&den[r]);
    let a: Vec<C> = (0..=ord).map(|j| scaled(&den[j], j)).collect();
    let b: Vec<C> = (0..=ord).map(|k| num.get(k).map_or(C::N(0.0), |c| scaled(c, k))).collect();
    let low = name.to_ascii_lowercase();
    let z = |k: usize| format!("{low}_z{k}");
    let sum = |terms: Vec<(C, String)>| {
        let t: String = terms.into_iter().filter(|(c, _)| !c.zero()).map(|(c, v)| format!("+({})*{v}", c.spice())).collect();
        if t.is_empty() { "0".to_string() } else { t[1..].to_string() }
    };
    let mut lines = vec![];
    for k in 1..=ord {
        lines.push(format!("C{name}_z{k} {} 0 {}", z(k), C::N(1.0).div(&w).spice()));
        if k <= r {
            lines.push(format!("R{name}_z{k} {} 0 1e9", z(k)));
        }
        if k < ord {
            lines.push(format!("G{name}_z{k} 0 {} {} 0 1", z(k), z(k + 1)));
        }
    }
    // p·z_ord = (u - Σ a_j z_(j+1)) / a_ord
    let mut t = vec![(C::N(1.0).div(&a[ord]), inp.clone())];
    t.extend((0..ord).map(|j| (a[j].div(&a[ord]).neg(), format!("V({})", z(j + 1)))));
    lines.push(format!("B{name}_z{ord} 0 {} I={}", z(ord), sum(t)));
    // y = Σ (b_k - b_ord a_k / a_ord) z_(k+1) + b_ord / a_ord u
    let feed = b[ord].div(&a[ord]);
    let mut t: Vec<(C, String)> = (0..ord).map(|k| (b[k].add(&feed.mul(&a[k]).neg()), format!("V({})", z(k + 1)))).collect();
    t.push((feed, inp));
    lines.push(format!("B{name}_y {low}_y 0 V={}", sum(t)));
    lines.push(format!("{name} {out} {low}_y 0 1"));
    lines.join("\n")
}

/// Every E or G line of `text` with `Laplace=` (in subcircuits too) as an s_xfer device and its
/// model; what cannot run is said in `warnings` and left as it was.
pub fn rewrite(text: &str, warnings: &mut Vec<String>) -> String {
    if !crate::schematic::contains_ci(text, "laplace") {
        return text.to_string(); // (most decks: no pass over them)
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    let mut k = 0;
    while k < lines.len() {
        // a line and its `+` continuations, as one
        let mut j = k + 1;
        while j < lines.len() && lines[j].trim_start().starts_with('+') {
            j += 1;
        }
        let whole = lines[k..j].iter().enumerate().map(|(i, l)| {
            let l = l.trim_end_matches(['\n', '\r']);
            if i == 0 { l } else { &l.trim_start()[1..] }
        }).collect::<Vec<_>>().join(" ");
        match device(&whole) {
            Some(Ok(lines)) => {
                out.push_str(&lines);
                out.push('\n');
            }
            other => {
                if let Some(Err(e)) = other {
                    warnings.push(format!("`{}`: {e}", whole.trim()));
                }
                lines[k..j].iter().for_each(|l| out.push_str(l));
            }
        }
        k = j;
    }
    out
}

/// A Laplace E/G line as plain elements; None: not one.
fn device(l: &str) -> Option<Result<String, String>> {
    let t = l.trim_start();
    let kind = t.chars().next()?.to_ascii_uppercase();
    if kind != 'E' && kind != 'G' {
        return None;
    }
    let low = t.to_ascii_lowercase();
    // `laplace =`, a word of its own (not a node `laplace_out`)
    let at = low.match_indices("laplace").map(|(k, _)| k).find(|&k| (k == 0 || low.as_bytes()[k - 1].is_ascii_whitespace()) && low[k + 7..].trim_start().starts_with('='))?;
    let toks: Vec<&str> = t[..at].split_whitespace().collect();
    if toks.len() != 5 {
        return Some(Err("a Laplace source needs its two output and two input nodes (E1 out 0 in 0 Laplace=…)".into()));
    }
    // the expression: up to an inline comment, or the options (window=, nfft=, mtol=) that
    // have no use here
    let v0 = low.len() - low[at + 7..].trim_start().len() + 1;
    let cut = [";", " $", "\t$", " window", " nfft", " mtol"].iter().filter_map(|o| low[v0..].find(o)).min().unwrap_or(low.len() - v0);
    let h = match coefficients(&t[v0..v0 + cut]) {
        Ok(h) => h,
        Err(e) => return Some(Err(e)),
    };
    Some(Ok(realise(toks[0], &toks[1..], &h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (ascending, as text: numbers in {:e}, expressions as written)
    fn c(e: &str) -> (Vec<String>, Vec<String>) {
        let R(n, d) = coefficients(e).unwrap_or_else(|x| panic!("{e}: {x}"));
        let t = |p: &Poly| p.iter().map(C::spice).collect();
        (t(&n), t(&d))
    }

    #[test]
    fn rational_functions() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(c("1/(1+s*tau)"), (v(&["1e0"]), v(&["1e0", "{tau}"])));
        assert_eq!(c("1/(1+s/1k)"), (v(&["1e3"]), v(&["1e3", "1e0"])));
        assert_eq!(c("1e6/(s^2+s*1e3/0.707+1e6)").1, ["7.07e5", "1e3", "7.07e-1"], "(over 0.707: the same function)");
        assert_eq!(c("(s+1)/(s+2)").0, ["1e0", "1e0"]);
        let w = "{(2e0*(pi))*(fc)}".to_string();
        assert_eq!(c("{2*pi*fc}/(s+2*pi*fc)"), (vec![w.clone()], vec![w, "1e0".to_string()]), "parameters stay expressions");
        assert_eq!(c("{1/(1+s*tau)}"), c("1/(1+s*tau)"), "the whole in braces");
        // a minus right after a power; powers without s any number
        assert_eq!(c("(s^2-s*2+4)/(s^2+s*2+4)").0, ["4e0", "-2e0", "1e0"]);
        assert_eq!(c("{k^0.5}/(1+s)").0, ["{pow((k),5e-1)}"]);
        assert_eq!(c("2**0.5/(1+s)").0, [format!("{:e}", 2f64.sqrt())]);
        assert_eq!(c("s^-1"), (v(&["1e0"]), v(&["0e0", "1e0"])));
        assert_eq!(c("s/(s*(1+s))"), (v(&["1e0"]), v(&["1e0", "1e0"])), "common s cancelled");
        assert!(coefficients("exp(-s*1m)/(1+s)").unwrap_err().contains("delay"));
        assert!(coefficients("s^2/(1+s)").unwrap_err().contains("improper"));
        assert!(coefficients("1/s^1.5").unwrap_err().contains("whole number"));
        assert!(coefficients("2^s").unwrap_err().contains("exponent"));
        assert!(!mentions_s("tau") && mentions_s("2*s+1") && !mentions_s("sigma"));
    }

    #[test]
    fn lines() {
        let mut w = vec![];
        let t = rewrite("E1 out 0 in 0 Laplace=1/(1+s*1m) ; a pole\nG2 a b c d laplace = {gm}/(1+s*{tau}) window=1\nR1 a 0 1k\nE3 laplace_out 0 in 0\n+ Laplace=2\nE4 o 0 i 0 Laplace=3\n+ window=1\n", &mut w);
        assert!(w.is_empty(), "{w:?}");
        assert!(t.contains("CE1_z1 e1_z1 0 1e-3\nBE1_z1 0 e1_z1 I=(1e0)*V(in,0)+(-1e0)*V(e1_z1)\nBE1_y e1_y 0 V=(1e0)*V(e1_z1)\nE1 out 0 e1_y 0 1\n"), "{t}");
        assert!(t.contains("G2 a b g2_y 0 1") && t.contains("{gm}") && t.contains("R1 a 0 1k") && !t.contains("pole") && !t.contains("window"), "{t}");
        assert!(t.contains("E3 laplace_out 0 in 0 2e0\n") && t.contains("E4 o 0 i 0 3e0\n"), "continuations: {t}");
        let t = rewrite("E1 out 0 Laplace=1/s\n", &mut w);
        assert!(w[0].contains("two output and two input") && t.contains("Laplace"), "left as it was, said");
    }

    /// H(s) in ngspice: H(0) at the operating point, a ramp from 1/s, a 2nd-order pole at 1 MHz.
    #[test]
    fn runs() {
        let run = |deck: &str, v: &str| {
            let r = crate::ngspice::run(&crate::schematic::prepare(deck, None, &mut vec![])).unwrap_or_else(|e| panic!("{deck}: {e:?}"));
            crate::ngspice::trace(v, &r.scale, r.id).unwrap()
        };
        let op = run("* op\n.param tau=1m\nV1 in 0 DC 1\nE1 out 0 in 0 Laplace=2/(1+s*tau)\nG1 0 o2 in 0 Laplace=3/(1+s*tau)\nR2 o2 0 1\n.tran 10u 1m\n.end\n", "V(out)");
        assert!((op.re[0] - 2.0).abs() < 1e-6 && (op.re[op.re.len() - 1] - 2.0).abs() < 1e-6, "{:?}", &op.re[..3]);
        let g = run("* op\n.param tau=1m\nV1 in 0 DC 1\nG1 0 o2 in 0 Laplace=3/(1+s*tau)\nR2 o2 0 1\n.tran 10u 1m\n.end\n", "V(o2)");
        assert!((g.re[0] - 3.0).abs() < 1e-6, "G: {}", g.re[0]);
        let ramp = run("* integ\nV1 in 0 PULSE(0 1 1m 1n 1n 1 2)\nE1 out 0 in 0 Laplace=1k/s\n.tran 10u 3m\n.end\n", "V(out)");
        let at = |t: f64| ramp.re[ramp.x.iter().position(|&x| x >= t).unwrap()];
        assert!(at(0.5e-3).abs() < 1e-6 && (at(3e-3) - 2.0).abs() < 0.02, "{} {}", at(0.5e-3), at(3e-3));
        let bw = run("* 2nd order\n.param w=6.2831853*1meg\nV1 in 0 AC 1\nE1 out 0 in 0 Laplace={w*w}/(s*s+s*w*sqrt(2)+w*w)\n.ac dec 50 10k 100meg\n.end\n", "vdb(out)");
        let k = bw.x.iter().position(|&f| f >= 1e6).unwrap();
        assert!((bw.re[k] + 3.01).abs() < 0.05 && bw.re[0].abs() < 1e-3, "{} {}", bw.re[k], bw.re[0]);
    }
}
