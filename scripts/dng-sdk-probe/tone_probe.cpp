// Prints what the Adobe DNG SDK 1.4 computes for the Genzo Standard base
// curve (DEV-02): the ACR3 default tone curve, the exposure ramp, the
// ProfileToneCurve spline, the 4097-entry table dng_render builds from a
// curve, RefBaselineRGBTone, and each DNG's render parameters. Ground truth
// for Genzo's color::tone; a test tool only, never part of Genzo. Build
// notes: README.md next to this file.
//
//   tone_probe --curves FILE...   JSON: ACR3, ramps, splines (synthetic sets
//                                 and each FILE's ProfileToneCurve), tables
//   tone_probe --rgbtone N OUT    N triples from the shared generator, through
//                                 RefBaselineRGBTone with the ACR3 table, as
//                                 raw little-endian f32 (3 per triple)
//   tone_probe --params FILE...   one JSON object per file: what dng_render
//                                 derives for its ramp and curve

#include <cmath>
#include <cstdio>
#include <cstring>
#include <cstdlib>
#include <vector>

#include "dng_1d_table.h"
#include "dng_camera_profile.h"
#include "dng_exceptions.h"
#include "dng_file_stream.h"
#include "dng_host.h"
#include "dng_info.h"
#include "dng_memory.h"
#include "dng_negative.h"
#include "dng_reference.h"
#include "dng_render.h"
#include "dng_spline.h"
#include "dng_tone_curve.h"

// Grid sizes: criterion 1 asks 10,000 points of the ACR3 curve; the ramps
// and splines use 1001, plus 101 across each ramp's toe.
static const int kAcr3Grid = 10001;
static const int kGrid = 1001;

static void print_function(const dng_1d_function &f, int grid, const char *format)
{
    printf("[");
    for (int i = 0; i < grid; i++) {
        printf(i ? ", " : "");
        printf(format, f.Evaluate(i / (double)(grid - 1)));
    }
    printf("]");
}

static void print_toe(const dng_function_exposure_ramp &ramp)
{
    printf("[");
    double lo = ramp.fBlack - 2.0 * ramp.fRadius, hi = ramp.fBlack + 2.0 * ramp.fRadius;
    for (int i = 0; i <= 100; i++) {
        double x = lo + (hi - lo) * i / 100.0;
        printf("%s[%.17g, %.17g]", i ? ", " : "", x, ramp.Evaluate(x));
    }
    printf("]");
}

static void print_table(const dng_1d_function &f)
{
    dng_1d_table table;
    table.Initialize(gDefaultDNGMemoryAllocator, f);
    printf("[");
    for (uint32 i = 0; i <= dng_1d_table::kTableSize; i++)
        printf("%s%.9g", i ? ", " : "", table.Table()[i]);
    printf("]");
}

static void print_spline(const char *name, const dng_tone_curve &curve)
{
    printf("{\"name\": \"%s\", \"valid\": %s, \"points\": [", name,
           curve.IsValid() ? "true" : "false");
    for (size_t i = 0; i < curve.fCoord.size(); i++)
        printf("%s%.9g, %.9g", i ? ", " : "", curve.fCoord[i].h, curve.fCoord[i].v);
    printf("]");
    if (curve.IsValid()) {
        dng_spline_solver solver;
        curve.Solve(solver);
        printf(", \"evaluate\": ");
        print_function(solver, kGrid, "%.17g");
        printf(", \"table\": ");
        print_table(solver);
    }
    printf("}");
}

static dng_tone_curve make_curve(const double *xy, int points)
{
    dng_tone_curve curve;
    curve.fCoord.clear();
    for (int i = 0; i < points; i++)
        // As stored in a file: FLOAT values (dng_camera_profile reads real32).
        curve.fCoord.push_back(dng_point_real64((real32)xy[2 * i + 1], (real32)xy[2 * i]));
    return curve;
}

static bool open_negative(const char *path, dng_host &host, AutoPtr<dng_negative> &negative)
{
    dng_file_stream stream(path);
    dng_info info;
    info.Parse(host, stream);
    info.PostParse(host);
    if (!info.IsValidDNG())
        return false;
    negative.Reset(host.Make_dng_negative());
    negative->Parse(host, stream, info);
    negative->PostParse(host, stream, info);
    return true;
}

static void curves(int argc, char **argv)
{
    printf("{\"acr3\": {\"evaluate\": ");
    print_function(dng_tone_curve_acr3_default::Get(), kAcr3Grid, "%.9g");
    printf(", \"table\": ");
    print_table(dng_tone_curve_acr3_default::Get());
    printf("}, \"ramps\": [");
    static const double blacks[] = {0.0, 1e-4, 0.005, 0.05, 0.5};
    for (int b = 0; b < 5; b++) {
        dng_function_exposure_ramp ramp(1.0, blacks[b], blacks[b]);
        printf("%s{\"white\": 1, \"black\": %.17g, \"evaluate\": ", b ? ", " : "", blacks[b]);
        print_function(ramp, kGrid, "%.17g");
        printf(", \"toe\": ");
        print_toe(ramp);
        printf("}");
    }
    printf("], \"splines\": [");

    // Synthetic point sets: (x, y) pairs. The last two are invalid.
    static const double two[] = {0, 0, 1, 1};
    static const double three[] = {0, 0, 0.3, 0.5, 1, 1};
    static const double uneven[] = {0, 0, 0.01, 0.002, 0.05, 0.03, 0.2, 0.25, 0.7, 0.85, 1, 1};
    static const double wavy[] = {0, 0, 0.2, 0.6, 0.4, 0.3, 0.6, 0.8, 1, 0.9};
    static const double decreasing_x[] = {0, 0, 0.5, 0.5, 0.4, 0.6, 1, 1};
    static const double out_of_range[] = {0, 0, 0.5, 1.2, 1, 1};
    print_spline("two", make_curve(two, 2));
    printf(", ");
    print_spline("three", make_curve(three, 3));
    printf(", ");
    print_spline("uneven", make_curve(uneven, 6));
    printf(", ");
    print_spline("non_monotonic", make_curve(wavy, 5));
    printf(", ");
    print_spline("x_not_increasing", make_curve(decreasing_x, 4));
    printf(", ");
    print_spline("out_of_range", make_curve(out_of_range, 3));
    for (int i = 0; i < argc; i++) {
        dng_host host;
        AutoPtr<dng_negative> negative;
        try {
            if (!open_negative(argv[i], host, negative))
                continue;
        } catch (const dng_exception &e) {
            fprintf(stderr, "%s: dng_exception %d\n", argv[i], (int)e.ErrorCode());
            return;
        }
        const dng_camera_profile *profile = negative->ProfileByID(dng_camera_profile_id());
        if (!profile || !profile->ToneCurve().IsValid())
            continue;
        printf(", ");
        print_spline(argv[i], profile->ToneCurve());
    }
    printf("]}\n");
}

// The shared generator (color/tests/tone_sdk.rs has the same one): xorshift32,
// 24-bit floats in [0, 1), and every eighth triple is an edge case.
static uint32 state = 0x9E3779B9u;

static uint32 next_u32()
{
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    return state;
}

static real32 next_unit() { return (real32)(next_u32() >> 8) * (1.0f / 16777216.0f); }

static void triple(uint32 i, real32 v[3])
{
    v[0] = next_unit();
    v[1] = next_unit();
    v[2] = next_unit();
    switch (i % 16) {
    case 0: v[1] = v[0]; break;               // two equal, the larger
    case 2: v[2] = v[1]; break;               // two equal
    case 4: v[1] = v[0]; v[2] = v[0]; break;  // neutral
    case 6: v[i % 3] = 0.0f; break;           // a zero channel
    case 8: v[(i / 16) % 3] = 1.0f; break;    // a channel at 1
    case 10: v[0] = 0.0f; v[1] = 0.0f; v[2] = 0.0f; break;
    case 12: v[0] = 1.0f; v[1] = 1.0f; v[2] = 1.0f; break;
    case 14: v[2] = v[0]; break;              // two equal, others
    default: break;
    }
}

static int rgbtone(uint32 count, const char *out)
{
    dng_1d_table table;
    table.Initialize(gDefaultDNGMemoryAllocator, dng_tone_curve_acr3_default::Get());
    std::vector<real32> r(count), g(count), b(count), rr(count), gg(count), bb(count);
    for (uint32 i = 0; i < count; i++) {
        real32 v[3];
        triple(i, v);
        r[i] = v[0];
        g[i] = v[1];
        b[i] = v[2];
    }
    RefBaselineRGBTone(r.data(), g.data(), b.data(), rr.data(), gg.data(), bb.data(), count, table);
    FILE *f = fopen(out, "wb");
    if (!f)
        return 1;
    for (uint32 i = 0; i < count; i++) {
        real32 o[3] = {rr[i], gg[i], bb[i]};
        fwrite(o, sizeof(real32), 3, f);
    }
    fclose(f);
    return 0;
}

static void params(int argc, char **argv)
{
    for (int i = 0; i < argc; i++) {
        printf("{\"file\": \"%s\", ", argv[i]);
        try {
            dng_host host;
            AutoPtr<dng_negative> negative;
            if (!open_negative(argv[i], host, negative)) {
                printf("\"error\": \"not a valid DNG\"}\n");
                continue;
            }
            const dng_camera_profile_id id;
            const dng_camera_profile *profile = negative->ProfileByID(id);
            // As dng_render's constructor and dng_render_task::Start do, at
            // exposure 0 and Shadows 5.
            double shadows = 5.0;
            bool sceneReferred = negative->ColorimetricReference() == crSceneReferred;
            if (!sceneReferred)
                shadows = 0.0;
            bool profileCurve = profile && profile->ToneCurve().IsValid();
            if (profile && profile->DefaultBlackRender() == defaultBlackRender_None)
                shadows = 0.0;
            double exposure = negative->TotalBaselineExposure(id) - log(negative->Stage3Gain()) / log(2.0);
            double white = 1.0 / pow(2.0, exposure > 0.0 ? exposure : 0.0);
            double black = shadows * negative->ShadowScale() * negative->Stage3Gain() * 0.001;
            if (black > 0.99 * white)
                black = 0.99 * white;
            printf("\"baseline_exposure\": %.17g, \"total_baseline_exposure\": %.17g, "
                   "\"stage3_gain\": %.17g, \"shadow_scale\": %.17g, \"colorimetric_reference\": %u, "
                   "\"default_black_render\": %u, \"profile_curve_points\": %u, "
                   "\"curve\": \"%s\", \"white\": %.17g, \"black\": %.17g}\n",
                   negative->BaselineExposure(), negative->TotalBaselineExposure(id),
                   negative->Stage3Gain(), negative->ShadowScale(),
                   (unsigned)negative->ColorimetricReference(),
                   profile ? (unsigned)profile->DefaultBlackRender() : 0u,
                   profileCurve ? (unsigned)profile->ToneCurve().fCoord.size() : 0u,
                   profileCurve ? "profile" : (sceneReferred ? "acr3" : "identity"), white, black);
        } catch (const dng_exception &e) {
            printf("\"error\": \"dng_exception %d\"}\n", (int)e.ErrorCode());
        }
    }
}

int main(int argc, char **argv)
{
    if (argc >= 2 && strcmp(argv[1], "--curves") == 0) {
        curves(argc - 2, argv + 2);
        return 0;
    }
    if (argc == 4 && strcmp(argv[1], "--rgbtone") == 0)
        return rgbtone((uint32)strtoul(argv[2], nullptr, 10), argv[3]);
    if (argc >= 2 && strcmp(argv[1], "--params") == 0) {
        params(argc - 2, argv + 2);
        return 0;
    }
    fprintf(stderr, "usage: tone_probe --curves FILE... | --rgbtone N OUT | --params FILE...\n");
    return 2;
}
