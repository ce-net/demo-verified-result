// node tests/wasm-verify.mjs <verify.wasm> <certificate.json>...
// Checks each certificate with the browser build, then shows that changing any one field fails.
import fs from "fs";
const [wasmPath, ...certs] = process.argv.slice(2);
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
const ex = instance.exports;
const call = (fn, o) => {
  const b = new TextEncoder().encode(JSON.stringify(o)), p = ex.buf(b.length);
  new Uint8Array(ex.memory.buffer, p, b.length).set(b);
  const n = ex[fn](b.length);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, ex.out_ptr(), n)));
};
const leaves = (v, p = []) => v && typeof v === "object" ? Object.entries(v).flatMap(([k, x]) => leaves(x, [...p, k])) : [p];
let failed = 0;
for (const f of certs) {
  const cert = JSON.parse(fs.readFileSync(f, "utf8"));
  const r = call("verify", { cert, issuer: cert.issuer, rerun: true });
  if (!r.ok) { failed++; console.log(f, "DOES NOT VERIFY", r.checks.filter(c => !c.ok)); continue; }
  let caught = 0; const ps = leaves(cert);
  for (const p of ps) {
    const t = structuredClone(cert); let o = t;
    for (const k of p.slice(0, -1)) o = o[k];
    const k = p[p.length - 1];
    o[k] = typeof o[k] === "string" ? o[k] + "0" : typeof o[k] === "number" ? o[k] + 1 : null;
    if (!call("verify", { cert: t, issuer: cert.issuer, rerun: false }).ok) caught++;
  }
  if (caught !== ps.length) failed++;
  console.log(`${f}: verifies in wasm (re-run ${r.checks.find(c => c.name === "re-run here").detail}); ${caught}/${ps.length} single-field edits rejected`);
}
process.exit(failed ? 1 : 0);
