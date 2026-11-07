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
    // ponytail: a factor is cancelled only where it is written alike above and below (G/(1+G):
    // G's denominator); a numeric polynomial gcd if other forms leave hidden pole-zero pairs
    fn mul(&self, o: &R) -> R {
        if self.0 == o.1 {
            return R(o.0.clone(), self.1.clone());
        }
        if self.1 == o.0 {
            return R(self.0.clone(), o.1.clone());
        }
        R(pmul(&self.0, &o.0), pmul(&self.1, &o.1))
    }
    fn div(&self, o: &R) -> R {
        self.mul(&R(o.1.clone(), o.0.clone()))
    }
}

/// The expression as a rational function of s: a recursive descent over `+ - * / ^ ( )`, numbers
/// (with SPICE suffixes), `s`, and anything without s kept as written (a parameter, `sqrt(2)`).
struct P<'a> {
    s: &'a str,
    i: usize,
    /// how deep in parentheses (a stack's worth at most)
    d: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        let r = &self.s[self.i..];
        self.i += r.len() - r.trim_start().len();
    }
    fn peek(&mut self) -> Option<char> {
        self.ws();
        self.s[self.i..].chars().next()
    }
    fn sum(&mut self) -> Result<R, String> {
        let mut a = self.prod()?;
        while let Some(c @ ('+' | '-')) = self.peek() {
            self.i += 1;
            a = sized(a.add(&self.prod()?, c == '-'))?;
        }
        Ok(a)
    }
    fn prod(&mut self) -> Result<R, String> {
        let mut a = self.unary()?;
        loop {
            match self.peek() {
                Some('*') if !self.s[self.i..].starts_with("**") => {
                    self.i += 1;
                    a = sized(a.mul(&self.unary()?))?;
                }
                Some('/') => {
                    self.i += 1;
                    a = sized(a.div(&self.unary()?))?;
                }
                _ => return Ok(a),
            }
        }
    }
    fn unary(&mut self) -> Result<R, String> {
        if self.d > 100 {
            return Err("nested too deep".into());
        }
        self.d += 1;
        let r = self.unary1();
        self.d -= 1;
        r
    }
    fn unary1(&mut self) -> Result<R, String> {
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
            out = sized(out.mul(&a))?;
        }
        Ok(if k < 0 { R::c(C::N(1.0)).div(&out) } else { out })
    }
    fn atom(&mut self) -> Result<R, String> {
        match self.peek() {
            // `{…}`: SPICE's expression braces, read as a group
            Some('{') => {
                let rest = &self.s[self.i..];
                let end = rest.find('}').ok_or("a { without its }")?;
                let mut inner = P { s: &rest[1..end], i: 0, d: self.d };
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
                let n = rest.char_indices().find(|&(k, c)| !(c.is_alphanumeric() || c == '.' || ((c == '+' || c == '-') && k > 0 && rest[..k].ends_with(['e', 'E']) && rest[..k].bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'e' || b == b'E')))).map_or(rest.len(), |p| p.0);
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

/// `r`, unless its order or its coefficients' expressions have grown past use (powers and products of
/// parameter polynomials grow like binomial coefficients: `(1+s*tau/20)^20`).
fn sized(r: R) -> Result<R, String> {
    if r.0.len().max(r.1.len()) > 65 {
        return Err("its order is above 64".into());
    }
    let len: usize = r.0.iter().chain(&r.1).map(|c| if let C::E(e) = c { e.len() } else { 0 }).sum();
    if len > 20_000 {
        return Err("it expands too far with parameters in it (multiplied out, its coefficients run past 20,000 characters): write it with numbers".into());
    }
    Ok(r)
}

/// The value of an R without s; None: s is in it.
fn constant(r: &R) -> Option<C> {
    (r.0.len() == 1 && r.1.len() == 1).then(|| r.0[0].div(&r.1[0]))
}

/// Whether ngspice's vector `v` is one of `realise`'s nodes (`e1#lapz1`, `x1.e1#lapy`): not
/// listed with the results.
pub fn internal(v: &str) -> bool {
    v.ends_with("#lapy") || v.rsplit_once("#lapz").is_some_and(|(_, k)| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()))
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
    let mut p = P { s: expr.trim(), i: 0, d: 0 };
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
    if num.iter().chain(&den).any(|c| matches!(c, C::N(v) if !v.is_finite())) {
        return Err("a coefficient is not a number (a division by 0, a root of a negative number)".into());
    }
    Ok(R(num, den))
}

/// H(s) = num/den as plain elements, in the controllable canonical form: a state per power of s
/// below, each a capacitor's voltage `<name>#lapzk` fed by G sources; the output an E (or G) of
/// their sum, `<name>#lapy` (`i(E1)` still a current; results do not list these nodes). Time is scaled by w (s = w·p, capacitors of 1/w) and
/// the states by the lowest power below, so they stay near the input's size (ngspice's voltage
/// tolerance is 1 µV: unscaled, a 1 MHz pole's state would be ~1e-13 V). Integrators (no s⁰
/// below) leak through 1 nS, so the operating point exists without a loop around them too (a DC
/// input on one has none: there it is ~1e9 times the input; `.tran … uic` starts it at 0).
fn realise(name: &str, n: &[&str], h: &R) -> String {
    let R(num, den) = h;
    let ord = den.len() - 1;
    let out = format!("{} {}", n[0], n[1]);
    if ord == 0 {
        return format!("{name} {out} {} {} {}", n[2], n[3], num[0].div(&den[0]).spice());
    }
    // ponytail: r skips only coefficients written as 0; a parameter that is 0 at run time
    // (`1/(s+a)`, `.param a=0`) divides by 0 there: test it ahead (ngspice's ternary) if needed
    let r = den.iter().position(|c| !c.zero()).unwrap_or(0);
    // w: where the poles are, roughly (the zeros' if all poles sit at 0)
    let scale = |lo: &C, hi: &C, k: usize| match lo.div(hi) {
        C::N(v) => C::N(v.abs().powf(1.0 / k as f64)),
        q if k == 1 => C::E(format!("abs({})", q.text())),
        q => C::E(format!("pow(abs({}),{:e})", q.text(), 1.0 / k as f64)),
    };
    // (all poles at 0, `wc/s`, `K*(1+s*T)/(s*T)`: where |H| = 1 if the lowest power above
    // ruled, its crossover)
    let lo = num.iter().position(|c| !c.zero()).unwrap_or(ord);
    let w = if ord > r {
        scale(&den[r], &den[ord], ord - r)
    } else if ord > lo {
        scale(&num[lo], &den[ord], ord - lo)
    } else {
        C::N(1.0)
    };
    let w = if w == C::N(0.0) || matches!(w, C::N(v) if !v.is_finite()) { C::N(1.0) } else { w };
    let scaled = |c: &C, k: usize| c.mul(&w.powi(k as i32 - r as i32)).div(&den[r]);
    let a: Vec<C> = (0..=ord).map(|j| scaled(&den[j], j)).collect();
    let b: Vec<C> = (0..=ord).map(|k| num.get(k).map_or(C::N(0.0), |c| scaled(c, k))).collect();
    // (all linear G: node names like `in+` are no operators here, and .pz takes them)
    let low = name.to_ascii_lowercase();
    // ('#': no net of a schematic has it, so results can leave these out by name)
    let z = |k: usize| format!("{low}#lapz{k}");
    let y = format!("{low}#lapy");
    let mut lines = vec![];
    let mut g = |tag: String, to: &str, from: &str, c: C| {
        if !c.zero() {
            lines.push(format!("G{name}_{tag} 0 {to} {from} {}", c.spice()));
        }
    };
    let u = format!("{} {}", n[2], n[3]);
    // p·z_ord = (u - Σ a_j z_(j+1)) / a_ord
    g("u".into(), &z(ord), &u, C::N(1.0).div(&a[ord]));
    for j in 0..ord {
        g(format!("a{j}"), &z(ord), &format!("{} 0", z(j + 1)), a[j].div(&a[ord]).neg());
    }
    // y = Σ (b_k - b_ord a_k / a_ord) z_(k+1) + b_ord / a_ord u, as a current through 1 S
    // (conductances as G sources of their own voltage: noiseless, unlike resistors)
    let feed = b[ord].div(&a[ord]);
    for k in 0..ord {
        g(format!("b{k}"), &y, &format!("{} 0", z(k + 1)), b[k].add(&feed.mul(&a[k]).neg()));
    }
    g("f".into(), &y, &u, feed);
    for k in 1..=ord {
        lines.push(format!("C{name}_z{k} {} 0 {}", z(k), C::N(1.0).div(&w).spice()));
        if k <= r {
            lines.push(format!("G{name}_l{k} {} 0 {} 0 1e-9", z(k), z(k)));
        }
        if k < ord {
            lines.push(format!("G{name}_z{k} 0 {} {} 0 1", z(k), z(k + 1)));
        }
    }
    lines.push(format!("G{name}_y {y} 0 {y} 0 1"));
    lines.push(format!("{name} {out} {y} 0 1"));
    lines.join("\n")
}

/// Every E or G line of `text` with `Laplace=` (in subcircuits too) as plain elements (see
/// `realise`); what cannot run is said in `warnings` and left as it was.
pub fn rewrite(text: &str, warnings: &mut Vec<String>) -> String {
    if !crate::schematic::contains_ci(text, "laplace") {
        return text.to_string(); // (most decks: no pass over them)
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    let mut k = 0;
    while k < lines.len() {
        // (not the title, whatever it says)
        if k == 0 || !lines[k].trim_start().starts_with(['E', 'e', 'G', 'g']) {
            out.push_str(lines[k]);
            k += 1;
            continue;
        }
        // a line and its `+` continuations, as one (comments and blank lines between them
        // skipped, as ngspice does); each without its inline comment
        let (mut j, mut end) = (k + 1, k + 1);
        while let Some(l) = lines.get(j).map(|l| l.trim_start()) {
            match l.chars().next() {
                Some('+') => end = j + 1,
                Some('*') | None => {}
                _ => break,
            }
            j += 1;
        }
        let j = end;
        let whole = lines[k..j].iter().enumerate().filter_map(|(i, l)| {
            let l = crate::schematic::uncommented(l.trim_end_matches(['\n', '\r']));
            if i == 0 { Some(l) } else { l.trim_start().strip_prefix('+') }
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

/// An E/G line (with its continuations) as plain elements, if it has `Laplace=`; None: not.
fn device(l: &str) -> Option<Result<String, String>> {
    let t = l.trim_start();
    let low = t.to_ascii_lowercase();
    // `laplace =`, a word of its own (not a node `laplace_out`)
    let at = low.match_indices("laplace").map(|(k, _)| k).find(|&k| (k == 0 || low.as_bytes()[k - 1].is_ascii_whitespace()) && low[k + 7..].trim_start().starts_with('='))?;
    let toks: Vec<&str> = t[..at].split_whitespace().collect();
    if toks.len() != 5 {
        return Some(Err("a Laplace source needs its two output and two input nodes (E1 out 0 in 0 Laplace=…)".into()));
    }
    // the expression: up to the options (window=, nfft=, mtol=) that have no use here
    let v0 = low.len() - low[at + 7..].trim_start().len() + 1;
    let rest = &low[v0..];
    let option = |k: usize| ["window", "nfft", "mtol"].iter().any(|o| rest[k..].trim_start().strip_prefix(o).is_some_and(|x| x.trim_start().starts_with('=')));
    let cut = rest.char_indices().find(|&(k, c)| c.is_whitespace() && option(k)).map_or(rest.len(), |p| p.0);
    let h = match coefficients(&t[v0..v0 + cut]) {
        Ok(h) => h,
        Err(e) => return Some(Err(e)),
    };
    let t = realise(toks[0], &toks[1..], &h);
    Some(if t.len() < 200_000 { Ok(t) } else { Err("with its parameters it would write more than 200 kB of netlist: write it with numbers".into()) })
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
        assert!(coefficients("1/(1+s*tau/20)^20").unwrap_err().contains("20,000"));
        assert!(coefficients("1/((1+s)^64)^64").unwrap_err().contains("order"));
        assert!(!mentions_s("tau") && mentions_s("2*s+1") && !mentions_s("sigma"));
        assert!(internal("e1#lapz1") && internal("x1.e1#lapy") && !internal("out") && !internal("clk_lapz1") && !internal("a#lapz1x"));
        assert_eq!(c("(1/(s-1))/(1+10/(s-1))"), (v(&["1e0"]), v(&["9e0", "1e0"])), "G/(1+G): no hidden pole at +1");
        assert!(coefficients("(-2)^0.5/(1+s)").unwrap_err().contains("not a number"));
        assert!(coefficients(&format!("{}1{}", "(".repeat(100_000), ")".repeat(100_000))).unwrap_err().contains("deep"));
    }

    #[test]
    fn lines() {
        let mut w = vec![];
        let t = rewrite("G title Laplace=1/(1+s)\nE1 out 0 in 0 Laplace=1/(1+s*1µ) ; a pole\nG2 a b c d laplace = {gm}/(1+s*{tau})\twindow=1 // as LTspice\nR1 a 0 1k\nE3 laplace_out 0 in 0\n* between\n\n+ Laplace=2\nE4 o 0 i 0 Laplace=3\n+ window=1\nE5 o 0 i 0 Laplace=1/(1+\u{a0}s)\n", &mut w);
        assert!(w.is_empty(), "{w:?}");
        assert!(t.contains("GE1_u 0 e1#lapz1 in 0 1e0\nGE1_a0 0 e1#lapz1 e1#lapz1 0 -1e0\nGE1_b0 0 e1#lapy e1#lapz1 0 1e0\nCE1_z1 e1#lapz1 0 1e-6\nGE1_y e1#lapy 0 e1#lapy 0 1\nE1 out 0 e1#lapy 0 1\n"), "{t}");
        assert!(t.contains("G2 a b g2#lapy 0 1") && t.contains("{gm}") && t.contains("R1 a 0 1k") && !t.contains("pole") && !t.contains("window") && !t.contains("LTspice"), "{t}");
        assert!(t.contains("E3 laplace_out 0 in 0 2e0\nE4 o 0 i 0 3e0\n") && !t.contains("between"), "continuations: {t}");
        assert!(t.contains("E5 o 0 e5#lapy 0 1"), "a no-break space: {t}");
        assert!(t.starts_with("G title Laplace=1/(1+s)\n"), "the title as it was: {t}");
        assert!(rewrite("* t\nE1 o 0 i 0 Laplace=1/(s*1n)\n", &mut w).contains("CE1_z1 e1#lapz1 0 1e-9"), "an integrator scaled at its crossover");
        let t = rewrite("* t\nE1 out 0 Laplace=1/s\n", &mut w);
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
