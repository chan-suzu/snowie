//! The snowflake icon, drawn in code.

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (qx, qy) = (ax + dx * t - px, ay + dy * t - py);
    (qx * qx + qy * qy).sqrt()
}

/// Snowflake picture, straight (non-premultiplied) RGBA, n×n.
pub fn snowflake_rgba(n: usize) -> Vec<u8> {
    let k = n as f32 / 32.0;
    let c = n as f32 / 2.0 - 0.5;
    let mut segs = Vec::new();
    for i in 0..6 {
        let a = i as f32 * std::f32::consts::PI / 3.0 - std::f32::consts::FRAC_PI_2;
        let (ca, sa) = (a.cos(), a.sin());
        let p = |d: f32| (c + ca * d * k, c + sa * d * k);
        let (ex, ey) = p(14.0);
        segs.push((c, c, ex, ey));
        for (d, l) in [(6.5f32, 4.5f32), (10.5, 3.5)] {
            let (bx, by) = p(d);
            for side in [-1.0f32, 1.0] {
                let b = a + side * 0.75;
                segs.push((bx, by, bx + b.cos() * l * k, by + b.sin() * l * k));
            }
        }
    }
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = segs.iter().map(|s| seg_dist(px, py, s.0, s.1, s.2, s.3)).fold(f32::MAX, f32::min) / k;
            let core = (1.6 - d).clamp(0.0, 1.0); // light-blue stroke
            let halo = (2.6 - d).clamp(0.0, 1.0); // darker outline for light taskbars
            let a = halo.max(core);
            let i = (y * n + x) * 4;
            out[i] = (40.0 + 190.0 * core) as u8;
            out[i + 1] = (90.0 + 150.0 * core) as u8;
            out[i + 2] = (170.0 + 85.0 * core) as u8;
            out[i + 3] = (a * 255.0) as u8;
        }
    }
    out
}
