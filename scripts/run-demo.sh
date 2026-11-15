#!/usr/bin/env bash
# Run every load case in cases/cases.json on two devices through compute.mapreduce@1, have
# cert.issue@1 sign each agreed result, check each certificate with cert.verify@1, and write the
# page's data (web/certs/*.json, web/keys.json).
#   POOL=<pool.json> scripts/run-demo.sh        (default pool: pool/example.json)
#   BLOBS=node      also store inputs and outputs in the local ce node's blob store (needs CE_API_TOKEN)
set -euo pipefail
cd "$(dirname "$0")/.."
POOL=${POOL:-pool/example.json}
BIN=${BIN:-}
mr="${BIN}compute-mapreduce"; issue="${BIN}cert-issue"; verify="${BIN}cert-verify"
commit=$(git log -1 --format=%H -- crates/fem-core/src/lib.rs 2>/dev/null || true)
mkdir -p web/certs
inputs=$(python3 -c 'import json;print(json.dumps([c["input"] for c in json.load(open("cases/cases.json"))]))')
echo "{\"map\":\"physics.fem@1.Solve\",\"inputs\":$inputs,\"replicas\":2,\"pool_file\":\"$POOL\"}" | $mr > web/certs/.mapreduce.json
python3 - "$commit" "${BLOBS:-}" <<'PY'
import json, os, subprocess, sys
commit, blobs = sys.argv[1], sys.argv[2]
cases = json.load(open("cases/cases.json"))
mr = json.load(open("web/certs/.mapreduce.json"))
if "error" in mr: sys.exit("compute.mapreduce: " + mr["error"])
index, issuer = [], None
for case, run in zip(cases, mr["runs"]):
    for r in run["replicas"]:
        c = r.get("answer", {}).get("computed", {})
        print(f'{case["id"]:15} {r.get("answer",{}).get("claim",{}).get("device_class","?"):34} {c.get("output_cid","-")} tip {c.get("tip_mm","-")} mm  {r.get("ms","-")} ms', file=sys.stderr)
    req = {"run": run, "commit": commit}
    if blobs: req["blobs"] = blobs
    out = json.loads(subprocess.run([os.environ.get("BIN", "") + "cert-issue"], input=json.dumps(req), capture_output=True, text=True).stdout)
    if "error" in out: sys.exit(f'cert.issue {case["id"]}: {out["error"]}')
    cert = out["cert"]
    v = subprocess.run([os.environ.get("BIN", "") + "cert-verify"], input=json.dumps({"cert": cert, "issuer": cert["issuer"], "rerun": True}), capture_output=True, text=True)
    if v.returncode != 0: sys.exit(f'cert.verify {case["id"]}: {v.stdout}')
    json.dump(cert, open(f'web/certs/{case["id"]}.json', "w"), indent=1)
    index.append({k: case[k] for k in ("id", "title", "sub")})
    issuer = cert["issuer"]
    print(f'{case["id"]:15} certificate verified, {len(cert["nodes"])} devices', file=sys.stderr)
json.dump(index, open("web/certs/index.json", "w"), indent=1)
json.dump({"issuer": issuer, "note": "The cert.issue@1 key this page trusts."}, open("web/keys.json", "w"), indent=1)
PY
