//! compute.mapreduce@1 as a cell. `{"map":"physics.fem@1.Solve","inputs":[...],"replicas":2,
//! "pool":[{"name":..,"cmd":[..]}] | "pool_file":path,"deadline_s"?:n}`: every input runs on
//! `replicas` distinct pool members at once; a member that fails is replaced by the next unused
//! one. A member is any command that speaks the cell's one-line contract (a local binary, a remote
//! shell, a mesh call), so the scheduler never knows how a device is reached.
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Member {
    name: String,
    cmd: Vec<String>,
}

fn call(m: &Member, req: &Value, deadline: Instant) -> Result<Value, String> {
    let (prog, args) = m.cmd.split_first().ok_or("empty member command")?;
    let mut child = Command::new(prog).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("{}: {e}", m.name))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    stdin.write_all(format!("{req}\n").as_bytes()).map_err(|e| e.to_string())?;
    drop(stdin);
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    loop {
        if let Some(_status) = child.try_wait().map_err(|e| e.to_string())? {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{}: past the deadline", m.name));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let s = reader.join().map_err(|_| "reader failed")?;
    let line = s.lines().rev().find(|l| !l.trim().is_empty()).ok_or(format!("{}: no answer", m.name))?;
    let v: Value = serde_json::from_str(line).map_err(|e| format!("{}: answer is not JSON: {e}", m.name))?;
    if let Some(e) = v.get("error") {
        return Err(format!("{}: {e}", m.name));
    }
    Ok(v)
}

fn run(req: Value) -> Result<Value, String> {
    let map = req.get("map").and_then(Value::as_str).unwrap_or("physics.fem@1.Solve");
    if map != "physics.fem@1.Solve" {
        return Err(format!("this scheduler maps physics.fem@1.Solve, not {map}"));
    }
    let pool_v = match (req.get("pool"), req.get("pool_file").and_then(Value::as_str)) {
        (Some(p), _) => p.clone(),
        (None, Some(f)) => serde_json::from_str(&std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?).map_err(|e| format!("{f}: {e}"))?,
        _ => return Err("no pool".into()),
    };
    let pool: Vec<Member> = pool_v
        .as_array()
        .ok_or("pool must be a list")?
        .iter()
        .map(|m| Member {
            name: m.get("name").and_then(Value::as_str).unwrap_or("?").into(),
            cmd: m.get("cmd").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        })
        .collect();
    let replicas = req.get("replicas").and_then(Value::as_u64).unwrap_or(2) as usize;
    if replicas == 0 || replicas > pool.len() {
        return Err(format!("{replicas} replicas need at least that many pool members ({} given)", pool.len()));
    }
    let deadline = Instant::now() + Duration::from_secs(req.get("deadline_s").and_then(Value::as_u64).unwrap_or(300));
    let inputs = req.get("inputs").and_then(Value::as_array).ok_or("no inputs")?.clone();
    let t0 = Instant::now();
    let mut runs = Vec::new();
    for (ix, input) in inputs.iter().enumerate() {
        // start each input on a different member so load spreads across the pool
        let order: Vec<Member> = (0..pool.len()).map(|k| pool[(ix + k) % pool.len()].clone()).collect();
        let handles: Vec<_> = (0..replicas)
            .map(|r| {
                let mut tries: Vec<Member> = vec![order[r].clone()];
                tries.extend(order[replicas..].iter().cloned());
                let req = json!({"op": "Solve", "input": input, "blob": r == 0});
                std::thread::spawn(move || {
                    let mut errors = Vec::new();
                    for m in tries {
                        let t = Instant::now();
                        match call(&m, &req, deadline) {
                            Ok(a) => return json!({"member": m.name, "ok": true, "ms": t.elapsed().as_millis() as u64, "answer": a, "errors": errors}),
                            Err(e) => errors.push(e),
                        }
                    }
                    json!({"ok": false, "errors": errors})
                })
            })
            .collect();
        let results: Vec<Value> = handles.into_iter().map(|h| h.join().unwrap_or(json!({"ok": false, "errors": ["thread failed"]}))).collect();
        // a spare member taken by two failing replicas would count twice; keep distinct members only
        let mut seen = Vec::new();
        let results: Vec<Value> = results
            .into_iter()
            .filter(|r| {
                let m = r.get("member").and_then(Value::as_str).unwrap_or("").to_string();
                if m.is_empty() || !seen.contains(&m) {
                    seen.push(m);
                    true
                } else {
                    false
                }
            })
            .collect();
        let cids: Vec<&str> = results.iter().filter_map(|r| r.pointer("/answer/computed/output_cid").and_then(Value::as_str)).collect();
        let agree = cids.len() >= replicas && cids.windows(2).all(|w| w[0] == w[1]);
        runs.push(json!({"input": input, "agree": agree, "replicas": results}));
    }
    Ok(json!({"map": map, "runs": runs, "spent_ms": t0.elapsed().as_millis() as u64}))
}

fn main() {
    cells::finish(cells::read_input().and_then(run))
}
