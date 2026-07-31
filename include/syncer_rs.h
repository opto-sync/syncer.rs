#ifndef SYNCER_RS_H
#define SYNCER_RS_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SYNCER_RS_ABI_VERSION 1u

typedef enum syncer_rs_array_strategy {
    SYNCER_RS_ARRAY_REPLACE = 0,
    SYNCER_RS_ARRAY_APPEND = 1,
    SYNCER_RS_ARRAY_UNION = 2,
    SYNCER_RS_ARRAY_MERGE_BY_INDEX = 3,
    SYNCER_RS_ARRAY_MERGE_BY_KEY = 4
} syncer_rs_array_strategy_t;

typedef struct syncer_rs_options {
    uint32_t abi_version;
    int32_t array_strategy;
    uint32_t max_depth;
    bool resolve_by_timestamp;
    const char *lww_keys;
    const char *fww_keys;
    const char *array_match_keys;
} syncer_rs_options_t;

static inline syncer_rs_options_t syncer_rs_default_options(void) {
    syncer_rs_options_t options;
    options.abi_version = SYNCER_RS_ABI_VERSION;
    options.array_strategy = SYNCER_RS_ARRAY_REPLACE;
    options.max_depth = 0;
    options.resolve_by_timestamp = false;
    options.lww_keys = 0;
    options.fww_keys = 0;
    options.array_match_keys = 0;
    return options;
}

/*
 * Returned strings belong to syncer.rs and must be released with
 * syncer_rs_free. NULL indicates malformed JSON or invalid options.
 */
char *syncer_rs_merge_json(const char *base, const char *incoming);
char *syncer_rs_merge_json_ex(
    const char *base,
    const char *incoming,
    const syncer_rs_options_t *options
);
void syncer_rs_free(char *result);

/* Static "major.minor.patch" string. Do not free. */
const char *syncer_rs_version(void);

#ifdef __cplusplus
}
#endif

#endif
