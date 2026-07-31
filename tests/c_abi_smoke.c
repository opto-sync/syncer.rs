#include "syncer_rs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
    const char *base = "{\"items\":[{\"id\":1,\"left\":true}]}";
    const char *incoming = "{\"items\":[{\"id\":\"1\",\"right\":true}]}";
    syncer_rs_options_t options = syncer_rs_default_options();
    options.array_strategy = SYNCER_RS_ARRAY_MERGE_BY_KEY;

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
    return status;
}
