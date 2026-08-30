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

    char *envelope = NULL;
    char *snapshot = NULL;
    int recorded = syncer_rs_optimistic_record(
        "notes/42",
        "mutation-3",
        "desktop",
        "{\"phone\":2}",
        "{\"text\":\"draft\"}",
        &envelope,
        &snapshot
    );
    if (recorded != SYNCER_RS_OPT_OK || envelope == NULL || snapshot == NULL) {
        fputs("optimistic record failed\n", stderr);
        return EXIT_FAILURE;
    }

    char *next = NULL;
    int received = syncer_rs_optimistic_receive(envelope, "{\"phone\":2}", &next);
    if (received != SYNCER_RS_OPT_OK || next == NULL) {
        fputs("optimistic receive failed\n", stderr);
        syncer_rs_free(envelope);
        syncer_rs_free(snapshot);
        return EXIT_FAILURE;
    }

    syncer_rs_free(envelope);
    syncer_rs_free(snapshot);
    syncer_rs_free(next);
    return status;
}
