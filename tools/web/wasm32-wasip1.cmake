# CMake's toolchain for the browser build's C (libwinevbs): clang's WebAssembly target, against the
# wasi-libc that tools/web/wasi-libc.sh builds (WASI_SYSROOT). Static libraries only: the player's
# own link (rust-lld) makes the program.
set(CMAKE_SYSTEM_NAME WASI)
set(CMAKE_SYSTEM_VERSION 1)
set(CMAKE_SYSTEM_PROCESSOR wasm32)
set(CMAKE_C_COMPILER clang)
set(CMAKE_C_COMPILER_TARGET wasm32-wasip1)
set(CMAKE_AR llvm-ar)
set(CMAKE_RANLIB llvm-ranlib)
set(CMAKE_SYSROOT $ENV{WASI_SYSROOT})
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
# the WebAssembly features Rust's wasm32-wasip1 uses, so the objects link with its own
set(CMAKE_C_FLAGS_INIT "-mbulk-memory -mnontrapping-fptoint -msign-ext -mmutable-globals -mmultivalue -mreference-types")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
