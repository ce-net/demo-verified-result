//! The browser face of cert.verify@1 and physics.fem@1, built for wasm32-unknown-unknown with no
//! bindings generator and no `unsafe`: JavaScript asks `buf(len)` for an input buffer, writes UTF-8
//! JSON there, calls `verify` or `solve`, then reads the answer (`out_ptr`, returned length).
//!
//! verify: `{"cert": {...}, "issuer": "ed25519:<hex>" | null, "rerun": bool}` -> cert.verify Report.
//! solve:  a canonical physics.fem@1 input -> `{input_cid, output_cid, tip_mm, ...}` or `{error}`.
//! solver: `{"sha256": ..., "code_ref": ...}` of the solver compiled into this module.
use serde_json::{json, Value};
use std::cell::RefCell;

thread_local! {
    static IN: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Resize the input buffer to `len` bytes and return its address in linear memory.
#[no_mangle]
pub extern "C" fn buf(len: usize) -> usize {
    IN.with(|b| {
        let mut b = b.borrow_mut();
        b.clear();
        b.resize(len, 0);
        b.as_mut_ptr() as usize
    })
}

/// Address of the last answer.
#[no_mangle]
pub extern "C" fn out_ptr() -> usize {
    OUT.with(|b| b.borrow().as_ptr() as usize)
}

fn answer(f: impl FnOnce(&[u8]) -> Value) -> usize {
    let v = IN.with(|b| f(&b.borrow()));
    let s = v.to_string().into_bytes();
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        *o = s;
        o.len()
    })
}

pub fn verify_json(input: &[u8]) -> Value {
    let req: Value = match serde_json::from_slice(input) {
        Ok(v) => v,
        Err(e) => return json!({"ok": false, "checks": [{"name": "parse", "ok": false, "detail": format!("not JSON: {e}")}]}),
    };
    let cert = req.get("cert").cloned().unwrap_or(Value::Null);
    let issuer = req.get("issuer").and_then(Value::as_str);
    let rerun = req.get("rerun").and_then(Value::as_bool).unwrap_or(false);
    serde_json::to_value(cert_core::verify(&cert, issuer, rerun)).unwrap_or(Value::Null)
}

pub fn solve_json(input: &[u8]) -> Value {
    let r = serde_json::from_slice::<Value>(input).map_err(|e| e.to_string()).and_then(|v| cert_core::parse_case(&v)).and_then(|c| {
        let a = fem_core::analytic(&c);
        cert_core::compute(&c).map(|(comp, _)| json!({"computed": comp, "tip_eb_mm": a.tip_eb_mm, "sigma_mid_mpa_eb": a.sigma_mid_mpa}))
    });
    r.unwrap_or_else(|e| json!({"error": e}))
}

#[no_mangle]
pub extern "C" fn verify(_len: usize) -> usize {
    answer(verify_json)
}

#[no_mangle]
pub extern "C" fn solve(_len: usize) -> usize {
    answer(solve_json)
}

#[no_mangle]
pub extern "C" fn solver(_len: usize) -> usize {
    answer(|_| json!({"sha256": cert_core::solver_sha256(), "code_ref": cert_core::code_ref("")}))
}
