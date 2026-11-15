//! cert.issue@1 and cert.verify@1: the certificate of one physics.fem@1 result.
//!
//! A certificate binds `code_ref` (the solver source, pinned by SHA-256 as a ce-ref), `input_cid`
//! and `output_cid` (ce-blob ids: SHA-256 of the canonical input and of the output blob), one
//! signed claim per device that computed it, and the issuer's signature over every other field.
//! The same code verifies natively and in the browser (crates/verify-wasm).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use fem_core::Case;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The key type cells sign with.
pub type SigningKeyT = SigningKey;

pub const CERT_TYPE: &str = "ce.cert/verified-result@1";
pub const CLAIM_DOMAIN: &str = "ce.cert/node-claim@1\n";
pub const CERT_DOMAIN: &str = "ce.cert/verified-result@1\n";
pub const API: &str = "physics.fem@1";
/// Where the solver lives; the portable ce-ref id is the SHA-256 of this normalized origin.
pub const REPO_ORIGIN: &str = "github.com/ce-net/demo-verified-result";
pub const SOLVER_PATH: &str = "crates/fem-core/src/lib.rs";
/// Relative difference allowed between the FEM tip deflection and the Euler-Bernoulli value.
pub const TOLERANCE: f64 = 0.02;

pub fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// The ce-blob id of some bytes.
pub fn cid(b: &[u8]) -> String {
    format!("ce-blob:sha256:{}", sha256_hex(b))
}

/// The SHA-256 of the solver this binary was built from.
pub fn solver_sha256() -> String {
    sha256_hex(fem_core::SOURCE.as_bytes())
}

/// `code_ref` for the solver at `commit` (may be empty when unknown).
pub fn code_ref(commit: &str) -> Value {
    let repo_id = sha256_hex(REPO_ORIGIN.as_bytes());
    let at = if commit.is_empty() { String::new() } else { format!("@{commit}") };
    json!({
        "api": API,
        "ref": format!("repo:ce-net/demo-verified-result/{SOLVER_PATH}{at} sha256:{}", solver_sha256()),
        "portable": format!("ce-ref://repo/{repo_id}/{SOLVER_PATH}"),
        "sha256": solver_sha256(),
    })
}

/// Canonical JSON: object keys sorted (serde_json's map is ordered), no whitespace.
pub fn canonical(v: &Value) -> String {
    v.to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaseJson {
    pub case: String,
    pub e_mpa: u32,
    pub h_mm: u32,
    pub l_mm: u32,
    pub nu_milli: u32,
    pub nx: u32,
    pub ny: u32,
    pub p_n: u32,
    pub t_mm: u32,
}

impl CaseJson {
    pub fn to_case(&self) -> Result<Case, String> {
        if self.case != "cantilever-q6" {
            return Err(format!("unknown case kind '{}'", self.case));
        }
        let c = Case { l_mm: self.l_mm, h_mm: self.h_mm, t_mm: self.t_mm, e_mpa: self.e_mpa, nu_milli: self.nu_milli, p_n: self.p_n, nx: self.nx, ny: self.ny };
        c.check()?;
        Ok(c)
    }
}

pub fn parse_case(v: &Value) -> Result<Case, String> {
    let cj: CaseJson = serde_json::from_value(v.clone()).map_err(|e| format!("bad input: {e}"))?;
    let c = cj.to_case()?;
    // exactly the canonical form, no extra fields
    let canon: Value = serde_json::from_str(&c.canonical()).map_err(|e| e.to_string())?;
    if &canon != v {
        return Err("input is not in canonical form".into());
    }
    Ok(c)
}

pub fn key_from_seed(seed: &[u8; 32]) -> SigningKey {
    SigningKey::from_bytes(seed)
}

pub fn pub_str(k: &VerifyingKey) -> String {
    format!("ed25519:{}", hex::encode(k.as_bytes()))
}

fn parse_pub(s: &str) -> Result<VerifyingKey, String> {
    let h = s.strip_prefix("ed25519:").ok_or("public key must start with ed25519:")?;
    let b: [u8; 32] = hex::decode(h).map_err(|_| "public key is not hex")?.try_into().map_err(|_| "public key is not 32 bytes")?;
    VerifyingKey::from_bytes(&b).map_err(|_| "not a valid ed25519 point".to_string())
}

fn parse_sig(s: &str) -> Result<Signature, String> {
    let b: [u8; 64] = hex::decode(s).map_err(|_| "signature is not hex")?.try_into().map_err(|_| "signature is not 64 bytes")?;
    Ok(Signature::from_bytes(&b))
}

fn f64_str(x: f64) -> String {
    format!("{x}")
}

/// What one device computed for one input: everything it signs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Computed {
    pub input_cid: String,
    pub output_cid: String,
    pub code_sha256: String,
    pub tip_mm: String,
    pub tip_bits: String,
    pub sigma_mid_mpa: String,
    pub sigma_bits: String,
    pub line_mm: Vec<String>,
}

pub fn compute(c: &Case) -> Result<(Computed, Vec<u8>), String> {
    let s = fem_core::solve(c)?;
    let out = s.bytes();
    let comp = Computed {
        input_cid: cid(c.canonical().as_bytes()),
        output_cid: cid(&out),
        code_sha256: solver_sha256(),
        tip_mm: f64_str(s.tip_mm),
        tip_bits: format!("{:016x}", s.tip_mm.to_bits()),
        sigma_mid_mpa: f64_str(s.sigma_mid_mpa),
        sigma_bits: format!("{:016x}", s.sigma_mid_mpa.to_bits()),
        line_mm: s.line_mm.iter().map(|x| f64_str(*x)).collect(),
    };
    Ok((comp, out))
}

fn claim_body(input_cid: &str, output_cid: &str, code_sha256: &str, device_class: &str, pubk: &str) -> Value {
    json!({"api": API, "code_sha256": code_sha256, "device_class": device_class, "input_cid": input_cid, "output_cid": output_cid, "pub": pubk})
}

/// A device's signed claim: "I ran the solver with this SHA-256 on this input and got this output".
pub fn sign_claim(k: &SigningKey, comp: &Computed, device_class: &str, ms: u64) -> Value {
    let pubk = pub_str(&k.verifying_key());
    let body = claim_body(&comp.input_cid, &comp.output_cid, &comp.code_sha256, device_class, &pubk);
    let sig = k.sign(format!("{CLAIM_DOMAIN}{}", canonical(&body)).as_bytes());
    json!({"device_class": device_class, "pub": pubk, "code_sha256": comp.code_sha256, "ms": ms, "sig": hex::encode(sig.to_bytes())})
}

fn check_claim(n: &Value, input_cid: &str, output_cid: &str) -> Result<String, String> {
    let g = |k: &str| n.get(k).and_then(Value::as_str).ok_or(format!("node claim has no '{k}'"));
    let pubk = g("pub")?;
    let body = claim_body(input_cid, output_cid, g("code_sha256")?, g("device_class")?, pubk);
    parse_pub(pubk)?
        .verify(format!("{CLAIM_DOMAIN}{}", canonical(&body)).as_bytes(), &parse_sig(g("sig")?)?)
        .map_err(|_| format!("signature of {} does not match", g("device_class").unwrap_or("?")))?;
    Ok(pubk.to_string())
}

/// cert.issue@1: check that at least two distinct devices signed the same output for the same
/// input with the pinned solver, then sign the certificate.
pub fn issue(issuer: &SigningKey, input: &Value, comp: &Computed, nodes: &[Value], commit: &str, issued_at: &str) -> Result<Value, String> {
    let c = parse_case(input)?;
    if comp.input_cid != cid(c.canonical().as_bytes()) {
        return Err("input_cid does not match the input".into());
    }
    let mut keys = Vec::new();
    for n in nodes {
        let k = check_claim(n, &comp.input_cid, &comp.output_cid)?;
        if n.get("code_sha256").and_then(Value::as_str) != Some(solver_sha256().as_str()) {
            return Err("a device ran a different solver".into());
        }
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    if keys.len() < 2 {
        return Err(format!("{} distinct device(s) agree; a certificate needs 2", keys.len()));
    }
    let a = fem_core::analytic(&c);
    let tip: f64 = comp.tip_mm.parse().map_err(|_| "bad tip value")?;
    let rel = (tip - a.tip_eb_mm).abs() / a.tip_eb_mm;
    let mut cert = json!({
        "type": CERT_TYPE,
        "code_ref": code_ref(commit),
        "input": input,
        "input_cid": comp.input_cid,
        "output_cid": comp.output_cid,
        "result": {
            "tip_mm": comp.tip_mm, "tip_bits": comp.tip_bits,
            "sigma_mid_mpa": comp.sigma_mid_mpa, "sigma_bits": comp.sigma_bits,
            "line_mm": comp.line_mm,
        },
        "analytic": {
            "method": "Euler-Bernoulli cantilever, tip load: d = P L^3 / (3 E I), sigma = M c / I",
            "tip_eb_mm": f64_str(a.tip_eb_mm),
            "tip_timoshenko_mm": f64_str(a.tip_timoshenko_mm),
            "sigma_mid_mpa": f64_str(a.sigma_mid_mpa),
            "rel_err_tip": f64_str(rel),
            "tolerance": f64_str(TOLERANCE),
        },
        "nodes": nodes,
        "issued_at": issued_at,
        "issuer": pub_str(&issuer.verifying_key()),
    });
    let sig = issuer.sign(format!("{CERT_DOMAIN}{}", canonical(&cert)).as_bytes());
    cert["sig"] = Value::String(hex::encode(sig.to_bytes()));
    Ok(cert)
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub ok: bool,
    pub checks: Vec<Check>,
}

/// cert.verify@1. `expected_issuer` pins the issuer key (`ed25519:<hex>`); `rerun` re-solves the
/// input with this build's solver and compares the output hash.
pub fn verify(cert: &Value, expected_issuer: Option<&str>, rerun: bool) -> Report {
    let mut checks = Vec::new();
    let mut add = |name: &str, r: Result<String, String>| {
        let (ok, detail) = match r {
            Ok(d) => (true, d),
            Err(d) => (false, d),
        };
        checks.push(Check { name: name.into(), ok, detail });
    };
    let s = |k: &str| cert.get(k).and_then(Value::as_str).unwrap_or("").to_string();

    add("type", if s("type") == CERT_TYPE { Ok(CERT_TYPE.into()) } else { Err(format!("type is '{}'", s("type"))) });

    // issuer signature over every field but `sig`
    let issuer = s("issuer");
    add("issuer signature", (|| {
        let mut body = cert.as_object().ok_or("certificate is not an object")?.clone();
        let sig = body.remove("sig").and_then(|v| v.as_str().map(String::from)).ok_or("no sig")?;
        parse_pub(&issuer)?
            .verify(format!("{CERT_DOMAIN}{}", canonical(&Value::Object(body))).as_bytes(), &parse_sig(&sig)?)
            .map_err(|_| "the issuer signature does not match the certificate".to_string())?;
        Ok(format!("valid for {}", short(&issuer)))
    })());
    if let Some(want) = expected_issuer {
        add("issuer is the published key", if issuer == want { Ok(short(want)) } else { Err(format!("issuer {} is not the published {}", short(&issuer), short(want))) });
    }

    let input_cid = s("input_cid");
    let output_cid = s("output_cid");
    let case = cert.get("input").ok_or("no input".to_string()).and_then(parse_case);
    add("input_cid", match &case {
        Ok(c) if cid(c.canonical().as_bytes()) == input_cid => Ok(short(&input_cid)),
        Ok(_) => Err("input_cid is not the hash of the input".into()),
        Err(e) => Err(e.clone()),
    });

    let code = cert.get("code_ref").and_then(|v| v.get("sha256")).and_then(Value::as_str).unwrap_or("");
    add("code_ref", if code == solver_sha256() { Ok(format!("solver sha256 {}", &code[..12.min(code.len())])) } else { Err("code_ref is not the solver this verifier runs".into()) });

    let nodes = cert.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut keys: Vec<String> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let dc = n.get("device_class").and_then(Value::as_str).unwrap_or("?").to_string();
        let r = check_claim(n, &input_cid, &output_cid).and_then(|k| {
            if n.get("code_sha256").and_then(Value::as_str) != Some(code) {
                return Err("signed a different solver hash".into());
            }
            Ok(k)
        });
        if let Ok(k) = &r {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        add(&format!("device {} signature ({dc})", i + 1), r.map(|k| format!("{} signed {}", short(&k), short(&output_cid))));
    }
    add("independent devices", if keys.len() >= 2 { Ok(format!("{} distinct keys agree", keys.len())) } else { Err(format!("{} distinct key(s); 2 needed", keys.len())) });

    if let Ok(c) = &case {
        let a = fem_core::analytic(c);
        let res = cert.get("result").cloned().unwrap_or(Value::Null);
        let an = cert.get("analytic").cloned().unwrap_or(Value::Null);
        add("analytic check", (|| {
            let tip: f64 = res.get("tip_mm").and_then(Value::as_str).ok_or("no tip")?.parse().map_err(|_| "bad tip")?;
            let stated = an.get("tip_eb_mm").and_then(Value::as_str).unwrap_or("");
            if stated != f64_str(a.tip_eb_mm) {
                return Err(format!("stated closed form {stated} is not {}", f64_str(a.tip_eb_mm)));
            }
            let rel = (tip - a.tip_eb_mm).abs() / a.tip_eb_mm;
            if rel <= TOLERANCE {
                Ok(format!("FEM {:.4} mm vs Euler-Bernoulli {:.4} mm: {:.2} % (tolerance {:.0} %)", tip, a.tip_eb_mm, rel * 100.0, TOLERANCE * 100.0))
            } else {
                Err(format!("{:.2} % off the closed form, more than {:.0} %", rel * 100.0, TOLERANCE * 100.0))
            }
        })());
        if rerun {
            add("re-run here", match compute(c) {
                Ok((comp, _)) if comp.output_cid != output_cid => Err(format!("this machine got {}, the certificate says {}", short(&comp.output_cid), short(&output_cid))),
                Ok((comp, _)) => {
                    let same = |k: &str, v: &str| res.get(k).and_then(Value::as_str) == Some(v);
                    if same("tip_mm", &comp.tip_mm) && same("tip_bits", &comp.tip_bits) && same("sigma_mid_mpa", &comp.sigma_mid_mpa) && same("sigma_bits", &comp.sigma_bits) && res.get("line_mm") == Some(&json!(comp.line_mm)) {
                        Ok(format!("same output {} and the same numbers", short(&comp.output_cid)))
                    } else {
                        Err("same output hash but the stated numbers differ".into())
                    }
                }
                Err(e) => Err(e),
            });
        }
    }
    let ok = checks.iter().all(|c| c.ok);
    Report { ok, checks }
}

fn short(s: &str) -> String {
    let t = s.rsplit(':').next().unwrap_or(s);
    format!("{}...", &t[..12.min(t.len())])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Value {
        serde_json::from_str(&Case { l_mm: 1000, h_mm: 100, t_mm: 50, e_mpa: 70_000, nu_milli: 330, p_n: 2000, nx: 40, ny: 4 }.canonical()).unwrap()
    }

    fn cert() -> (Value, String) {
        let c = parse_case(&input()).unwrap();
        let (comp, _) = compute(&c).unwrap();
        let a = key_from_seed(&[1; 32]);
        let b = key_from_seed(&[2; 32]);
        let i = key_from_seed(&[3; 32]);
        let nodes = vec![sign_claim(&a, &comp, "x86_64 linux", 5), sign_claim(&b, &comp, "aarch64 linux", 6)];
        let cert = issue(&i, &input(), &comp, &nodes, "", "2026-11-15T00:00:00Z").unwrap();
        (cert, pub_str(&i.verifying_key()))
    }

    #[test]
    fn verifies() {
        let (c, k) = cert();
        let r = verify(&c, Some(&k), true);
        assert!(r.ok, "{:?}", r.checks);
    }

    #[test]
    fn any_tampered_leaf_fails() {
        let (c, k) = cert();
        let mut paths = Vec::new();
        fn walk(v: &Value, p: Vec<String>, out: &mut Vec<Vec<String>>) {
            match v {
                Value::Object(m) => m.iter().for_each(|(k, x)| walk(x, [p.clone(), vec![k.clone()]].concat(), out)),
                Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| walk(x, [p.clone(), vec![i.to_string()]].concat(), out)),
                _ => out.push(p),
            }
        }
        walk(&c, vec![], &mut paths);
        assert!(paths.len() > 20);
        for p in paths {
            let mut t = c.clone();
            let mut at = &mut t;
            for k in &p {
                at = match at {
                    Value::Array(a) => &mut a[k.parse::<usize>().unwrap()],
                    x => &mut x[k.as_str()],
                };
            }
            *at = match at {
                Value::String(s) => Value::String(format!("{s}0")),
                Value::Number(n) => json!(n.as_u64().unwrap_or(0) + 1),
                _ => Value::Null,
            };
            assert!(!verify(&t, Some(&k), false).ok, "tampering {p:?} still verified");
        }
    }

    #[test]
    fn one_device_is_not_enough() {
        let c = parse_case(&input()).unwrap();
        let (comp, _) = compute(&c).unwrap();
        let a = key_from_seed(&[1; 32]);
        let nodes = vec![sign_claim(&a, &comp, "x", 1), sign_claim(&a, &comp, "x", 1)];
        assert!(issue(&a, &input(), &comp, &nodes, "", "t").is_err());
    }
}

#[cfg(test)]
mod golden {
    use super::*;

    /// The crane-bracket case's output hash, computed on aarch64 Linux, x86_64 Linux and wasm32.
    /// Any platform or compiler that changes one bit of the solution fails here.
    #[test]
    fn crane_bracket_output_is_pinned() {
        let c = Case { l_mm: 2000, h_mm: 200, t_mm: 100, e_mpa: 210_000, nu_milli: 300, p_n: 10_000, nx: 80, ny: 8 };
        let (comp, _) = compute(&c).unwrap();
        assert_eq!(comp.output_cid, "ce-blob:sha256:69c17da8277194dcf500604638f4c6620df19c18e56d03dc39305793d2388019");
    }
}
