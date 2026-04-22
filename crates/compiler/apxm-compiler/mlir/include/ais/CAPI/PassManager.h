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

// Specific pass additions (grouped by category)
// Analysis passes
void apxm_pass_manager_add_unconsumed_value_warning(ApxmPassManager *pm);

// Transform passes
void apxm_pass_manager_add_normalize(ApxmPassManager *pm);
void apxm_pass_manager_add_build_prompt(ApxmPassManager *pm);
void apxm_pass_manager_add_fuse_ask_ops(ApxmPassManager *pm);
void apxm_pass_manager_add_condense_ops(ApxmPassManager *pm);
void apxm_pass_manager_add_scheduling(ApxmPassManager *pm);

// Optimization passes
void apxm_pass_manager_add_canonicalizer(ApxmPassManager *pm);  // Includes DCE
void apxm_pass_manager_add_cse(ApxmPassManager *pm);
void apxm_pass_manager_add_symbol_dce(ApxmPassManager *pm);

// Read and erase a pass's per-run stat attributes from a module.
//
// Looks up `<pass_name>_fired_count` and `<pass_name>_ir_size_delta` as
// IntegerAttr on the module op. If either is present its value is written
// to the corresponding out-parameter and the attribute is removed; otherwise
// the out-parameter is set to 0.
//
// Returns 0 on success, non-zero if any pointer argument is null.
int apxm_module_drain_pass_stats(ApxmModule *module,
                                 const char *pass_name,
                                 int64_t *fired_count_out,
                                 int64_t *ir_size_delta_out);

#ifdef __cplusplus
}
#endif

#endif // APXM_CAPI_PASS_MANAGER_H
