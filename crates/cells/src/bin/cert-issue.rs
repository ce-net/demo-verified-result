//! cert.issue@1 as a cell: `{"run": <one compute.mapreduce@1 run>, "commit"?: "<git sha>",
//! "blobs"?: "node" | "dir:<path>"}` -> a signed certificate. Refuses unless at least two distinct
//! devices signed the same output for the input with the pinned solver. With `blobs` it stores the
//! input and output in ce-blob (the local ce node's `/blobs`, or a directory named by hash).
use serde_json::{json, Value};
use std::io::{Read, Write};

fn put_node(data: &[u8]) -> Result<String, String> {
    let url = std::env::var("CE_NODE_URL").unwrap_or_else(|_| "http://127.0.0.1:8844".into());
    let addr = url.strip_prefix("http://").ok_or("CE_NODE_URL must be http://host:port")?.trim_end_matches('/').to_string();
    let token = std::env::var("CE_API_TOKEN").map_err(|_| "CE_API_TOKEN not set")?;
    let mut s = std::net::TcpStream::connect(&addr).map_err(|e| format!("{addr}: {e}"))?;
    write!(s, "POST /blobs HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", token.trim(), data.len()).map_err(|e| e.to_string())?;
    s.write_all(data).map_err(|e| e.to_string())?;
    let mut r = String::new();
    s.read_to_string(&mut r).map_err(|e| e.to_string())?;
    let body = r.split("\r\n\r\n").nth(1).unwrap_or("");
    let v: Value = serde_json::from_str(body.trim()).map_err(|_| format!("blob store answered: {}", r.lines().next().unwrap_or("")))?;
    v.get("hash").and_then(Value::as_str).map(String::from).ok_or("no hash in the blob store's answer".into())
}

fn store(where_: &str, data: &[u8], want: &str) -> Result<(), String> {
    let h = cert_core::sha256_hex(data);
    if format!("ce-blob:sha256:{h}") != want {
        return Err("blob does not hash to its cid".into());
    }
    if where_ == "node" {
        let got = put_node(data)?;
        if got != h {
            return Err(format!("blob store hashed {got}, expected {h}"));
        }
    } else if let Some(dir) = where_.strip_prefix("dir:") {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        std::fs::write(format!("{dir}/{h}"), data).map_err(|e| e.to_string())?;
    } else {
        return Err(format!("unknown blobs target '{where_}'"));
    }
    Ok(())
}

fn now_rfc3339() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    // civil-from-days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn run(req: Value) -> Result<Value, String> {
    let run = req.get("run").ok_or("no run")?;
    let input = run.get("input").ok_or("run has no input")?;
    let reps: Vec<&Value> = run.get("replicas").and_then(Value::as_array).ok_or("run has no replicas")?.iter().filter(|r| r.get("ok").and_then(Value::as_bool) == Some(true)).collect();
    let first = reps.first().ok_or("no replica succeeded")?;
    let comp: cert_core::Computed = serde_json::from_value(first.pointer("/answer/computed").cloned().ok_or("replica has no result")?).map_err(|e| e.to_string())?;
    for r in &reps {
        let other = r.pointer("/answer/computed/output_cid").and_then(Value::as_str).unwrap_or("");
        if other != comp.output_cid {
            return Err(format!("devices disagree: {} vs {other}", comp.output_cid));
        }
    }
    let nodes: Vec<Value> = reps.iter().filter_map(|r| r.pointer("/answer/claim").cloned()).collect();
    let key = cells::key("cert-issue", "CERT_ISSUE_KEY")?;
    let commit = req.get("commit").and_then(Value::as_str).unwrap_or("");
    let cert = cert_core::issue(&key, input, &comp, &nodes, commit, &now_rfc3339())?;
    if let Some(b) = req.get("blobs").and_then(Value::as_str) {
        store(b, cert_core::parse_case(input)?.canonical().as_bytes(), &comp.input_cid)?;
        let hexs = reps.iter().find_map(|r| r.pointer("/answer/output_hex").and_then(Value::as_str)).ok_or("no replica returned the output blob")?;
        let out: Vec<u8> = (0..hexs.len() / 2).map(|i| u8::from_str_radix(&hexs[2 * i..2 * i + 2], 16)).collect::<Result<_, _>>().map_err(|_| "output blob is not hex")?;
        store(b, &out, &comp.output_cid)?;
    }
    Ok(json!({"cert": cert}))
}

fn main() {
    cells::finish(cells::read_input().and_then(run))
}
