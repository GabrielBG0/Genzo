// Writes the Adobe DNG SDK 1.4's stage 2 image of a DNG: the raw image after
// OpcodeList1, linearization (black and white levels, LinearizationTable)
// and OpcodeList2, over the ActiveArea. Ground truth for Genzo's stage 1
// (DEV-01); a test tool only, never part of Genzo. Build notes: README.md.
//
//   stage2_probe IN.dng OUT.bin
//
// OUT.bin: "GZS2", then little-endian u32 width, height, planes and pixel
// type (3 = 16-bit, 11 = 32-bit float), then the samples row by row,
// planes interleaved. 16-bit samples are 0 to 65535 for 0 to 1.

#include <cstdio>
#include <cstdlib>
#include <cstring>

#include "dng_exceptions.h"
#include "dng_file_stream.h"
#include "dng_host.h"
#include "dng_image.h"
#include "dng_info.h"
#include "dng_negative.h"
#include "dng_pixel_buffer.h"
#include "dng_tag_types.h"

int main(int argc, char **argv)
{
    if (argc != 3) {
        fprintf(stderr, "usage: stage2_probe IN.dng OUT.bin\n");
        return 2;
    }
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
        const dng_image &image = *negative->Stage2Image();

        dng_rect bounds = image.Bounds();
        uint32 w = bounds.W(), h = bounds.H(), planes = image.Planes();
        uint32 type = image.PixelType();
        uint32 size = image.PixelSize();
        dng_pixel_buffer buffer;
        buffer.fArea = bounds;
        buffer.fPlane = 0;
        buffer.fPlanes = planes;
        buffer.fColStep = planes;
        buffer.fRowStep = (int32)(w * planes);
        buffer.fPlaneStep = 1;
        buffer.fPixelType = type;
        buffer.fPixelSize = size;
        size_t bytes = (size_t)w * h * planes * size;
        buffer.fData = malloc(bytes);
        image.Get(buffer);

        FILE *out = fopen(argv[2], "wb");
        if (!out) {
            fprintf(stderr, "cannot write %s\n", argv[2]);
            return 1;
        }
        uint32 header[4] = {w, h, planes, type};
        fwrite("GZS2", 1, 4, out);
        fwrite(header, 4, 4, out);
        fwrite(buffer.fData, 1, bytes, out);
        fclose(out);
        free(buffer.fData);
        printf("{\"width\": %u, \"height\": %u, \"planes\": %u, \"pixel_type\": %u}\n", w, h, planes, type);
    } catch (const dng_exception &e) {
        fprintf(stderr, "dng_exception %d\n", (int)e.ErrorCode());
        return 1;
    }
    return 0;
}
