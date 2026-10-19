add_executable( color_probe color_probe.cpp )
target_include_directories( color_probe PRIVATE "../../lib/dng_sdk" "../../lib/common/public" "../../lib/xmp_core/public/include" )
target_link_libraries( color_probe dng_sdk xmp_core md5_lib expat_lib common )

add_executable( stage2_probe stage2_probe.cpp )
target_include_directories( stage2_probe PRIVATE "../../lib/dng_sdk" "../../lib/common/public" "../../lib/xmp_core/public/include" )
target_link_libraries( stage2_probe dng_sdk xmp_core md5_lib expat_lib common )

add_executable( tone_probe tone_probe.cpp )
target_include_directories( tone_probe PRIVATE "../../lib/dng_sdk" "../../lib/common/public" "../../lib/xmp_core/public/include" )
target_link_libraries( tone_probe dng_sdk xmp_core md5_lib expat_lib common )

add_executable( render_probe render_probe.cpp )
target_include_directories( render_probe PRIVATE "../../lib/dng_sdk" "../../lib/common/public" "../../lib/xmp_core/public/include" )
target_link_libraries( render_probe dng_sdk xmp_core md5_lib expat_lib common )
