//! physics.fem@1: a small deterministic 2D plane-stress solver for a cantilever beam.
//!
//! Determinism contract (the reason this crate exists): every result is a pure function of the
//! integer input, computed in IEEE-754 binary64 with only `+ - * /` and `sqrt` (all correctly
//! rounded by the standard), in one fixed loop order, with no fused multiply-add, no threads, no
//! hash-map iteration and no libm transcendentals. So x86_64, aarch64 and wasm32 builds of this one
//! file produce the same bits, and the output hash is the same on every machine.
//!
//! The certificate pins this file by its SHA-256 (`SOURCE` below is the file itself).
//!
//! Element: 4-node rectangle with Wilson-Taylor incompatible bending modes (Q6), condensed to 8 dofs.
//! Mesh: `nx` x `ny` equal rectangles over `l_mm` x `h_mm`, thickness `t_mm`. Left edge clamped,
//! tip load `p_n` (downward) on the right edge as a uniform shear traction. Solve: banded Cholesky.

/// This source file, byte for byte. Its SHA-256 is the certificate's `code_ref.sha256`.
pub const SOURCE: &str = include_str!("lib.rs");

/// Output format tag, first bytes of every output blob.
pub const OUT_MAGIC: &[u8; 8] = b"FEMOUT1\0";

/// Largest mesh accepted (elements). Keeps a single call under a second on a small CPU.
pub const MAX_ELEMENTS: u32 = 64_000;

/// One load case. Integers only, so the input has exactly one canonical encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    /// Span from the clamped edge to the loaded tip, mm.
    pub l_mm: u32,
    /// Section depth, mm.
    pub h_mm: u32,
    /// Section width (plane-stress thickness), mm.
    pub t_mm: u32,
    /// Young's modulus, MPa (N/mm^2).
    pub e_mpa: u32,
    /// Poisson's ratio times 1000.
    pub nu_milli: u32,
    /// Total tip load, N, pointing down.
    pub p_n: u32,
    /// Elements along the span (even).
    pub nx: u32,
    /// Elements through the depth (even).
    pub ny: u32,
}

impl Case {
    pub fn check(&self) -> Result<(), String> {
        let pos = [("l_mm", self.l_mm), ("h_mm", self.h_mm), ("t_mm", self.t_mm), ("e_mpa", self.e_mpa), ("p_n", self.p_n)];
        for (k, v) in pos {
            if v == 0 {
                return Err(format!("{k} must be > 0"));
            }
        }
        if self.nu_milli >= 500 {
            return Err("nu_milli must be < 500".into());
        }
        if self.nx < 2 || self.ny < 2 || self.nx % 2 != 0 || self.ny % 2 != 0 {
            return Err("nx and ny must be even and >= 2".into());
        }
        if (self.nx as u64) * (self.ny as u64) > MAX_ELEMENTS as u64 {
            return Err(format!("mesh larger than {MAX_ELEMENTS} elements"));
        }
        Ok(())
    }

    /// The canonical input: keys sorted, no spaces, integers only. Its SHA-256 is `input_cid`.
    pub fn canonical(&self) -> String {
        format!(
            "{{\"case\":\"cantilever-q6\",\"e_mpa\":{},\"h_mm\":{},\"l_mm\":{},\"nu_milli\":{},\"nx\":{},\"ny\":{},\"p_n\":{},\"t_mm\":{}}}",
            self.e_mpa, self.h_mm, self.l_mm, self.nu_milli, self.nx, self.ny, self.p_n, self.t_mm
        )
    }
}

/// Closed-form beam theory for the same case.
#[derive(Clone, Debug, PartialEq)]
pub struct Analytic {
    /// Euler-Bernoulli tip deflection P L^3 / (3 E I), mm (positive down).
    pub tip_eb_mm: f64,
    /// Timoshenko tip deflection: Euler-Bernoulli plus the shear term P L / (k G A), k = 5/6, mm.
    pub tip_timoshenko_mm: f64,
    /// Bending stress at the top fiber at mid-span, M c / I with M = P L / 2, MPa.
    pub sigma_mid_mpa: f64,
}

pub fn analytic(c: &Case) -> Analytic {
    let (l, h, t, e, nu, p) = (c.l_mm as f64, c.h_mm as f64, c.t_mm as f64, c.e_mpa as f64, c.nu_milli as f64 / 1000.0, c.p_n as f64);
    let i = t * h * h * h / 12.0;
    let tip_eb = p * l * l * l / (3.0 * e * i);
    let g = e / (2.0 * (1.0 + nu));
    let shear = p * l / ((5.0 / 6.0) * g * (t * h));
    Analytic { tip_eb_mm: tip_eb, tip_timoshenko_mm: tip_eb + shear, sigma_mid_mpa: p * (l / 2.0) * (h / 2.0) / i }
}

/// The FEM result. `bytes()` is the output blob; its SHA-256 is `output_cid`.
#[derive(Clone, Debug, PartialEq)]
pub struct Solution {
    /// Every nodal displacement, node (i, j) at index 2 * (i * (ny + 1) + j) (+1 for v), mm.
    pub u: Vec<f64>,
    /// Downward deflection of the mid-height node of the tip, mm.
    pub tip_mm: f64,
    /// Axial stress at the top fiber at mid-span (average of the two adjacent elements), MPa.
    pub sigma_mid_mpa: f64,
    /// Downward deflection of the mid-height line, nx + 1 values from root to tip, mm.
    pub line_mm: Vec<f64>,
}

impl Solution {
    pub fn bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(8 + 4 + 8 * (self.u.len() + 2));
        b.extend_from_slice(OUT_MAGIC);
        b.extend_from_slice(&(self.u.len() as u32).to_le_bytes());
        for x in &self.u {
            b.extend_from_slice(&x.to_le_bytes());
        }
        b.extend_from_slice(&self.tip_mm.to_le_bytes());
        b.extend_from_slice(&self.sigma_mid_mpa.to_le_bytes());
        b
    }
}

// Node local coordinates, counter-clockwise from (-1, -1).
const XI: [f64; 4] = [-1.0, 1.0, 1.0, -1.0];
const ETA: [f64; 4] = [-1.0, -1.0, 1.0, 1.0];

struct Element {
    /// Condensed 8x8 stiffness, row-major.
    k: [[f64; 8]; 8],
    /// Kii^-1 Kic (4x8): internal modes are -r u_e.
    r: [[f64; 8]; 4],
    d: [[f64; 3]; 3],
    dx: f64,
    dy: f64,
}

/// Strain-displacement matrix (3 x 12) at (xi, eta) for a dx x dy rectangle: 8 nodal dofs, then
/// the four incompatible modes (u: 1-xi^2, u: 1-eta^2, v: 1-xi^2, v: 1-eta^2).
fn b_matrix(xi: f64, eta: f64, dx: f64, dy: f64) -> [[f64; 12]; 3] {
    let mut b = [[0.0; 12]; 3];
    for n in 0..4 {
        let dndx = XI[n] * (1.0 + ETA[n] * eta) / 4.0 * (2.0 / dx);
        let dndy = ETA[n] * (1.0 + XI[n] * xi) / 4.0 * (2.0 / dy);
        b[0][2 * n] = dndx;
        b[1][2 * n + 1] = dndy;
        b[2][2 * n] = dndy;
        b[2][2 * n + 1] = dndx;
    }
    let dp1dx = -2.0 * xi * (2.0 / dx);
    let dp2dy = -2.0 * eta * (2.0 / dy);
    b[0][8] = dp1dx;
    b[2][9] = dp2dy;
    b[2][10] = dp1dx;
    b[1][11] = dp2dy;
    b
}

fn element(c: &Case) -> Result<Element, String> {
    let (e, nu, t) = (c.e_mpa as f64, c.nu_milli as f64 / 1000.0, c.t_mm as f64);
    let dx = c.l_mm as f64 / c.nx as f64;
    let dy = c.h_mm as f64 / c.ny as f64;
    let f = e / (1.0 - nu * nu);
    let d = [[f, f * nu, 0.0], [f * nu, f, 0.0], [0.0, 0.0, f * (1.0 - nu) / 2.0]];
    let g = 1.0 / 3.0f64.sqrt();
    let det_j = dx * dy / 4.0;
    let mut k12 = [[0.0f64; 12]; 12];
    for &(xi, eta) in &[(-g, -g), (g, -g), (g, g), (-g, g)] {
        let b = b_matrix(xi, eta, dx, dy);
        let mut db = [[0.0f64; 12]; 3];
        for r in 0..3 {
            for col in 0..12 {
                let mut s = 0.0;
                for m in 0..3 {
                    s += d[r][m] * b[m][col];
                }
                db[r][col] = s;
            }
        }
        for a in 0..12 {
            for col in 0..12 {
                let mut s = 0.0;
                for m in 0..3 {
                    s += b[m][a] * db[m][col];
                }
                k12[a][col] += s * det_j * t;
            }
        }
    }
    // Kii^-1 by Gauss-Jordan on [Kii | I] in fixed order (Kii is SPD, no pivoting needed).
    let mut aug = [[0.0f64; 8]; 4];
    for i in 0..4 {
        for j in 0..4 {
            aug[i][j] = k12[8 + i][8 + j];
        }
        aug[i][4 + i] = 1.0;
    }
    for p in 0..4 {
        let piv = aug[p][p];
        if !(piv > 0.0) {
            return Err("singular internal block".into());
        }
        for j in 0..8 {
            aug[p][j] /= piv;
        }
        for i in 0..4 {
            if i != p {
                let m = aug[i][p];
                for j in 0..8 {
                    aug[i][j] -= m * aug[p][j];
                }
            }
        }
    }
    let mut r = [[0.0f64; 8]; 4];
    for i in 0..4 {
        for j in 0..8 {
            let mut s = 0.0;
            for m in 0..4 {
                s += aug[i][4 + m] * k12[8 + m][j];
            }
            r[i][j] = s;
        }
    }
    let mut k = [[0.0f64; 8]; 8];
    for i in 0..8 {
        for j in 0..8 {
            let mut s = 0.0;
            for m in 0..4 {
                s += k12[i][8 + m] * r[m][j];
            }
            k[i][j] = k12[i][j] - s;
        }
    }
    Ok(Element { k, r, d, dx, dy })
}

/// Lower band of a symmetric matrix: entry (i, j), j <= i, i - j <= hb, at i * (hb + 1) + (i - j).
struct Band {
    n: usize,
    hb: usize,
    a: Vec<f64>,
}

impl Band {
    fn at(&self, i: usize, j: usize) -> usize {
        i * (self.hb + 1) + (i - j)
    }

    /// In-place Cholesky factor L (A = L L^T), then solve for `b`, fixed loop order.
    fn solve(mut self, mut b: Vec<f64>) -> Result<Vec<f64>, String> {
        let (n, hb) = (self.n, self.hb);
        for i in 0..n {
            let lo = i.saturating_sub(hb);
            for j in lo..=i {
                let mut s = self.a[self.at(i, j)];
                for k in lo.max(j.saturating_sub(hb))..j {
                    s -= self.a[self.at(i, k)] * self.a[self.at(j, k)];
                }
                if i == j {
                    if !(s > 0.0) {
                        return Err(format!("matrix not positive definite at dof {i}"));
                    }
                    let ii = self.at(i, i);
                    self.a[ii] = s.sqrt();
                } else {
                    let ij = self.at(i, j);
                    self.a[ij] = s / self.a[self.at(j, j)];
                }
            }
        }
        for i in 0..n {
            let mut s = b[i];
            for k in i.saturating_sub(hb)..i {
                s -= self.a[self.at(i, k)] * b[k];
            }
            b[i] = s / self.a[self.at(i, i)];
        }
        for i in (0..n).rev() {
            let mut s = b[i];
            for k in (i + 1)..n.min(i + hb + 1) {
                s -= self.a[self.at(k, i)] * b[k];
            }
            b[i] = s / self.a[self.at(i, i)];
        }
        Ok(b)
    }
}

fn node(c: &Case, i: u32, j: u32) -> usize {
    (i * (c.ny + 1) + j) as usize
}

fn element_dofs(c: &Case, i: u32, j: u32) -> [usize; 8] {
    let ns = [node(c, i, j), node(c, i + 1, j), node(c, i + 1, j + 1), node(c, i, j + 1)];
    let mut d = [0usize; 8];
    for (m, n) in ns.iter().enumerate() {
        d[2 * m] = 2 * n;
        d[2 * m + 1] = 2 * n + 1;
    }
    d
}

/// Solve one case. Same input, same bits, on every platform.
pub fn solve(c: &Case) -> Result<Solution, String> {
    c.check()?;
    let el = element(c)?;
    let nn = ((c.nx + 1) * (c.ny + 1)) as usize;
    let n = 2 * nn;
    let hb = 2 * (c.ny as usize + 2) + 1;
    let mut band = Band { n, hb, a: vec![0.0; n * (hb + 1)] };
    // clamped: every dof of the nodes at x = 0
    let fixed = |dof: usize| dof / 2 <= c.ny as usize;
    for i in 0..c.nx {
        for j in 0..c.ny {
            let dofs = element_dofs(c, i, j);
            for a in 0..8 {
                for b in 0..8 {
                    let (ra, cb) = (dofs[a], dofs[b]);
                    if cb > ra || fixed(ra) || fixed(cb) {
                        continue;
                    }
                    let idx = band.at(ra, cb);
                    band.a[idx] += el.k[a][b];
                }
            }
        }
    }
    for dof in 0..n {
        if fixed(dof) {
            let idx = band.at(dof, dof);
            band.a[idx] = 1.0;
        }
    }
    let mut f = vec![0.0f64; n];
    let p = c.p_n as f64;
    let ny = c.ny as f64;
    for j in 0..=c.ny {
        let w = if j == 0 || j == c.ny { p / (2.0 * ny) } else { p / ny };
        f[2 * node(c, c.nx, j) + 1] = -w;
    }
    let u = band.solve(f)?;

    let tip_mm = -u[2 * node(c, c.nx, c.ny / 2) + 1];
    let line_mm = (0..=c.nx).map(|i| -u[2 * node(c, i, c.ny / 2) + 1]).collect();
    let sx = |ei: u32, xi: f64| -> f64 {
        let dofs = element_dofs(c, ei, c.ny - 1);
        let mut q = [0.0f64; 12];
        for m in 0..8 {
            q[m] = u[dofs[m]];
        }
        for a in 0..4 {
            let mut s = 0.0;
            for m in 0..8 {
                s += el.r[a][m] * q[m];
            }
            q[8 + a] = -s;
        }
        let b = b_matrix(xi, 1.0, el.dx, el.dy);
        let mut eps = [0.0f64; 3];
        for r in 0..3 {
            let mut s = 0.0;
            for m in 0..12 {
                s += b[r][m] * q[m];
            }
            eps[r] = s;
        }
        el.d[0][0] * eps[0] + el.d[0][1] * eps[1] + el.d[0][2] * eps[2]
    };
    let mid = c.nx / 2;
    let sigma_mid_mpa = (sx(mid - 1, 1.0) + sx(mid, -1.0)) / 2.0;
    Ok(Solution { u, tip_mm, sigma_mid_mpa, line_mm })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(nx: u32, ny: u32) -> Case {
        Case { l_mm: 2000, h_mm: 200, t_mm: 100, e_mpa: 210_000, nu_milli: 300, p_n: 10_000, nx, ny }
    }

    #[test]
    fn matches_beam_theory() {
        let c = case(80, 8);
        let s = solve(&c).unwrap();
        let a = analytic(&c);
        let e_tip = (s.tip_mm - a.tip_eb_mm).abs() / a.tip_eb_mm;
        let e_sig = (s.sigma_mid_mpa - a.sigma_mid_mpa).abs() / a.sigma_mid_mpa;
        assert!(e_tip < 0.02, "tip {} vs {} ({e_tip})", s.tip_mm, a.tip_eb_mm);
        assert!(e_sig < 0.02, "sigma {} vs {} ({e_sig})", s.sigma_mid_mpa, a.sigma_mid_mpa);
    }

    #[test]
    fn same_input_same_bits() {
        let c = case(20, 4);
        assert_eq!(solve(&c).unwrap().bytes(), solve(&c).unwrap().bytes());
    }

    #[test]
    fn refuses_bad_input() {
        assert!(solve(&case(3, 4)).is_err());
        assert!(solve(&Case { p_n: 0, ..case(4, 4) }).is_err());
    }
}
