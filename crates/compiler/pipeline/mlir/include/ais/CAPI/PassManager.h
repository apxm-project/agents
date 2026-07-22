#ifndef APXM_CAPI_PASS_MANAGER_H
#define APXM_CAPI_PASS_MANAGER_H

#include "ais/CAPI/Types.h"
#include "ais/CAPI/Module.h"
#include "ais/CAPI/PassRegistry.h"
#include "ais/CAPI/Error.h"
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// Pass manager lifecycle
ApxmPassManager *apxm_pass_manager_create(ApxmCompilerContext *ctx);
void apxm_pass_manager_destroy(ApxmPassManager *pm);
void apxm_pass_manager_clear(ApxmPassManager *pm);

// Pass execution and registration
bool apxm_pass_manager_run(ApxmPassManager *pm, ApxmModule *module);
bool apxm_pass_manager_add_pass_by_name(ApxmPassManager *pm, const char *pass_name);
bool apxm_pass_manager_has_pass(ApxmPassManager *pm, const char *pass_name);

// Read and erase a pass's per-run stat attributes from a module.
//
// Looks up `ais.<pass_name>_fired_count` and `ais.<pass_name>_ir_size_delta`
// as IntegerAttr on the module op (the `ais.` prefix is required because
// builtin.module rejects unprefixed attribute names). If either is present
// its value is written to the corresponding out-parameter and the attribute
// is stripped; otherwise the out-parameter is set to 0.
//
// Returns 0 on success, non-zero if any pointer argument is null.
int apxm_module_drain_pass_stats(ApxmModule *module,
                                 const char *pass_name,
                                 int64_t *fired_count_out,
                                 int64_t *ir_size_delta_out);

// Remove every `ais.<pass>_fired_count` and `ais.<pass>_ir_size_delta`
// attribute from the module op. The non-diagnostic compile path strips these
// so that pass stats do not leak into the serialized artifact (where they
// would break golden-roundtrip and idempotency checks).
//
// Returns 0 on success, non-zero if `module` is null.
int apxm_module_strip_all_pass_stats(ApxmModule *module);

// Walk every op in the module and sum the `ais.est_template_tokens`
// IntegerAttr value (treating absent attrs as 0). Returns the total in
// `total_out`. The pass runner computes `tokens_saved` as the pre/post delta
// around each pass.
//
// Returns 0 on success, non-zero if any pointer argument is null.
int apxm_module_total_template_tokens(ApxmModule *module, uint64_t *total_out);

#ifdef __cplusplus
}
#endif

#endif // APXM_CAPI_PASS_MANAGER_H
