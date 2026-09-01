#include "syncer_rs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
    const char *base = "{\"items\":[{\"id\":1,\"left\":true}]}";
    const char *incoming = "{\"items\":[{\"id\":\"1\",\"right\":true}]}";
    syncer_rs_options_t options = syncer_rs_default_options();
    if (options.detect_circular_refs) {
        fputs("detect_circular_refs defaulted to true\n", stderr);
        return EXIT_FAILURE;
    }
    options.array_strategy = SYNCER_RS_ARRAY_MERGE_BY_KEY;
    options.detect_circular_refs = true;

    char *merged = syncer_rs_merge_json_ex(base, incoming, &options);
    if (merged == NULL) {
        fputs("merge returned NULL\n", stderr);
        return EXIT_FAILURE;
    }

    const char *expected =
        "{\"items\":[{\"id\":\"1\",\"left\":true,\"right\":true}]}";
    int status = strcmp(merged, expected) == 0 ? EXIT_SUCCESS : EXIT_FAILURE;
    if (status != EXIT_SUCCESS) {
        fprintf(stderr, "unexpected merge: %s\n", merged);
    }

    syncer_rs_free(merged);
    if (status != EXIT_SUCCESS) {
        return status;
    }

    char *optimistic_envelope = (char *)1;
    char *snapshot = NULL;
    int recorded = syncer_rs_optimistic_record(
        "notes/42",
        "mutation-3",
        "desktop",
        "{\"phone\":2}",
        "{\"text\":\"draft\"}",
        &optimistic_envelope,
        &snapshot
    );
    if (recorded != SYNCER_RS_OPT_OK || optimistic_envelope == NULL ||
        snapshot == NULL) {
        fputs("optimistic record failed or left stale outputs\n", stderr);
        return EXIT_FAILURE;
    }

    char *next = (char *)1;
    int received = syncer_rs_optimistic_receive(
        optimistic_envelope, "{\"phone\":2}", &next);
    if (received != SYNCER_RS_OPT_OK || next == NULL) {
        fputs("optimistic receive failed\n", stderr);
        syncer_rs_free(optimistic_envelope);
        syncer_rs_free(snapshot);
        return EXIT_FAILURE;
    }

    syncer_rs_free(optimistic_envelope);
    syncer_rs_free(snapshot);
    syncer_rs_free(next);

    const char *envelope =
        "{\"schemaVersion\":\"opto-sync.causal.v1\","
        "\"documentId\":\"documents/example\","
        "\"mutationId\":\"mutation-0001\","
        "\"replicaId\":\"desktop\","
        "\"clock\":{\"phone\":2,\"desktop\":1},"
        "\"operation\":{\"kind\":\"upsert\","
        "\"value\":{\"title\":\"offline edit\"}}}";
    const char *checkpoint = "{\"phone\":2}";
    char *error = (char *)1;

    if (syncer_rs_causal_validate(envelope, &error) != SYNCER_RS_OK ||
        error != NULL) {
        fputs("causal validate failed or did not clear error_out\n", stderr);
        return EXIT_FAILURE;
    }

    int disposition = -1;
    if (syncer_rs_causal_disposition(
            envelope, checkpoint, &disposition, &error) != SYNCER_RS_OK ||
        disposition != SYNCER_RS_DISP_APPLY || error != NULL) {
        fputs("causal disposition failed\n", stderr);
        return EXIT_FAILURE;
    }

    char *joined = (char *)1;
    if (syncer_rs_causal_acknowledge(
            envelope, checkpoint, &joined, &error) != SYNCER_RS_OK ||
        joined == NULL || error != NULL) {
        fputs("causal acknowledge failed\n", stderr);
        return EXIT_FAILURE;
    }
    if (strcmp(joined, "{\"desktop\":1,\"phone\":2}") != 0) {
        fprintf(stderr, "unexpected checkpoint: %s\n", joined);
        syncer_rs_free(joined);
        return EXIT_FAILURE;
    }
    syncer_rs_free(joined);
    return status;
}
