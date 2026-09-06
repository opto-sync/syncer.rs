#ifndef SYNCER_RS_H
#define SYNCER_RS_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SYNCER_RS_ABI_VERSION 2u

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
    bool detect_circular_refs;
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
    options.detect_circular_refs = false;
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

/*
 * Optimistic writes for Flutter/Dart FFI and Rust desktop hosts.
 * envelope_out and snapshot_out MUST be persisted in one local transaction.
 * Each non-null char ** output is initialized to NULL before validation.
 * Release returned strings with syncer_rs_free. Diagnostics never include
 * document payloads or secrets.
 */
#define SYNCER_RS_OPT_OK 0
#define SYNCER_RS_OPT_ERR_CONFLICT 1
#define SYNCER_RS_OPT_ERR_MISSING_REPLICA 2
#define SYNCER_RS_OPT_ERR_STALE_VECTOR 3
#define SYNCER_RS_OPT_ERR_INVALID 4
#define SYNCER_RS_OPT_ERR_PANIC 5

int syncer_rs_optimistic_record(
    const char *document_id,
    const char *mutation_id,
    const char *replica_id,
    const char *clock_json,
    const char *payload_json,
    char **envelope_out,
    char **snapshot_out
);
int syncer_rs_optimistic_receive(
    const char *envelope_json,
    const char *checkpoint_json,
    char **checkpoint_out
);

#define SYNCER_RS_OK 0
#define SYNCER_RS_ERR_NULL 1
#define SYNCER_RS_ERR_UTF8 2
#define SYNCER_RS_ERR_JSON 3
#define SYNCER_RS_ERR_SCHEMA 4
#define SYNCER_RS_ERR_DOCUMENT 5
#define SYNCER_RS_ERR_MUTATION 6
#define SYNCER_RS_ERR_ACTOR 7
#define SYNCER_RS_ERR_VECTOR 8
#define SYNCER_RS_ERR_PANIC 9

#define SYNCER_RS_DISP_DUPLICATE 0
#define SYNCER_RS_DISP_STALE 1
#define SYNCER_RS_DISP_APPLY 2
#define SYNCER_RS_DISP_CONCURRENT 3

/*
 * Causal envelopes are merge+ordering only. Desktop lifecycle and SQLite
 * checkpoints live in opto-sync-clients/desktop-rust, not here.
 *
 * Each non-null char ** output is initialized to NULL before validation.
 * Successful checkpoint_out and populated error_out strings must be released
 * with syncer_rs_free.
 */
int syncer_rs_causal_validate(const char *envelope_json, char **error_out);
int syncer_rs_causal_disposition(
    const char *envelope_json,
    const char *checkpoint_json,
    int *disposition_out,
    char **error_out
);
int syncer_rs_causal_acknowledge(
    const char *envelope_json,
    const char *checkpoint_json,
    char **checkpoint_out,
    char **error_out
);

#ifdef __cplusplus
}
#endif

#endif
