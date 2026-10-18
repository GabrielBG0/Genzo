//! DNG opcodes the pipeline applies, parsed into typed parameters, and the
//! decision whether a file's opcode lists can be honored (DEV-01, stage 0).
//! The decoder applies none of them: OpcodeList1 and 2 run in stage 1 and
//! OpcodeList3 after the demosaic; `genzo_pipeline` holds the CPU reference.
//!
//! The parameter layouts and their meaning follow the DNG specification as
//! the Adobe DNG SDK implements it (`dng_gain_map.cpp`,
//! `dng_bad_pixels.cpp`, `dng_misc_opcodes.cpp`); see the functions in
//! `genzo_pipeline::reference::opcodes` for how they are applied.

use crate::raw::{Opcode, OpcodeKind, OpcodeLists};

/// Which pixels an opcode touches (DNG `dng_area_spec`): a rectangle,
/// a range of planes, and a row and column pitch from the rectangle's top
/// left. An empty rectangle covers the whole image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AreaSpec {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
    pub plane: u32,
    pub planes: u32,
    pub row_pitch: u32,
    pub col_pitch: u32,
}

/// GainMap (opcode 9): a grid of gains, interpolated bilinearly at each
/// pixel's relative position in the image and multiplied in.
#[derive(Debug, Clone, PartialEq)]
pub struct GainMap {
    pub area: AreaSpec,
    pub points_v: u32,
    pub points_h: u32,
    /// Between map points, in relative image coordinates (0 to 1).
    pub spacing_v: f64,
    pub spacing_h: f64,
    /// The first map point, in relative image coordinates.
    pub origin_v: f64,
    pub origin_h: f64,
    pub map_planes: u32,
    /// Row by row, each point's gains for every map plane.
    pub gains: Vec<f32>,
}

impl GainMap {
    /// The gain at map point (`v`, `h`) for map plane `plane`.
    pub fn entry(&self, v: u32, h: u32, plane: u32) -> f32 {
        let i = (v as usize * self.points_h as usize + h as usize) * self.map_planes as usize
            + plane as usize;
        self.gains[i]
    }
}

/// FixBadPixelsConstant (opcode 4): on the stored values of a Bayer
/// image, every sample equal to `constant` is replaced by the mean of its
/// nearest same-color neighbors. `bayer_phase` places the pattern's
/// greens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixBadPixelsConstant {
    pub constant: u32,
    pub bayer_phase: u32,
}

/// An opcode the pipeline applies.
#[derive(Debug, Clone, PartialEq)]
pub enum StageOpcode {
    GainMap(GainMap),
    FixBadPixelsConstant(FixBadPixelsConstant),
}

/// What the pipeline does with a file's opcode lists: the opcodes it
/// applies, list by list, in file order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpcodePlan {
    pub list1: Vec<StageOpcode>,
    pub list2: Vec<StageOpcode>,
    pub list3: Vec<StageOpcode>,
    /// A lens opcode not marked optional was skipped (decided for M1: the
    /// status bar says lens corrections from the file are not applied).
    pub lens_corrections_skipped: bool,
}

/// Most gain map points accepted, over all planes. Real maps have a few
/// thousand.
const MAX_GAIN_POINTS: u64 = 1 << 20;

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn f64_at(b: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn area(b: &[u8]) -> Option<AreaSpec> {
    let a = AreaSpec {
        top: u32_at(b, 0)?,
        left: u32_at(b, 4)?,
        bottom: u32_at(b, 8)?,
        right: u32_at(b, 12)?,
        plane: u32_at(b, 16)?,
        planes: u32_at(b, 20)?,
        row_pitch: u32_at(b, 24)?,
        col_pitch: u32_at(b, 28)?,
    };
    let sane = a.top <= a.bottom
        && a.left <= a.right
        && a.planes >= 1
        && a.row_pitch >= 1
        && a.col_pitch >= 1;
    sane.then_some(a)
}

/// Parses GainMap parameters (big-endian): the area spec, then the map's
/// size, spacing, origin and planes, then its gains as f32.
pub fn parse_gain_map(b: &[u8]) -> Result<GainMap, String> {
    let bad = |what: &str| format!("GainMap: {what}");
    let area = area(b).ok_or_else(|| bad("invalid area"))?;
    let points_v = u32_at(b, 32).ok_or_else(|| bad("short"))?;
    let points_h = u32_at(b, 36).ok_or_else(|| bad("short"))?;
    let spacing_v = f64_at(b, 40).ok_or_else(|| bad("short"))?;
    let spacing_h = f64_at(b, 48).ok_or_else(|| bad("short"))?;
    let origin_v = f64_at(b, 56).ok_or_else(|| bad("short"))?;
    let origin_h = f64_at(b, 64).ok_or_else(|| bad("short"))?;
    let map_planes = u32_at(b, 72).ok_or_else(|| bad("short"))?;
    let count = u64::from(points_v) * u64::from(points_h) * u64::from(map_planes);
    if points_v == 0 || points_h == 0 || map_planes == 0 || count > MAX_GAIN_POINTS {
        return Err(bad(&format!("{points_v}x{points_h}x{map_planes} points")));
    }
    // Spacing divides positions; it only matters along an axis with more
    // than one point.
    let spacing_ok = |s: f64, n: u32| s.is_finite() && (s > 0.0 || n == 1);
    if !spacing_ok(spacing_v, points_v) || !spacing_ok(spacing_h, points_h) {
        return Err(bad("invalid spacing"));
    }
    if !origin_v.is_finite() || !origin_h.is_finite() {
        return Err(bad("invalid origin"));
    }
    let gains_bytes = b.get(76..).ok_or_else(|| bad("short"))?;
    if gains_bytes.len() as u64 != count * 4 {
        return Err(bad(&format!(
            "{} bytes of gains for {count} points",
            gains_bytes.len()
        )));
    }
    let gains: Vec<f32> = gains_bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&g| f32::from_be_bytes(g))
        .collect();
    if gains.iter().any(|g| !g.is_finite()) {
        return Err(bad("a gain that is not finite"));
    }
    Ok(GainMap {
        area,
        points_v,
        points_h,
        spacing_v,
        spacing_h,
        origin_v,
        origin_h,
        map_planes,
        gains,
    })
}

/// Parses FixBadPixelsConstant parameters: the constant and the Bayer
/// phase (0 to 3), big-endian.
pub fn parse_fix_bad_pixels_constant(b: &[u8]) -> Result<FixBadPixelsConstant, String> {
    let (Some(constant), Some(bayer_phase), 8) = (u32_at(b, 0), u32_at(b, 4), b.len()) else {
        return Err(format!("FixBadPixelsConstant: {} parameter bytes", b.len()));
    };
    if bayer_phase > 3 {
        return Err(format!("FixBadPixelsConstant: Bayer phase {bayer_phase}"));
    }
    Ok(FixBadPixelsConstant {
        constant,
        bayer_phase,
    })
}

fn is_lens(kind: OpcodeKind) -> bool {
    matches!(
        kind,
        OpcodeKind::WarpRectilinear | OpcodeKind::WarpRectilinear2 | OpcodeKind::FixVignetteRadial
    )
}

/// Decides what to do with each opcode (DEV-01, decided 2026-10-18):
/// FixBadPixelsConstant in OpcodeList1 and GainMap in OpcodeList2 are
/// applied, where the corpus has them and the pipeline implements them.
/// Elsewhere they count as not implemented: GainMap in OpcodeList3, for
/// one, would run on demosaiced values that stage 2 already white balanced,
/// so its clip at 1 would be in the wrong units. The lens opcodes are
/// skipped, and when one is not optional the plan says so; any other opcode
/// is skipped when optional and makes the file unsupported when not. An
/// opcode whose parameters do not parse is skipped when optional, and
/// otherwise makes the file unsupported.
pub fn plan(lists: &OpcodeLists) -> Result<OpcodePlan, String> {
    let mut plan = OpcodePlan::default();
    for (n, list, out) in [
        (1, &lists.list1, &mut plan.list1),
        (2, &lists.list2, &mut plan.list2),
        (3, &lists.list3, &mut plan.list3),
    ] {
        for op in list {
            let parsed = match (n, op.kind) {
                (2, OpcodeKind::GainMap) => {
                    Some(parse_gain_map(&op.params).map(StageOpcode::GainMap))
                }
                (1, OpcodeKind::FixBadPixelsConstant) => Some(
                    parse_fix_bad_pixels_constant(&op.params)
                        .map(StageOpcode::FixBadPixelsConstant),
                ),
                _ => None,
            };
            match parsed {
                Some(Ok(stage)) => out.push(stage),
                Some(Err(_)) | None if op.optional => {}
                Some(Err(e)) => return Err(format!("OpcodeList{n}: {e}")),
                None if is_lens(op.kind) => plan.lens_corrections_skipped = true,
                None => return Err(unsupported(n, op)),
            }
        }
    }
    Ok(plan)
}

fn unsupported(list: u32, op: &Opcode) -> String {
    format!(
        "the DNG opcode {:?} in OpcodeList{list}, which is not optional",
        op.kind
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_bytes(a: [u32; 8]) -> Vec<u8> {
        a.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    /// A 2 x 3 single-plane GainMap over the whole image.
    pub(crate) fn gain_map_bytes(gains: &[f32]) -> Vec<u8> {
        let mut b = area_bytes([0, 0, 0, 0, 0, 1, 1, 1]);
        b.extend(2u32.to_be_bytes());
        b.extend(3u32.to_be_bytes());
        for v in [1.0f64, 0.5, 0.0, 0.0] {
            b.extend(v.to_be_bytes());
        }
        b.extend(1u32.to_be_bytes());
        for g in gains {
            b.extend(g.to_be_bytes());
        }
        b
    }

    fn op(kind: OpcodeKind, optional: bool, params: Vec<u8>) -> Opcode {
        Opcode {
            kind,
            dng_version: [1, 3, 0, 0],
            optional,
            preview_skip: false,
            params,
        }
    }

    #[test]
    fn parses_a_gain_map() {
        let g = parse_gain_map(&gain_map_bytes(&[1.0, 1.5, 2.0, 1.1, 1.2, 1.3])).unwrap();
        assert_eq!((g.points_v, g.points_h, g.map_planes), (2, 3, 1));
        assert_eq!((g.spacing_v, g.spacing_h), (1.0, 0.5));
        assert_eq!(g.entry(1, 2, 0), 1.3);
        assert_eq!(g.entry(0, 1, 0), 1.5);
    }

    #[test]
    fn rejects_bad_gain_maps() {
        let good = gain_map_bytes(&[1.0; 6]);
        // Every truncation, and one gain too many.
        for len in 0..good.len() {
            assert!(parse_gain_map(&good[..len]).is_err(), "length {len}");
        }
        let mut long = good.clone();
        long.extend(1.0f32.to_be_bytes());
        assert!(parse_gain_map(&long).is_err());
        // Zero spacing with more than one point; a NaN gain; pitch 0.
        let mut zero = good.clone();
        zero[40..48].copy_from_slice(&0.0f64.to_be_bytes());
        assert!(parse_gain_map(&zero).is_err());
        let nan = gain_map_bytes(&[1.0, f32::NAN, 1.0, 1.0, 1.0, 1.0]);
        assert!(parse_gain_map(&nan).is_err());
        let mut pitch = good;
        pitch[24..28].copy_from_slice(&0u32.to_be_bytes());
        assert!(parse_gain_map(&pitch).is_err());
    }

    #[test]
    fn parses_fix_bad_pixels_constant() {
        let b: Vec<u8> = [0u32, 2].iter().flat_map(|v| v.to_be_bytes()).collect();
        assert_eq!(
            parse_fix_bad_pixels_constant(&b).unwrap(),
            FixBadPixelsConstant {
                constant: 0,
                bayer_phase: 2
            }
        );
        assert!(parse_fix_bad_pixels_constant(&b[..7]).is_err());
        let phase: Vec<u8> = [0u32, 4].iter().flat_map(|v| v.to_be_bytes()).collect();
        assert!(parse_fix_bad_pixels_constant(&phase).is_err());
    }

    #[test]
    fn plans_by_the_m1_rules() {
        let gm = gain_map_bytes(&[1.0; 6]);
        let fbp: Vec<u8> = [0u32, 1].iter().flat_map(|v| v.to_be_bytes()).collect();
        let lists = OpcodeLists {
            list1: vec![op(OpcodeKind::FixBadPixelsConstant, false, fbp)],
            list2: vec![
                op(OpcodeKind::GainMap, false, gm),
                op(OpcodeKind::MapPolynomial, true, vec![]),
            ],
            list3: vec![op(OpcodeKind::WarpRectilinear, true, vec![])],
        };
        let p = plan(&lists).unwrap();
        assert_eq!((p.list1.len(), p.list2.len(), p.list3.len()), (1, 1, 0));
        // An optional lens opcode skipped: nothing to tell.
        assert!(!p.lens_corrections_skipped);

        // A lens opcode not optional: skipped, and said so.
        let mut l = lists.clone();
        l.list3[0].optional = false;
        assert!(plan(&l).unwrap().lens_corrections_skipped);

        // Any other opcode not optional: unsupported.
        let mut l = lists.clone();
        l.list2[1].optional = false;
        assert!(plan(&l).unwrap_err().contains("MapPolynomial"));
        let mut l = lists.clone();
        l.list2.push(op(OpcodeKind::Unknown(99), false, vec![]));
        assert!(plan(&l).is_err());

        // Applied only in the list the pipeline runs them from.
        let mut l = lists.clone();
        l.list3
            .push(op(OpcodeKind::GainMap, false, gain_map_bytes(&[1.0; 6])));
        assert!(plan(&l).unwrap_err().contains("OpcodeList3"));
        l.list3[1].optional = true;
        assert_eq!(plan(&l).unwrap().list3.len(), 0);
        let mut l = lists.clone();
        l.list2.push(op(
            OpcodeKind::FixBadPixelsConstant,
            false,
            l.list1[0].params.clone(),
        ));
        assert!(plan(&l).unwrap_err().contains("OpcodeList2"));

        // A GainMap that does not parse: skipped when optional only.
        let mut l = lists.clone();
        l.list2[0].params.truncate(10);
        assert!(plan(&l).is_err());
        l.list2[0].optional = true;
        assert_eq!(plan(&l).unwrap().list2.len(), 0);
    }
}
