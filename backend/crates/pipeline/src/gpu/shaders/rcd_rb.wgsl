// RCD step 4.2: red at blue positions and blue at red positions. Writes
// (red, blue) at every red or blue position; green positions get zero.

@group(0) @binding(1) var cfa: texture_2d<f32>;
@group(0) @binding(2) var green: texture_2d<f32>;
@group(0) @binding(3) var pq_dir: texture_2d<f32>;
@group(0) @binding(4) var rb_out: texture_storage_2d<rg32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!in_image(p)) {
        return;
    }
    let color = fc(p);
    if (color == 1u) {
        textureStore(rb_out, p, vec4<f32>(0.0));
        return;
    }

    let pq_c = ld(pq_dir, p);
    let pq_n = 0.25 * (ld(pq_dir, p + vec2(-1, -1)) + ld(pq_dir, p + vec2(1, -1)) + ld(pq_dir, p + vec2(-1, 1)) + ld(pq_dir, p + vec2(1, 1)));
    let pq_disc = select(pq_c, pq_n, abs(0.5 - pq_c) < abs(0.5 - pq_n));

    // The diagonal neighbors carry the other color natively.
    let nw = ld(cfa, p + vec2(-1, -1));
    let ne = ld(cfa, p + vec2(1, -1));
    let sw = ld(cfa, p + vec2(-1, 1));
    let se = ld(cfa, p + vec2(1, 1));
    let g = ld(green, p);

    let nw_grad = EPS + abs(nw - se) + abs(nw - ld(cfa, p + vec2(-3, -3))) + abs(g - ld(green, p + vec2(-2, -2)));
    let ne_grad = EPS + abs(ne - sw) + abs(ne - ld(cfa, p + vec2(3, -3))) + abs(g - ld(green, p + vec2(2, -2)));
    let sw_grad = EPS + abs(ne - sw) + abs(sw - ld(cfa, p + vec2(-3, 3))) + abs(g - ld(green, p + vec2(-2, 2)));
    let se_grad = EPS + abs(nw - se) + abs(se - ld(cfa, p + vec2(3, 3))) + abs(g - ld(green, p + vec2(2, 2)));

    let nw_est = nw - ld(green, p + vec2(-1, -1));
    let ne_est = ne - ld(green, p + vec2(1, -1));
    let sw_est = sw - ld(green, p + vec2(-1, 1));
    let se_est = se - ld(green, p + vec2(1, 1));

    let p_est = (nw_grad * se_est + se_grad * nw_est) / (nw_grad + se_grad);
    let q_est = (ne_grad * sw_est + sw_grad * ne_est) / (ne_grad + sw_grad);
    let other = g + mix(p_est, q_est, clamp(pq_disc, 0.0, 1.0));
    let own = ld(cfa, p);
    let rb = select(vec2<f32>(other, own), vec2<f32>(own, other), color == 0u);
    textureStore(rb_out, p, vec4<f32>(rb, 0.0, 0.0));
}
