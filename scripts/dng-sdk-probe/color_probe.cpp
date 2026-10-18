// Prints what the Adobe DNG SDK 1.4 computes for a DNG's camera color, one
// JSON object per line: the as-shot white (xy, temperature, tint), and for
// that white and for fixed ones (D50, D55, D65, Standard A) the camera white
// and CameraToPCS (camera RGB to XYZ D50). With --temperature, it prints the
// SDK's temperature and tint to xy conversion for a fixed set of pairs
// instead. Ground truth for Genzo's color crate; a test tool only, never part
// of Genzo. Build notes: README.md next to this file.

#include <cstdio>
#include <cstring>

#include "dng_camera_profile.h"
#include "dng_color_spec.h"
#include "dng_exceptions.h"
#include "dng_file_stream.h"
#include "dng_host.h"
#include "dng_info.h"
#include "dng_matrix.h"
#include "dng_negative.h"
#include "dng_temperature.h"
#include "dng_xy_coord.h"

static void print_spec(const char *name, dng_color_spec &spec, const dng_xy_coord &white)
{
    spec.SetWhiteXY(white);
    dng_temperature t(white);
    const dng_vector &cw = spec.CameraWhite();
    const dng_matrix &m = spec.CameraToPCS();
    printf("{\"white\": \"%s\", \"xy\": [%.12f, %.12f], \"temperature\": %.9f, \"tint\": %.9f, "
           "\"camera_white\": [%.12f, %.12f, %.12f], \"camera_to_pcs\": [",
           name, white.x, white.y, t.Temperature(), t.Tint(), cw[0], cw[1], cw[2]);
    for (uint32 r = 0; r < 3; r++)
        for (uint32 c = 0; c < 3; c++)
            printf("%s%.12f", (r || c) ? ", " : "", m[r][c]);
    printf("]}");
}

static void print_temperatures()
{
    static const double pairs[][2] = {
        {2000, 0}, {2850, -20}, {3200, 5}, {4000, 0}, {5000, 10}, {5500, -10},
        {6500, 0}, {7500, 30}, {10000, 50}, {20000, -80}, {50000, -150}, {1700, 150},
    };
    for (const auto &p : pairs) {
        dng_temperature t(p[0], p[1]);
        dng_xy_coord xy = t.Get_xy_coord();
        dng_temperature back(xy);
        printf("{\"temperature\": %.3f, \"tint\": %.3f, \"xy\": [%.12f, %.12f], "
               "\"back\": [%.9f, %.9f]}\n",
               p[0], p[1], xy.x, xy.y, back.Temperature(), back.Tint());
    }
}

int main(int argc, char **argv)
{
    if (argc == 2 && strcmp(argv[1], "--temperature") == 0) {
        print_temperatures();
        return 0;
    }
    for (int i = 1; i < argc; i++) {
        printf("{\"file\": \"%s\", ", argv[i]);
        try {
            dng_file_stream stream(argv[i]);
            dng_host host;
            dng_info info;
            info.Parse(host, stream);
            info.PostParse(host);
            if (!info.IsValidDNG()) {
                printf("\"error\": \"not a valid DNG\"}\n");
                continue;
            }
            AutoPtr<dng_negative> negative(host.Make_dng_negative());
            negative->Parse(host, stream, info);
            negative->PostParse(host, stream, info);
            AutoPtr<dng_color_spec> spec(negative->MakeColorSpec(dng_camera_profile_id()));

            // As dng_render does: the neutral, else the white xy, else D55.
            dng_xy_coord asShot;
            const char *source;
            if (negative->HasCameraNeutral()) {
                asShot = spec->NeutralToXY(negative->CameraNeutral());
                source = "neutral";
            } else if (negative->HasCameraWhiteXY()) {
                asShot = negative->CameraWhiteXY();
                source = "white_xy";
            } else {
                asShot = D55_xy_coord();
                source = "default_d55";
            }
            printf("\"as_shot_source\": \"%s\", \"specs\": [", source);
            print_spec("as_shot", *spec, asShot);
            printf(", ");
            print_spec("D50", *spec, D50_xy_coord());
            printf(", ");
            print_spec("D55", *spec, D55_xy_coord());
            printf(", ");
            print_spec("D65", *spec, D65_xy_coord());
            printf(", ");
            print_spec("A", *spec, StdA_xy_coord());
            printf("]}\n");
        } catch (const dng_exception &e) {
            printf("\"error\": \"dng_exception %d\"}\n", (int)e.ErrorCode());
        } catch (...) {
            printf("\"error\": \"exception\"}\n");
        }
    }
    return 0;
}
