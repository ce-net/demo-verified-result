//! What every cell binary here shares: one JSON line in on stdin, one JSON line out on stdout, and
//! the device key a cell signs with.
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;

/// Read one JSON value from stdin (one line, or the whole input when it spans lines).
pub fn read_input() -> Result<Value, String> {
    let mut s = String::new();
    let stdin = std::io::stdin();
    let mut h = stdin.lock();
    h.read_line(&mut s).map_err(|e| e.to_string())?;
    if serde_json::from_str::<Value>(&s).is_err() {
        h.read_to_string(&mut s).map_err(|e| e.to_string())?;
    }
    serde_json::from_str(&s).map_err(|e| format!("bad input: {e}"))
}

/// Print the answer as one line; an `Err` becomes `{"error": ...}` and exit code 1.
pub fn finish(r: Result<Value, String>) -> ! {
    let (v, code) = match r {
        Ok(v) => (v, 0),
        Err(e) => (json!({ "error": e }), 1),
    };
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
    std::process::exit(code)
}

fn data_dir(cell: &str) -> PathBuf {
    if let Ok(d) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(d).join(cell);
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share").join(cell)
}

/// The cell's Ed25519 key: `$<ENV>` (a file holding 64 hex chars) or `<data dir>/<cell>/key`,
/// made from the OS random source on first use and kept owner-only.
pub fn key(cell: &str, env: &str) -> Result<cert_core::SigningKeyT, String> {
    let path = std::env::var(env).map(PathBuf::from).unwrap_or_else(|_| data_dir(cell).join("key"));
    if let Ok(s) = std::fs::read_to_string(&path) {
        let b: [u8; 32] = cert_core_hex(s.trim())?;
        return Ok(cert_core::key_from_seed(&b));
    }
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut seed)).map_err(|e| format!("no random source for a new key: {e}"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let hexs: String = seed.iter().map(|b| format!("{b:02x}")).collect();
    write_private(&path, &hexs)?;
    Ok(cert_core::key_from_seed(&seed))
}

#[cfg(unix)]
fn write_private(p: &std::path::Path, s: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(p).map_err(|e| e.to_string())?;
    f.write_all(s.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn write_private(p: &std::path::Path, s: &str) -> Result<(), String> {
    std::fs::write(p, s).map_err(|e| e.to_string())
}

fn cert_core_hex(s: &str) -> Result<[u8; 32], String> {
    if s.len() != 64 {
        return Err("key file must hold 64 hex characters".into());
    }
    let mut b = [0u8; 32];
    for i in 0..32 {
        b[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| "key file is not hex")?;
    }
    Ok(b)
}

/// A device class people can read: `$PHYSICS_FEM_DEVICE_CLASS`, else the CPU architecture and OS.
pub fn device_class() -> String {
    std::env::var("PHYSICS_FEM_DEVICE_CLASS").unwrap_or_else(|_| format!("{} {}", std::env::consts::ARCH, std::env::consts::OS))
}
