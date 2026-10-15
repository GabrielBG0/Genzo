// RCD step 3: green at red and blue CFA positions.

@group(0) @binding(1) var cfa: texture_2d<f32>;
@group(0) @binding(2) var lpf: texture_2d<f32>;
@group(0) @binding(3) var vh: texture_2d<f32>;
@group(0) @binding(4) var g_out: texture_storage_2d<r32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_image(p)) {
        return;
    }
    let c = ld(cfa, p);
    if (fc(p) == 1u) {
        textureStore(g_out, p, vec4<f32>(c, 0.0, 0.0, 0.0));
        return;
    }

    let vh_c = ld(vh, p);
    let vh_n = 0.25 * (ld(vh, p + vec2(-1, -1)) + ld(vh, p + vec2(1, -1)) + ld(vh, p + vec2(-1, 1)) + ld(vh, p + vec2(1, 1)));
    let vh_disc = select(vh_c, vh_n, abs(0.5 - vh_c) < abs(0.5 - vh_n));

    let n1 = ld(cfa, p + vec2(0, -1));
    let s1 = ld(cfa, p + vec2(0, 1));
    let w1 = ld(cfa, p + vec2(-1, 0));
    let e1 = ld(cfa, p + vec2(1, 0));
    let n2 = ld(cfa, p + vec2(0, -2));
    let s2 = ld(cfa, p + vec2(0, 2));
    let w2 = ld(cfa, p + vec2(-2, 0));
    let e2 = ld(cfa, p + vec2(2, 0));

    // Cardinal gradients.
    let n_grad = EPS + abs(n1 - s1) + abs(c - n2) + abs(n1 - ld(cfa, p + vec2(0, -3))) + abs(n2 - ld(cfa, p + vec2(0, -4)));
    let s_grad = EPS + abs(s1 - n1) + abs(c - s2) + abs(s1 - ld(cfa, p + vec2(0, 3))) + abs(s2 - ld(cfa, p + vec2(0, 4)));
    let w_grad = EPS + abs(w1 - e1) + abs(c - w2) + abs(w1 - ld(cfa, p + vec2(-3, 0))) + abs(w2 - ld(cfa, p + vec2(-4, 0)));
    let e_grad = EPS + abs(e1 - w1) + abs(c - e2) + abs(e1 - ld(cfa, p + vec2(3, 0))) + abs(e2 - ld(cfa, p + vec2(4, 0)));

    // Cardinal estimations, scaled by the low-pass ratio.
    let l = ld(lpf, p);
    let n_est = n1 * (l + l) / (EPS + l + ld(lpf, p + vec2(0, -2)));
    let s_est = s1 * (l + l) / (EPS + l + ld(lpf, p + vec2(0, 2)));
    let w_est = w1 * (l + l) / (EPS + l + ld(lpf, p + vec2(-2, 0)));
    let e_est = e1 * (l + l) / (EPS + l + ld(lpf, p + vec2(2, 0)));

    let v_est = (s_grad * n_est + n_grad * s_est) / (n_grad + s_grad);
    let h_est = (w_grad * e_est + e_grad * w_est) / (e_grad + w_grad);
    let g = mix(v_est, h_est, clamp(vh_disc, 0.0, 1.0));
    textureStore(g_out, p, vec4<f32>(g, 0.0, 0.0, 0.0));
}
