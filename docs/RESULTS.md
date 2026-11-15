# Results, 2026-11-15

Each case ran on two devices through `compute.mapreduce@1`: an ARM server (aarch64 Linux) and an
x86_64 desktop (x86_64 Linux). The output hash is the same on both, and the wasm build in the browser
(Chromium, headless) reproduces it. Stress is the axial stress at the top fiber at mid-span.

| Load case | Mesh | FEM tip | Euler-Bernoulli tip | Difference | Stress FEM / M c / I | Output hash (first 16) |
|---|---|---|---|---|---|---|
| Crane runway bracket | 80 x 8 | 1.9153 mm | 1.9048 mm | 0.55 % | 15.00 / 15.00 MPa | `69c17da8277194dc` |
| Conveyor side rail | 100 x 8 | 4.1978 mm | 4.1853 mm | 0.30 % | 11.72 / 11.72 MPa | `1f54228d8d720894` |
| Robot gripper finger | 60 x 6 | 0.5743 mm | 0.5714 mm | 0.50 % | 10.00 / 10.00 MPa | `91cb0b7c8e9039b2` |
| Parts shelf arm | 40 x 4 | 3.2457 mm | 3.2323 mm | 0.41 % | 4.44 / 4.44 MPa | `04524592c2af00e8` |

Browser check (headless Chromium, 1440 x 1500 and 390 x 844): every certificate verifies, including
the WebAssembly re-run (about 25 ms). Edits made in the page's text box:

| Edit | Checks that fail |
|---|---|
| input load 10000 -> 10001 N | issuer signature, input_cid, analytic check, re-run |
| FEM tip value, last digit | issuer signature, re-run |
| a device's class name | issuer signature, that device's signature, independent devices |

`node tests/wasm-verify.mjs web/verify.wasm web/certs/*.json` edits every leaf of every certificate
one at a time: 120/120, 140/140, 100/100 and 80/80 edits rejected.

The input and output blobs of every case are in the issuing node's ce-blob store under the ids above.
