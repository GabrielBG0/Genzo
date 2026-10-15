/*
 * A thin C layer over libheif, so the Rust side never mirrors libheif's
 * structs or enums. Everything that crosses into Rust is an int, a size, a
 * pointer to bytes or an opaque handle. Compiled against the real headers, so
 * a layout change in libheif breaks this build instead of corrupting memory.
 */
#include <libheif/heif.h>
#include <string.h>

/* Returns 0 on success. On failure, *message points to a string owned by
 * libheif that stays valid until the context is freed; copy it at once. */
static int check(struct heif_error err, const char** message) {
  if (err.code == heif_error_Ok) return 0;
  if (message) *message = err.message;
  return (int)err.code;
}

void gz_init(void) { heif_init(NULL); }

const char* gz_version(void) { return heif_get_version(); }

struct heif_context* gz_context_new(int max_threads) {
  struct heif_context* ctx = heif_context_alloc();
  if (ctx && max_threads > 0) heif_context_set_max_decoding_threads(ctx, max_threads);
  return ctx;
}

void gz_context_free(struct heif_context* ctx) { heif_context_free(ctx); }

/* The data must outlive the context: it is read without a copy. */
int gz_context_read(struct heif_context* ctx, const uint8_t* data, size_t len, const char** message) {
  return check(heif_context_read_from_memory_without_copy(ctx, data, len, NULL), message);
}

int gz_primary_handle(struct heif_context* ctx, struct heif_image_handle** out, const char** message) {
  return check(heif_context_get_primary_image_handle(ctx, out), message);
}

void gz_handle_free(struct heif_image_handle* h) { heif_image_handle_release(h); }

void gz_handle_info(const struct heif_image_handle* h, int* width, int* height, int* has_alpha,
                    int* luma_bits) {
  *width = heif_image_handle_get_width(h);
  *height = heif_image_handle_get_height(h);
  *has_alpha = heif_image_handle_has_alpha_channel(h);
  *luma_bits = heif_image_handle_get_luma_bits_per_pixel(h);
}

/* Color: 0 = none, 1 = nclx (fills the four codes), 2 = ICC (see gz_icc). */
int gz_color(const struct heif_image_handle* h, int* primaries, int* transfer, int* matrix,
             int* full_range) {
  enum heif_color_profile_type t = heif_image_handle_get_color_profile_type(h);
  if (t == heif_color_profile_type_rICC || t == heif_color_profile_type_prof) return 2;
  if (t != heif_color_profile_type_nclx) return 0;
  struct heif_color_profile_nclx* nclx = NULL;
  if (heif_image_handle_get_nclx_color_profile(h, &nclx).code != heif_error_Ok || !nclx) return 0;
  *primaries = (int)nclx->color_primaries;
  *transfer = (int)nclx->transfer_characteristics;
  *matrix = (int)nclx->matrix_coefficients;
  *full_range = nclx->full_range_flag;
  heif_nclx_color_profile_free(nclx);
  return 1;
}

size_t gz_icc_size(const struct heif_image_handle* h) {
  return heif_image_handle_get_raw_color_profile_size(h);
}

int gz_icc_copy(const struct heif_image_handle* h, uint8_t* out, const char** message) {
  return check(heif_image_handle_get_raw_color_profile(h, out), message);
}

/* Size of the first Exif block, or 0 when there is none. */
size_t gz_exif_size(const struct heif_image_handle* h, heif_item_id* id) {
  if (heif_image_handle_get_list_of_metadata_block_IDs(h, "Exif", id, 1) < 1) return 0;
  return heif_image_handle_get_metadata_size(h, *id);
}

int gz_exif_copy(const struct heif_image_handle* h, heif_item_id id, uint8_t* out,
                 const char** message) {
  return check(heif_image_handle_get_metadata(h, id, out), message);
}

/* The first thumbnail's handle, or NULL. */
struct heif_image_handle* gz_thumbnail(const struct heif_image_handle* h) {
  heif_item_id id;
  if (heif_image_handle_get_list_of_thumbnail_IDs(h, &id, 1) < 1) return NULL;
  struct heif_image_handle* thumb = NULL;
  if (heif_image_handle_get_thumbnail(h, id, &thumb).code != heif_error_Ok) return NULL;
  return thumb;
}

/* Decodes to interleaved RGBA: 8 bits per channel when wide == 0, else
 * 16-bit little-endian words holding values in the source bit depth.
 * The container's rotation, mirroring and crop are applied. */
int gz_decode(const struct heif_image_handle* h, int wide, struct heif_image** out,
              const char** message) {
  enum heif_chroma chroma = wide ? heif_chroma_interleaved_RRGGBBAA_LE : heif_chroma_interleaved_RGBA;
  return check(heif_decode_image(h, out, heif_colorspace_RGB, chroma, NULL), message);
}

void gz_image_free(struct heif_image* img) { heif_image_release(img); }

const uint8_t* gz_image_pixels(const struct heif_image* img, int* width, int* height,
                               size_t* stride, int* bits) {
  *width = heif_image_get_width(img, heif_channel_interleaved);
  *height = heif_image_get_height(img, heif_channel_interleaved);
  *bits = heif_image_get_bits_per_pixel_range(img, heif_channel_interleaved);
  return heif_image_get_plane_readonly2(img, heif_channel_interleaved, stride);
}
