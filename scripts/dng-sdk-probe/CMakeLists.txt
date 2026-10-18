add_executable( color_probe color_probe.cpp )
target_include_directories( color_probe PRIVATE "../../lib/dng_sdk" "../../lib/common/public" "../../lib/xmp_core/public/include" )
target_link_libraries( color_probe dng_sdk xmp_core md5_lib expat_lib common )
