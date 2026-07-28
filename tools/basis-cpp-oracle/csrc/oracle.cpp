// The reference transcoder, reduced to one C function.
//
// Everything the Rust side needs is: hand it a KTX2 file, a level and a target,
// get back the bytes the reference produces. No allocation crosses the
// boundary — the caller sizes the buffer from `basis_oracle_size` first.

#include "basisu_transcoder.h"

#include <cstring>

extern "C" {

// Must be called once before anything else. Idempotent.
void basis_oracle_init() { basist::basisu_transcoder_init(); }

// How many bytes level `level` of `data` occupies in `target`, or 0 if the
// file cannot be read or the level does not exist.
uint32_t basis_oracle_size(const uint8_t *data, uint32_t length, uint32_t level,
                           int32_t target) {
  basist::ktx2_transcoder file;
  if (!file.init(data, length)) return 0;
  if (level >= file.get_levels()) return 0;

  basist::ktx2_image_level_info info;
  if (!file.get_image_level_info(info, level, 0, 0)) return 0;

  return basist::basis_get_bytes_per_block_or_pixel(
             static_cast<basist::transcoder_texture_format>(target)) *
         (basist::basis_transcoder_format_is_uncompressed(
              static_cast<basist::transcoder_texture_format>(target))
              ? info.m_orig_width * info.m_orig_height
              : info.m_total_blocks);
}

// Transcode into `out`, which must be `basis_oracle_size` bytes. Returns 1 on
// success. A target the file's codec cannot reach is a 0, not a crash.
int32_t basis_oracle_transcode(const uint8_t *data, uint32_t length,
                               uint32_t level, int32_t target, uint8_t *out,
                               uint32_t out_length) {
  basist::ktx2_transcoder file;
  if (!file.init(data, length)) return 0;
  if (!file.start_transcoding()) return 0;
  if (level >= file.get_levels()) return 0;

  basist::ktx2_image_level_info info;
  if (!file.get_image_level_info(info, level, 0, 0)) return 0;

  const basist::transcoder_texture_format format =
      static_cast<basist::transcoder_texture_format>(target);
  const bool uncompressed =
      basist::basis_transcoder_format_is_uncompressed(format);
  const uint32_t count =
      uncompressed ? info.m_orig_width * info.m_orig_height : info.m_total_blocks;
  const uint32_t needed =
      count * basist::basis_get_bytes_per_block_or_pixel(format);
  if (out_length < needed) return 0;

  return file.transcode_image_level(level, 0, 0, out, count, format) ? 1 : 0;
}

}  // extern "C"
