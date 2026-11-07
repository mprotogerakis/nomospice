//! LTspice's Laplace sources (`E1 out 0 in 0 Laplace=1/(1+s*tau)`, `G…` likewise) for ngspice,
//! which has none: the expression becomes a rational function of s and runs as XSPICE's `s_xfer`
//! (`A… %vd(in) %vd(out)` for E, `%id(out)` for G), in .ac, .tran and the operating point alike.
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
        self.mul(&C::N(-1.0))
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
        self.ws();
        let rest = &self.s[self.i..];
        let n = rest.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(rest.len());
        let k: i32 = rest[..n].parse().map_err(|_| "a power of s must be a whole number".to_string())?;
        self.i += n;
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

/// Whether `s` occurs in `t` as a word of its own.
fn mentions_s(t: &str) -> bool {
    let b = t.as_bytes();
    (0..b.len()).any(|k| {
        b[k].eq_ignore_ascii_case(&b's') && (k == 0 || !(b[k - 1].is_ascii_alphanumeric() || b[k - 1] == b'_')) && (k + 1 == b.len() || !(b[k + 1].is_ascii_alphanumeric() || b[k + 1] == b'_'))
    })
}

/// `expr` (the Laplace= value) as s_xfer's coefficients, by descending power of s: numerator, denominator.
fn coefficients(expr: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut p = P { s: expr.trim(), i: 0 };
    let mut r = p.sum()?;
    if p.peek().is_some() {
        return Err(format!("cannot read `{}`", &p.s[p.i..]));
    }
    if r.0.len() > r.1.len() {
        return Err("more powers of s above than below (an improper transfer function): it cannot run".into());
    }
    // all numbers: scaled so the highest power below has 1 (readable; the same function)
    let num = |p: &Poly| p.iter().map(|c| if let C::N(v) = c { Some(*v) } else { None }).collect::<Option<Vec<f64>>>();
    if let (Some(n), Some(d)) = (num(&r.0), num(&r.1)) {
        let lead = *d.last().unwrap_or(&1.0);
        if lead != 0.0 && lead != 1.0 {
            r = R(n.iter().map(|v| C::N(v / lead)).collect(), d.iter().map(|v| C::N(v / lead)).collect());
        }
    }
    let out = |p: &Poly| p.iter().rev().map(|c| match c { C::N(v) => format!("{v:e}"), C::E(e) => format!("{{{e}}}") }).collect::<Vec<_>>();
    Ok((out(&r.0), out(&r.1)))
}

/// Every E or G line of `text` with `Laplace=` (in subcircuits too) as an s_xfer device and its
/// model; what cannot run is said in `warnings` and left as it was.
pub fn rewrite(text: &str, warnings: &mut Vec<String>) -> String {
    if !crate::schematic::contains_ci(text, "laplace") {
        return text.to_string(); // (most decks: no pass over them)
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let l = line.trim_end_matches(['\n', '\r']);
        match device(l) {
            None => out.push_str(line),
            Some(Err(e)) => {
                warnings.push(format!("`{}`: {e}", l.trim()));
                out.push_str(line);
            }
            Some(Ok(lines)) => {
                out.push_str(&lines);
                out.push('\n');
            }
        }
    }
    out
}

/// A Laplace E/G line as `A…` + `.model …`; None: not one.
fn device(l: &str) -> Option<Result<String, String>> {
    let t = l.trim_start();
    let kind = t.chars().next()?.to_ascii_uppercase();
    if kind != 'E' && kind != 'G' {
        return None;
    }
    let low = t.to_ascii_lowercase();
    let at = low.find("laplace")?;
    let eq = low[at + 7..].trim_start();
    if !eq.starts_with('=') {
        return None;
    }
    let toks: Vec<&str> = t[..at].split_whitespace().collect();
    if toks.len() != 5 {
        return Some(Err("a Laplace source needs its two output and two input nodes (E1 out 0 in 0 Laplace=…)".into()));
    }
    // the expression: up to LTspice's options (window=, nfft=, mtol=), which have no use here
    let value = &t[at + 7 + (low[at + 7..].len() - eq.len()) + 1..];
    let cut = ["window", "nfft", "mtol"].iter().filter_map(|o| value.to_ascii_lowercase().find(&format!(" {o}"))).min().unwrap_or(value.len());
    let (num, den) = match coefficients(&value[..cut]) {
        Ok(c) => c,
        Err(e) => return Some(Err(e)),
    };
    let (name, n) = (toks[0], &toks[1..]);
    let model = format!("{}_laplace", name.to_ascii_lowercase());
    let out = if kind == 'E' { format!("%vd({} {})", n[0], n[1]) } else { format!("%id({} {})", n[0], n[1]) };
    Some(Ok(format!("A{name} %vd({} {}) {out} {model}\n.model {model} s_xfer(gain=1 num_coeff=[{}] den_coeff=[{}])", n[2], n[3], num.join(" "), den.join(" "))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_functions() {
        let c = |e: &str| coefficients(e).unwrap_or_else(|x| panic!("{e}: {x}"));
        assert_eq!(c("1/(1+s*tau)"), (vec!["1e0".into()], vec!["{tau}".into(), "1e0".into()]));
        assert_eq!(c("1/(1+s/1k)"), (vec!["1e3".into()], vec!["1e0".into(), "1e3".into()]));
        // second order: w0²/(s²+s*w0/Q+w0²)
        let (n, d) = c("1e6/(s^2+s*1e3/0.707+1e6)");
        assert_eq!((n.len(), d.len(), d[0].as_str(), d[2].as_str()), (1, 3, "1e0", "1e6"), "{d:?}");
        assert_eq!(c("(s+1)/(s+2)").0, ["1e0", "1e0"]);
        let w = "{(2e0*(pi))*(fc)}".to_string();
        assert_eq!(c("{2*pi*fc}/(s+2*pi*fc)"), (vec![w.clone()], vec!["1e0".to_string(), w]), "parameters stay expressions");
        assert_eq!(c("{1/(1+s*tau)}"), c("1/(1+s*tau)"), "the whole in braces");
        assert!(coefficients("exp(-s*1m)/(1+s)").unwrap_err().contains("delay"));
        assert!(coefficients("s^2/(1+s)").unwrap_err().contains("improper"));
        assert!(!mentions_s("tau") && mentions_s("2*s+1") && !mentions_s("sigma"));
    }

    #[test]
    fn lines() {
        let mut w = vec![];
        let t = rewrite("E1 out 0 in 0 Laplace=1/(1+s*1m)\nG2 a b c d laplace = {gm}/(1+s*{tau}) window=1\nR1 a 0 1k\n", &mut w);
        assert!(w.is_empty(), "{w:?}");
        assert!(t.contains("AE1 %vd(in 0) %vd(out 0) e1_laplace\n.model e1_laplace s_xfer(gain=1 num_coeff=[1e3] den_coeff=[1e0 1e3])"), "{t}");
        assert!(t.contains("AG2 %vd(c d) %id(a b) g2_laplace") && t.contains("num_coeff=[{gm}]") && t.contains("den_coeff=[{tau} 1e0]") && t.contains("R1 a 0 1k"), "{t}");
        let t = rewrite("E1 out 0 Laplace=1/s\n", &mut w);
        assert!(w[0].contains("two output and two input") && t.contains("Laplace"), "left as it was, said");
    }
}
