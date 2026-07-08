#include <stddef.h>
extern void Reset_Handler(void);
/* Cortex-M vector table: [0]=initial SP, [1]=reset, rest unused. */
__attribute__((section(".vectors"), used))
void *const vectors[16] = {
    (void *)0x20020000,     /* initial SP: top of the 128 KiB DTCM */
    (void *)Reset_Handler,  /* reset: the embedded-Swift entry (@_cdecl) */
};
/* Bare-metal runtime stubs the embedded-Swift runtime references but the
 * leaf blink never actually calls. */
unsigned long __stack_chk_guard = 0xDEADBEEF;
void __stack_chk_fail(void) { for (;;) {} }
int posix_memalign(void **p, size_t a, size_t s) { (void)p;(void)a;(void)s; return 1; }
void free(void *p) { (void)p; }
int putchar(int c) { return c; }
void *__aeabi_memmove(void *d, const void *s, size_t n) {
    char *dd = d; const char *ss = s;
    while (n--) *dd++ = *ss++;
    return d;
}
