#ifndef FRAMELEAN_FLL_H
#define FRAMELEAN_FLL_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct FllOperation FllOperation;
/* ABI version 1. JSON strings are UTF-8; returned buffers belong to FLL. */
uint32_t fll_abi_version(void);
char *fll_versions(void);
FllOperation *fll_start(const char *request_json);
char *fll_poll(const FllOperation *handle);
void fll_cancel(const FllOperation *handle);
/* Release exactly once after a terminal poll, with no concurrent handle calls. */
void fll_release(FllOperation *handle);
void fll_free(char *buffer);
#ifdef __cplusplus
}
#endif
#endif
