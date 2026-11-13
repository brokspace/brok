/*
 * Lets the prebuilt ONNX Runtime objects link on an older glibc.
 *
 * They are built against glibc 2.38 and reference a few symbols that older
 * systems lack. The release workflow links this file into the Linux x86_64
 * binary, built on Ubuntu 20.04, so that the binary runs on glibc 2.31 and
 * newer instead of only on the newest distributions.
 *
 * - __libc_single_threaded (glibc 2.32): 0 means "may have several threads",
 *   which is always safe; libstdc++ then keeps its atomic reference counts.
 * - __isoc23_strto* (glibc 2.38): the C23 variants differ from strto* only in
 *   accepting "0b" binary prefixes, which ONNX Runtime does not rely on.
 */
#include <stdlib.h>

__attribute__((weak)) char __libc_single_threaded = 0;

__attribute__((weak)) long __isoc23_strtol(const char *s, char **end, int base)
{
    return strtol(s, end, base);
}

__attribute__((weak)) long long __isoc23_strtoll(const char *s, char **end, int base)
{
    return strtoll(s, end, base);
}

__attribute__((weak)) unsigned long long __isoc23_strtoull(const char *s, char **end, int base)
{
    return strtoull(s, end, base);
}
