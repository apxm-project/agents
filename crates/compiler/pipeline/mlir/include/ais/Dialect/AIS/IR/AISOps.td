/**
 * @file AISOps.td
 * @brief Canonical APXM AIS semantic operations.
 */

#ifndef APXM_AIS_OPS
#define APXM_AIS_OPS

include "ais/Dialect/AIS/IR/AISDialect.td"
include "ais/Dialect/AIS/IR/AISTypes.td"
include "ais/Dialect/AIS/IR/AISAttributes.td"
include "mlir/IR/CommonTypeConstraints.td"
include "mlir/Interfaces/ControlFlowInterfaces.td"
include "mlir/Interfaces/SideEffectInterfaces.td"
include "ais/Dialect/AIS/IR/AISOps.semantic.generated.td"

#endif // APXM_AIS_OPS
