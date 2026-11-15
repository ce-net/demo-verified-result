//! cert.verify@1 as a cell: a certificate, or `{"cert":..,"issuer"?:"ed25519:..","rerun"?:bool}`
//! -> the checks. Exit code 1 when any check fails. The browser runs the same code as wasm.
use serde_json::Value;

fn run(req: Value) -> Result<Value, String> {
    let (cert, issuer, rerun) = match req.get("cert") {
        Some(c) => (c.clone(), req.get("issuer").and_then(Value::as_str).map(String::from), req.get("rerun").and_then(Value::as_bool).unwrap_or(true)),
        None => (req, None, true),
    };
    let r = cert_core::verify(&cert, issuer.as_deref(), rerun);
    let v = serde_json::to_value(&r).map_err(|e| e.to_string())?;
    if r.ok {
        Ok(v)
    } else {
        println!("{v}");
        std::process::exit(1)
    }
}

fn main() {
    cells::finish(cells::read_input().and_then(run))
}
