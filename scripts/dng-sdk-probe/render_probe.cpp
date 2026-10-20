// Renders a DNG through the Adobe DNG SDK 1.4's dng_render with its default
// settings (Shadows 5, the profile's or the ACR3 tone curve) at a given
// exposure, into linear ProPhoto RGB as floats, and prints the mean of each
// BLOCK x BLOCK block of the result. Ground truth for DEV-02's criterion 5:
// Genzo's stage 6 with Genzo Standard, compared as block means, since the
// demosaics differ. A test tool only, never part of Genzo. Build notes:
// README.md next to this file.
//
//   render_probe IN.dng EXPOSURE BLOCK
//
// OpcodeList3 (lens corrections, which Genzo skips in M1) is cleared before
// stage 3, so the two renders cover the same geometry. The output is one
// JSON object: the stage 3 size, the default crop within it, what was
// cleared, and the block means row by row over the default crop.

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

#include "dng_1d_function.h"
#include "dng_color_space.h"
#include "dng_exceptions.h"
#include "dng_file_stream.h"
#include "dng_host.h"
#include "dng_image.h"
#include "dng_info.h"
#include "dng_negative.h"
#include "dng_opcode_list.h"
#include "dng_pixel_buffer.h"
#include "dng_render.h"

// ProPhoto RGB with no encoding gamma: the SDK's own primaries and scaling
// (dng_space_ProPhoto), linear.
class linear_prophoto : public dng_color_space
{
public:
    linear_prophoto()
    {
        SetMatrixToPCS(dng_matrix_3by3(0.7977, 0.1352, 0.0313,
                                       0.2880, 0.7119, 0.0001,
                                       0.0000, 0.0000, 0.8249));
    }

    const dng_1d_function &GammaFunction() const override { return dng_1d_identity::Get(); }
};

int main(int argc, char **argv)
{
    if (argc != 4) {
        fprintf(stderr, "usage: render_probe IN.dng EXPOSURE BLOCK\n");
        return 2;
    }
    const double exposure = atof(argv[2]);
    const uint32 block = (uint32)strtoul(argv[3], nullptr, 10);
    try {
        dng_file_stream stream(argv[1]);
        dng_host host;
        dng_info info;
        info.Parse(host, stream);
        info.PostParse(host);
        if (!info.IsValidDNG()) {
            fprintf(stderr, "not a valid DNG\n");
            return 1;
        }
        AutoPtr<dng_negative> negative(host.Make_dng_negative());
        negative->Parse(host, stream, info);
        negative->PostParse(host, stream, info);
        negative->ReadStage1Image(host, stream, info);
        negative->BuildStage2Image(host);
        const uint32 cleared = negative->OpcodeList3().Count();
        negative->OpcodeList3().Clear();
        negative->BuildStage3Image(host);

        linear_prophoto space;
        dng_render render(host, *negative);
        render.SetFinalSpace(space);
        render.SetFinalPixelType(ttFloat);
        render.SetExposure(exposure);
        AutoPtr<dng_image> image(render.Render());

        const dng_rect bounds = image->Bounds();
        const uint32 w = bounds.W(), h = bounds.H();
        dng_pixel_buffer buffer;
        buffer.fArea = bounds;
        buffer.fPlane = 0;
        buffer.fPlanes = 3;
        buffer.fColStep = 3;
        buffer.fRowStep = (int32)(w * 3);
        buffer.fPlaneStep = 1;
        buffer.fPixelType = ttFloat;
        buffer.fPixelSize = 4;
        std::vector<real32> pixels((size_t)w * h * 3);
        buffer.fData = pixels.data();
        image->Get(buffer);

        const dng_image &stage3 = *negative->Stage3Image();
        const dng_rect crop = negative->DefaultCropArea();
        printf("{\"stage3\": [%u, %u], \"crop\": [%d, %d, %d, %d], \"rendered\": [%u, %u], "
               "\"exposure\": %.17g, \"block\": %u, \"opcode_list_3_cleared\": %u, \"means\": [",
               stage3.Bounds().W(), stage3.Bounds().H(), crop.l, crop.t, crop.r, crop.b, w, h,
               exposure, block, cleared);
        const uint32 bw = w / block, bh = h / block;
        for (uint32 by = 0; by < bh; by++) {
            printf("%s[", by ? ", " : "");
            for (uint32 bx = 0; bx < bw; bx++) {
                double sum[3] = {0, 0, 0};
                for (uint32 y = by * block; y < (by + 1) * block; y++)
                    for (uint32 x = bx * block; x < (bx + 1) * block; x++)
                        for (int c = 0; c < 3; c++)
                            sum[c] += pixels[((size_t)y * w + x) * 3 + c];
                const double n = (double)block * block;
                printf("%s[%.9g, %.9g, %.9g]", bx ? ", " : "", sum[0] / n, sum[1] / n, sum[2] / n);
            }
            printf("]");
        }
        printf("]}\n");
    } catch (const dng_exception &e) {
        fprintf(stderr, "dng_exception %d\n", (int)e.ErrorCode());
        return 1;
    }
    return 0;
}
