//! physics.fem@1 as a cell: `{"op":"Solve","input":<canonical case>,"blob"?:bool}` -> the result,
//! this device's signed claim over it and (with `blob`) the output blob as hex.
//! `{"op":"Health"}` -> `{ok, version, solver_sha256, device_class}`.
use serde_json::{json, Value};
use std::time::Instant;

fn run(req: Value) -> Result<Value, String> {
    match req.get("op").and_then(Value::as_str).unwrap_or("Solve") {
        "Health" => Ok(json!({"ok": true, "version": env!("CARGO_PKG_VERSION"), "solver_sha256": cert_core::solver_sha256(), "device_class": cells::device_class()})),
        "Solve" => {
            let input = req.get("input").ok_or("no input")?;
            let case = cert_core::parse_case(input)?;
            let t = Instant::now();
            let (comp, out) = cert_core::compute(&case)?;
            let ms = t.elapsed().as_millis() as u64;
            let key = cells::key("physics-fem", "PHYSICS_FEM_KEY")?;
            let claim = cert_core::sign_claim(&key, &comp, &cells::device_class(), ms);
            let mut a = json!({"computed": comp, "claim": claim});
            if req.get("blob").and_then(Value::as_bool).unwrap_or(false) {
                a["output_hex"] = Value::String(out.iter().map(|b| format!("{b:02x}")).collect());
            }
            Ok(a)
        }
        op => Err(format!("unknown op '{op}' (Solve, Health)")),
    }
}

fn main() {
    cells::finish(cells::read_input().and_then(run))
}
