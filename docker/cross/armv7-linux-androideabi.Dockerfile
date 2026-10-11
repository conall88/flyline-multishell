FROM ghcr.io/cross-rs/armv7-linux-androideabi:main

# Create libunwind.a symlinks where the NDK gcc linker expects them
RUN find /android-ndk -name "libgcc.a" -exec sh -c 'for f; do ln -sf "$f" "$(dirname "$f")/libunwind.a"; done' _ {} + || true
RUN find /android-ndk -name "libunwind.a" -exec sh -c 'for f; do cp -f "$f" /android-ndk/sysroot/usr/lib/arm-linux-androideabi/ 2>/dev/null || true; done' _ {} +
RUN sh -c 'cc=$(find /android-ndk -name "*arm*clang" | head -n1); "$cc" -shared -o /android-ndk/sysroot/usr/lib/arm-linux-androideabi/libreadline.so -x c /dev/null' || true
RUN find /android-ndk -name "libc.so" -o -name "libc.a" -exec sh -c 'for f; do cp -f /android-ndk/sysroot/usr/lib/arm-linux-androideabi/libreadline.so "$(dirname "$f")/" 2>/dev/null || true; done' _ {} +
